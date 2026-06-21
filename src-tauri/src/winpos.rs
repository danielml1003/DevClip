//! Windows: compute where the palette should open, anchored to the text caret.
//!
//! Evaluated *before* our window takes focus (otherwise the foreground window /
//! focused control would be our own palette). Walks a fallback ladder and
//! returns the desired top-left for the window in **physical** screen pixels:
//!
//! 1. the active **text caret** (classic Win32 caret), placed just below it;
//! 2. the **focused input control**, placed beneath it;
//! 3. the **center of the focused application window**;
//! 4. the **center of the display** the active app is on;
//! 5. `None` → the caller centers on the primary display.
//!
//! Each candidate is validated to be on a real monitor and clamped to that
//! monitor's work area, using a window size scaled to the foreground monitor's
//! DPI — so it behaves correctly across multiple monitors with mixed scaling.

#![cfg(windows)]

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetWindowRect, GetWindowThreadProcessId, GUITHREADINFO,
};

/// Small gap (px) placed between the caret / input box and the palette.
const GAP: i32 = 6;

/// Compute the palette's top-left for the given **logical** window size.
/// Returns `None` if no on-screen anchor could be resolved (caller centers).
pub fn caret_origin(logical: (i32, i32)) -> Option<(i32, i32)> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }

        // Physical window size on the foreground app's monitor.
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

        // Level 1: the text caret.
        if have_gti && !gti.hwndCaret.0.is_null() {
            let r = gti.rcCaret;
            let mut p = POINT { x: r.left, y: r.bottom };
            if ClientToScreen(gti.hwndCaret, &mut p).as_bool() {
                if let Some(work) = work_area_at(p.x, p.y) {
                    return Some(clamp(p.x, p.y + GAP, wsize, work));
                }
            }
        }

        // Level 2: beneath the focused input control.
        let focus = if have_gti && !gti.hwndFocus.0.is_null() {
            gti.hwndFocus
        } else {
            fg
        };
        if let Some(rect) = window_rect(focus) {
            if let Some(work) = work_area_at(rect.left, rect.bottom) {
                return Some(clamp(rect.left, rect.bottom + GAP, wsize, work));
            }
        }

        // Level 3: the center of the focused application window.
        if let Some(rect) = window_rect(fg) {
            let mx = (rect.left + rect.right) / 2;
            let my = (rect.top + rect.bottom) / 2;
            if let Some(work) = work_area_at(mx, my) {
                return Some(clamp(mx - wsize.0 / 2, my - wsize.1 / 2, wsize, work));
            }
        }

        // Level 4: the center of the display the active app is on.
        if let Some(work) = monitor_work_area(MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST)) {
            let mx = (work.left + work.right) / 2;
            let my = (work.top + work.bottom) / 2;
            return Some(clamp(mx - wsize.0 / 2, my - wsize.1 / 2, wsize, work));
        }

        None
    }
}

unsafe fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    GetWindowRect(hwnd, &mut rect).ok().map(|_| rect)
}

/// Work area of the monitor under `(x, y)`, or `None` if the point is on no
/// monitor (used to reject bogus anchor coordinates).
unsafe fn work_area_at(x: i32, y: i32) -> Option<RECT> {
    let hmon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL);
    if hmon.0.is_null() {
        return None;
    }
    monitor_work_area(hmon)
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
