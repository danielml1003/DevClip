//! Windows: compute where the palette should open, anchored to the text caret.
//!
//! Evaluated *before* our window takes focus. The chosen point is ALWAYS
//! clamped to the work area of the monitor that owns the foreground window —
//! i.e. the screen the user is actively working on — so the palette can never
//! land on the wrong monitor or off-screen, regardless of caret quirks. Within
//! that monitor it tries, in order:
//!
//! 1. the active **text caret** (classic Win32 caret), just below it;
//! 2. beneath the **focused input control**;
//! 3. the **center of the active monitor** (always succeeds).
//!
//! Returns the desired top-left for the window in **physical** screen pixels.

#![cfg(windows)]

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowRect, GetWindowThreadProcessId, GUITHREADINFO,
};

/// Small gap (px) placed between the caret / input box and the palette.
const GAP: i32 = 6;

/// Compute the palette's top-left for the given **logical** window size.
/// `None` only if there's no foreground window at all (caller centers).
pub fn caret_origin(logical: (i32, i32)) -> Option<(i32, i32)> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }

        // The active screen = the monitor that owns the foreground window.
        // Everything is clamped here so we never cross monitors or go off-screen.
        let work = monitor_work_area(MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST))?;

        // Window size scaled to that monitor's DPI.
        let dpi = GetDpiForWindow(fg);
        let scale = if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 };
        let wsize = (
            (logical.0 as f64 * scale).round() as i32,
            (logical.1 as f64 * scale).round() as i32,
        );

        let tid = GetWindowThreadProcessId(fg, None);
        let mut gti = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let have_gti = tid != 0 && GetGUIThreadInfo(tid, &mut gti).is_ok();

        // Level 1: the text caret (only if it lands on the active monitor).
        if have_gti && !gti.hwndCaret.0.is_null() {
            let r = gti.rcCaret;
            let mut p = POINT { x: r.left, y: r.bottom };
            if ClientToScreen(gti.hwndCaret, &mut p).as_bool() && point_in(&work, p.x, p.y) {
                return Some(clamp(p.x, p.y + GAP, wsize, work));
            }
        }

        // Level 2: beneath the focused input control (if on the active monitor).
        let focus = if have_gti && !gti.hwndFocus.0.is_null() {
            gti.hwndFocus
        } else {
            fg
        };
        if let Some(rect) = window_rect(focus) {
            let cx = (rect.left + rect.right) / 2;
            let cy = (rect.top + rect.bottom) / 2;
            if point_in(&work, cx, cy) {
                return Some(clamp(rect.left, rect.bottom + GAP, wsize, work));
            }
        }

        // Level 3: center of the active monitor (always valid).
        let cx = (work.left + work.right) / 2 - wsize.0 / 2;
        let cy = (work.top + work.bottom) / 2 - wsize.1 / 2;
        Some(clamp(cx, cy, wsize, work))
    }
}

fn point_in(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

unsafe fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    GetWindowRect(hwnd, &mut rect).ok().map(|_| rect)
}

unsafe fn monitor_work_area(hmon: HMONITOR) -> Option<RECT> {
    if hmon.0.is_null() {
        return None;
    }
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(hmon, &mut mi).as_bool() {
        Some(mi.rcWork)
    } else {
        None
    }
}

/// Clamp a desired top-left so a window of `wsize` stays within `work`.
fn clamp(x: i32, y: i32, wsize: (i32, i32), work: RECT) -> (i32, i32) {
    let nx = x.min(work.right - wsize.0).max(work.left);
    let ny = y.min(work.bottom - wsize.1).max(work.top);
    (nx, ny)
}
