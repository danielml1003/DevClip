//! Windows: compute where the palette should open.
//!
//! Per the chosen behavior, the palette opens **centered on the active screen**
//! — the monitor that owns the foreground window (the app you're working in).
//! This is evaluated before our window takes focus, is always on-screen, and
//! never crosses to the wrong monitor.

#![cfg(windows)]

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

/// Top-left for the palette (physical px), centered on the active monitor for
/// the given **logical** window size. `None` if there's no foreground window
/// (caller centers on the primary display).
pub fn active_screen_origin(logical: (i32, i32)) -> Option<(i32, i32)> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.0.is_null() {
            return None;
        }

        let work = monitor_work_area(MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST))?;

        // Window size scaled to the active monitor's DPI.
        let dpi = GetDpiForWindow(fg);
        let scale = if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 };
        let w = (logical.0 as f64 * scale).round() as i32;
        let h = (logical.1 as f64 * scale).round() as i32;

        let cx = (work.left + work.right) / 2 - w / 2;
        let cy = (work.top + work.bottom) / 2 - h / 2;

        // Clamp defensively (handles oversized windows / odd work areas).
        let x = cx.min(work.right - w).max(work.left);
        let y = cy.min(work.bottom - h).max(work.top);
        Some((x, y))
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
