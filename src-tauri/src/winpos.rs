//! Windows: compute where the palette should open, anchored to the text caret.
//!
//! This is evaluated *before* our window takes focus (otherwise the foreground
//! window / focused control would be our own palette). It walks a fallback
//! ladder, returning the desired top-left position for the window in physical
//! screen pixels:
//!
//! 1. the active **text caret** (classic Win32 caret), placed just below it;
//! 2. the **focused input control**, placed beneath it;
//! 3. the **center of the focused application window**;
//! 4. the **center of the display** the active app is on;
//! 5. `None` → the caller centers on the primary display.
//!
//! Levels 1–2 are clamped to the work area of the monitor they land on so the
//! window never spills off-screen.

#![cfg(windows)]

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowRect, GetWindowThreadProcessId, GUITHREADINFO,
};

/// Small gap (px) placed between the caret / input box and the palette.
const GAP: i32 = 6;

/// Compute the palette's top-left for the given window size `(w, h)`.
/// Returns `None` only if even the display-center fallback is unavailable, in
/// which case the caller should center the window itself.
pub fn caret_origin(wsize: (i32, i32)) -> Option<(i32, i32)> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }

        let tid = GetWindowThreadProcessId(fg, None);
        let mut gti = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let have_gti = tid != 0 && GetGUIThreadInfo(tid, &mut gti).is_ok();

        // Level 1: the text caret.
        if have_gti && !gti.hwndCaret.0.is_null() {
            let r = gti.rcCaret;
            let mut p = POINT { x: r.left, y: r.bottom };
            if ClientToScreen(gti.hwndCaret, &mut p).as_bool() {
                return Some(clamp_anchor(p.x, p.y + GAP, wsize));
            }
        }

        // Level 2: beneath the focused input control.
        let focus = if have_gti && !gti.hwndFocus.0.is_null() {
            gti.hwndFocus
        } else {
            fg
        };
        if let Some(rect) = window_rect(focus) {
            return Some(clamp_anchor(rect.left, rect.bottom + GAP, wsize));
        }

        // Level 3: the center of the focused application window.
        if let Some(rect) = window_rect(fg) {
            let cx = (rect.left + rect.right) / 2 - wsize.0 / 2;
            let cy = (rect.top + rect.bottom) / 2 - wsize.1 / 2;
            return Some(clamp_anchor(cx, cy, wsize));
        }

        // Level 4: the center of the display the active app is on.
        if let Some(work) = monitor_work_area(fg) {
            let cx = (work.left + work.right) / 2 - wsize.0 / 2;
            let cy = (work.top + work.bottom) / 2 - wsize.1 / 2;
            return Some((cx, cy));
        }

        // Level 5: let the caller center on the primary display.
        None
    }
}

unsafe fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    GetWindowRect(hwnd, &mut rect).ok().map(|_| rect)
}

/// Work area (excludes the taskbar) of the monitor containing `hwnd`.
unsafe fn monitor_work_area(hwnd: HWND) -> Option<RECT> {
    let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
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

/// Clamp a desired top-left `(x, y)` so a window of `wsize` stays within the
/// work area of the monitor nearest that point.
unsafe fn clamp_anchor(x: i32, y: i32, wsize: (i32, i32)) -> (i32, i32) {
    let hmon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if GetMonitorInfoW(hmon, &mut mi).as_bool() {
        let work = mi.rcWork;
        let nx = x.min(work.right - wsize.0).max(work.left);
        let ny = y.min(work.bottom - wsize.1).max(work.top);
        (nx, ny)
    } else {
        (x, y)
    }
}
