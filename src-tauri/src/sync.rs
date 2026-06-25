//! LAN sync: discover other DevClip instances on the local network and
//! exchange snippets + clipboard history directly, machine-to-machine.
//!
//! No cloud, no account, no extra apps — like LocalSend. Two transports:
//!
//! * **UDP broadcast** (port [`DISCOVERY_PORT`]) for discovery: a scanning
//!   instance broadcasts a "who's there?" datagram; every other instance
//!   replies (unicast) with its identity and TCP sync port.
//! * **TCP** (port [`SYNC_PORT`]) for the exchange: the client sends its full
//!   export, the server merges it and sends its own export back, the client
//!   merges that. One round trip and both sides hold the union, newest wins.
//!
//! The merge itself (last-write-wins, monotonic stats) lives in `devclip-core`
//! and is unit-tested there; this module is just framing and sockets.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use devclip_core::{ClipboardEntry, MergeStats, Snippet};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::commands::AppState;

/// UDP port used for discovery broadcasts.
pub const DISCOVERY_PORT: u16 = 19484;
/// TCP port used for the snippet/clipboard exchange.
pub const SYNC_PORT: u16 = 19485;
/// Wire protocol version; bumped if the payload shape changes incompatibly.
const PROTOCOL: u32 = 1;
/// Refuse absurd frames so a bad/hostile peer can't exhaust memory.
const MAX_FRAME: u32 = 64 * 1024 * 1024;

// ----- Wire types -------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct WireSnippet {
    sync_id: String,
    name: String,
    content: String,
    created_at: i64,
    updated_at: i64,
    last_used_at: Option<i64>,
    use_count: i64,
}

#[derive(Serialize, Deserialize)]
struct WireClip {
    content: String,
    created_at: i64,
}

/// The full payload one machine sends the other.
#[derive(Serialize, Deserialize)]
struct SyncPayload {
    protocol: u32,
    device_id: String,
    device_name: String,
    /// Port this sender's own TCP sync server listens on, so the receiver can
    /// remember a reachable address for one-tap re-sync later.
    sync_port: u16,
    snippets: Vec<WireSnippet>,
    clips: Vec<WireClip>,
}

/// A discovery datagram (request or reply), sent over UDP.
#[derive(Serialize, Deserialize)]
struct Beacon {
    /// "discover" (a scan request) or "announce" (a reply).
    kind: String,
    device_id: String,
    device_name: String,
    sync_port: u16,
}

/// A peer found during a scan, surfaced to the UI.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub device_id: String,
    pub device_name: String,
    /// "ip:port" of the peer's TCP sync server.
    pub addr: String,
}

// ----- Conversions ------------------------------------------------------------

fn snippet_to_wire(s: Snippet) -> WireSnippet {
    WireSnippet {
        sync_id: s.sync_id,
        name: s.name,
        content: s.content,
        created_at: s.created_at,
        updated_at: s.updated_at,
        last_used_at: s.last_used_at,
        use_count: s.use_count,
    }
}

fn wire_to_snippet(w: WireSnippet) -> Snippet {
    Snippet {
        id: 0, // local id is assigned by the store on insert
        sync_id: w.sync_id,
        name: w.name,
        content: w.content,
        created_at: w.created_at,
        updated_at: w.updated_at,
        last_used_at: w.last_used_at,
        use_count: w.use_count,
    }
}

fn wire_to_clip(w: WireClip) -> ClipboardEntry {
    ClipboardEntry {
        id: 0,
        content: w.content,
        created_at: w.created_at,
    }
}

// ----- Building / applying payloads ------------------------------------------

/// Snapshot this machine's snippets + clipboard into a payload to send.
fn build_payload(state: &AppState) -> Result<SyncPayload, String> {
    let (device_id, device_name, cap) = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        (s.device_id.clone(), s.device_name.clone(), s.clipboard_cap)
    };
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let snippets = store.all_snippets().map_err(|e| e.to_string())?;
    let clips = store.clipboard_history(cap).map_err(|e| e.to_string())?;
    Ok(SyncPayload {
        protocol: PROTOCOL,
        device_id,
        device_name,
        sync_port: SYNC_PORT,
        snippets: snippets.into_iter().map(snippet_to_wire).collect(),
        clips: clips
            .into_iter()
            .map(|c| WireClip {
                content: c.content,
                created_at: c.created_at,
            })
            .collect(),
    })
}

/// Merge a received payload into the local store, returning what changed.
fn apply_payload(state: &AppState, payload: SyncPayload) -> Result<MergeStats, String> {
    let cap = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        s.clipboard_cap
    };
    let snippets: Vec<Snippet> = payload.snippets.into_iter().map(wire_to_snippet).collect();
    let clips: Vec<ClipboardEntry> = payload.clips.into_iter().map(wire_to_clip).collect();

    let store = state.store.lock().map_err(|e| e.to_string())?;
    let (added, updated) = store.merge_snippets(&snippets).map_err(|e| e.to_string())?;
    let clips_added = store.merge_clipboard(&clips, cap).map_err(|e| e.to_string())?;
    Ok(MergeStats {
        snippets_added: added,
        snippets_updated: updated,
        clips_added,
    })
}

// ----- Length-prefixed framing ------------------------------------------------

fn write_frame(stream: &mut TcpStream, bytes: &[u8]) -> std::io::Result<()> {
    let len = bytes.len() as u32;
    stream.write_all(&len.to_be_bytes())?;
    stream.write_all(bytes)?;
    stream.flush()
}

fn read_frame(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_be_bytes(len_buf);
    if len > MAX_FRAME {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "sync frame too large",
        ));
    }
    let mut buf = vec![0u8; len as usize];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

// ----- Listeners (started once at app launch) --------------------------------

/// Start the UDP discovery responder and the TCP sync server. Failures to bind
/// (e.g. another instance already running on this machine) are logged and
/// sync is simply unavailable — they never crash the app.
pub fn start(app: AppHandle) {
    start_discovery_responder(app.clone());
    start_sync_server(app);
}

/// Listen for discovery broadcasts and reply with our identity.
fn start_discovery_responder(app: AppHandle) {
    std::thread::spawn(move || {
        let socket = match UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("devclip sync: discovery responder unavailable: {e}");
                return;
            }
        };
        let _ = socket.set_broadcast(true);
        let mut buf = [0u8; 2048];
        loop {
            let (n, from) = match socket.recv_from(&mut buf) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let Ok(beacon) = serde_json::from_slice::<Beacon>(&buf[..n]) else {
                continue;
            };
            if beacon.kind != "discover" {
                continue;
            }
            let Some(state) = app.try_state::<AppState>() else {
                continue;
            };
            // Ignore our own broadcast.
            let (my_id, my_name) = {
                let Ok(s) = state.settings.lock() else { continue };
                (s.device_id.clone(), s.device_name.clone())
            };
            if beacon.device_id == my_id {
                continue;
            }
            let reply = Beacon {
                kind: "announce".into(),
                device_id: my_id,
                device_name: my_name,
                sync_port: SYNC_PORT,
            };
            if let Ok(bytes) = serde_json::to_vec(&reply) {
                let _ = socket.send_to(&bytes, from);
            }
        }
    });
}

/// Accept incoming sync connections: receive the peer's payload, merge it, and
/// send our own snapshot back so both sides converge.
fn start_sync_server(app: AppHandle) {
    std::thread::spawn(move || {
        let listener = match TcpListener::bind(("0.0.0.0", SYNC_PORT)) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("devclip sync: sync server unavailable: {e}");
                return;
            }
        };
        for incoming in listener.incoming() {
            let Ok(mut stream) = incoming else { continue };
            let app = app.clone();
            std::thread::spawn(move || {
                if let Err(e) = handle_incoming_sync(&app, &mut stream) {
                    eprintln!("devclip sync: incoming sync failed: {e}");
                }
            });
        }
    });
}

fn handle_incoming_sync(app: &AppHandle, stream: &mut TcpStream) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .ok();
    let peer_ip = stream
        .peer_addr()
        .map(|a| a.ip().to_string())
        .unwrap_or_default();

    let bytes = read_frame(stream).map_err(|e| e.to_string())?;
    let payload: SyncPayload = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let peer_id = payload.device_id.clone();
    let peer_name = payload.device_name.clone();
    let peer_port = payload.sync_port;

    let Some(state) = app.try_state::<AppState>() else {
        return Err("app state unavailable".into());
    };

    // Snapshot OUR data before merging theirs, so we send them a clean copy.
    let our_payload = build_payload(&state)?;
    apply_payload(&state, payload)?;

    let out = serde_json::to_vec(&our_payload).map_err(|e| e.to_string())?;
    write_frame(stream, &out).map_err(|e| e.to_string())?;

    // Remember the peer for one-tap re-sync (and persist).
    if !peer_ip.is_empty() && !peer_id.is_empty() {
        let addr = format!("{peer_ip}:{peer_port}");
        if let Ok(mut s) = state.settings.lock() {
            s.remember_device(&peer_id, &peer_name, &addr, devclip_core::now_unix());
            let _ = s.save(&state.config_path);
        }
    }
    Ok(())
}

// ----- Client-side operations (driven by Tauri commands) ---------------------

/// Broadcast a discovery request and collect replies for ~1.2s.
pub fn discover(app: &AppHandle, window_ms: u64) -> Result<Vec<Peer>, String> {
    let Some(state) = app.try_state::<AppState>() else {
        return Err("app state unavailable".into());
    };
    let (my_id, my_name) = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        (s.device_id.clone(), s.device_name.clone())
    };

    // Ephemeral socket: send the broadcast, receive unicast replies here.
    let socket = UdpSocket::bind(("0.0.0.0", 0)).map_err(|e| e.to_string())?;
    socket.set_broadcast(true).map_err(|e| e.to_string())?;
    socket
        .set_read_timeout(Some(Duration::from_millis(250)))
        .map_err(|e| e.to_string())?;

    let req = Beacon {
        kind: "discover".into(),
        device_id: my_id.clone(),
        device_name: my_name,
        sync_port: SYNC_PORT,
    };
    let req_bytes = serde_json::to_vec(&req).map_err(|e| e.to_string())?;
    socket
        .send_to(&req_bytes, ("255.255.255.255", DISCOVERY_PORT))
        .map_err(|e| e.to_string())?;

    let mut peers: Vec<Peer> = Vec::new();
    let mut buf = [0u8; 2048];
    let deadline = Instant::now() + Duration::from_millis(window_ms);
    while Instant::now() < deadline {
        let (n, from) = match socket.recv_from(&mut buf) {
            Ok(v) => v,
            Err(_) => continue, // timeout tick; keep waiting until deadline
        };
        let Ok(b) = serde_json::from_slice::<Beacon>(&buf[..n]) else {
            continue;
        };
        if b.kind != "announce" || b.device_id == my_id {
            continue;
        }
        if peers.iter().any(|p| p.device_id == b.device_id) {
            continue; // dedup
        }
        peers.push(Peer {
            device_id: b.device_id,
            device_name: b.device_name,
            addr: format!("{}:{}", from.ip(), b.sync_port),
        });
    }
    Ok(peers)
}

/// Connect to a peer at `addr` ("ip:port"), exchange payloads, merge the
/// peer's data locally, and remember the device. Returns what changed locally.
pub fn sync_with(app: &AppHandle, addr: &str) -> Result<MergeStats, String> {
    let Some(state) = app.try_state::<AppState>() else {
        return Err("app state unavailable".into());
    };

    let our_payload = build_payload(&state)?;
    let out = serde_json::to_vec(&our_payload).map_err(|e| e.to_string())?;

    let mut stream = TcpStream::connect(addr)
        .map_err(|e| format!("could not reach {addr}: {e}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(30))).ok();

    write_frame(&mut stream, &out).map_err(|e| e.to_string())?;
    let bytes = read_frame(&mut stream).map_err(|e| e.to_string())?;
    let payload: SyncPayload = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;

    let peer_id = payload.device_id.clone();
    let peer_name = payload.device_name.clone();
    let stats = apply_payload(&state, payload)?;

    if !peer_id.is_empty() {
        if let Ok(mut s) = state.settings.lock() {
            s.remember_device(&peer_id, &peer_name, addr, devclip_core::now_unix());
            let _ = s.save(&state.config_path);
        }
    }
    Ok(stats)
}
