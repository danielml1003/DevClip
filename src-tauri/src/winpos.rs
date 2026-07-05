//! Windows: compute where the palette should open.
//!
//! Behavior: the palette opens **centered on the active window** — the exact
//! app window you were working in (VS Code, a browser, Discord, …), not just
//! the monitor. We read that window's rectangle, center the palette on it, then
//! clamp to the monitor work area so it's always fully on-screen. If the active
//! window can't be measured (or is minimized) we fall back to centering on the
//! active monitor.

#![cfg(windows)]

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect, IsIconic};

/// Top-left for the palette (physical px), centered on the **active window** for
/// the given **logical** window size, clamped to that window's monitor work
/// area. `None` if there's no foreground window (caller centers on the primary
/// display).
pub fn active_window_origin(logical: (i32, i32)) -> Option<(i32, i32)> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }

        // The monitor that owns the active window — the palette is always
        // clamped to this monitor's work area, wherever we center it.
        let work = monitor_work_area(MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST))?;

        // Palette size scaled to the active monitor's DPI.
        let dpi = GetDpiForWindow(fg);
        let scale = if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 };
        let w = (logical.0 as f64 * scale).round() as i32;
        let h = (logical.1 as f64 * scale).round() as i32;

        // Center on the active window's rectangle when we can read it; otherwise
        // (unreadable or minimized) fall back to the monitor work-area center.
        let (center_x, center_y) = match active_window_rect(fg) {
            Some(r) => ((r.left + r.right) / 2, (r.top + r.bottom) / 2),
            None => ((work.left + work.right) / 2, (work.top + work.bottom) / 2),
        };

        let x = center_x - w / 2;
        let y = center_y - h / 2;

        // Clamp so the palette stays fully within the monitor's work area.
        let x = x.min(work.right - w).max(work.left);
        let y = y.min(work.bottom - h).max(work.top);
        Some((x, y))
    }
}

/// The active window's screen rectangle (physical px), or `None` if it's
/// minimized or reports a degenerate rect.
unsafe fn active_window_rect(hwnd: HWND) -> Option<RECT> {
    if IsIconic(hwnd).as_bool() {
        return None; // minimized windows report an off-screen rectangle
    }
    let mut r = RECT::default();
    if GetWindowRect(hwnd, &mut r).is_ok() && r.right > r.left && r.bottom > r.top {
        Some(r)
    } else {
        None
    }
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
