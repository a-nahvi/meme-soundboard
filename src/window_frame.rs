use eframe::egui::Rect;
use std::{mem::size_of, ptr::null_mut, sync::Mutex};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT},
    Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect, ScreenToClient,
    },
    UI::{
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::{
            GetClientRect, HTBOTTOM, HTBOTTOMLEFT, HTBOTTOMRIGHT, HTCAPTION, HTCLIENT, HTLEFT,
            HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, IsZoomed, SWP_FRAMECHANGED, SWP_NOACTIVATE,
            SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos, WM_NCCALCSIZE, WM_NCHITTEST,
        },
    },
};

static CAPTION: Mutex<[i32; 4]> = Mutex::new([0; 4]);

pub fn apply(window: HWND) {
    // Preserve the resizable window style and ask Windows to recalculate its client area.
    unsafe {
        SetWindowPos(
            window,
            null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

pub fn set_caption(rect: Rect, pixels_per_point: f32) {
    if let Ok(mut caption) = CAPTION.lock() {
        *caption = [rect.left(), rect.top(), rect.right(), rect.bottom()]
            .map(|point| (point * pixels_per_point).round() as i32);
    }
}

pub unsafe fn window_message(window: HWND, message: u32, lparam: LPARAM) -> Option<LRESULT> {
    match message {
        WM_NCCALCSIZE => {
            if lparam != 0 && unsafe { IsZoomed(window) } != 0 {
                // Both forms of WM_NCCALCSIZE put the proposed rectangle at this address.
                let rect = unsafe { &mut *(lparam as *mut RECT) };
                let monitor = unsafe { MonitorFromRect(rect, MONITOR_DEFAULTTONEAREST) };
                let mut info = MONITORINFO {
                    cbSize: size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                if !monitor.is_null() && unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
                    *rect = info.rcWork;
                }
            }
            Some(0)
        }
        WM_NCHITTEST => Some(hit_test(window, lparam) as LRESULT),
        _ => None,
    }
}

fn hit_test(window: HWND, lparam: LPARAM) -> u32 {
    let mut point = POINT {
        x: lparam as i16 as i32,
        y: (lparam >> 16) as i16 as i32,
    };
    if unsafe { ScreenToClient(window, &mut point) } == 0 {
        return HTCLIENT;
    }
    if unsafe { IsZoomed(window) } == 0 {
        let mut rect = RECT::default();
        if unsafe { GetClientRect(window, &mut rect) } != 0 {
            let width = (unsafe { GetDpiForWindow(window) } as i32 * 6 / 96).max(4);
            let x = if point.x < width {
                -1
            } else if point.x >= rect.right - width {
                1
            } else {
                0
            };
            let y = if point.y < width {
                -1
            } else if point.y >= rect.bottom - width {
                1
            } else {
                0
            };
            let edge = match (x, y) {
                (-1, -1) => HTTOPLEFT,
                (1, -1) => HTTOPRIGHT,
                (-1, 1) => HTBOTTOMLEFT,
                (1, 1) => HTBOTTOMRIGHT,
                (0, -1) => HTTOP,
                (0, 1) => HTBOTTOM,
                (-1, 0) => HTLEFT,
                (1, 0) => HTRIGHT,
                _ => HTCLIENT,
            };
            if edge != HTCLIENT {
                return edge;
            }
        }
    }
    let [left, top, right, bottom] = *CAPTION.lock().unwrap_or_else(|error| error.into_inner());
    if (left..right).contains(&point.x) && (top..bottom).contains(&point.y) {
        HTCAPTION
    } else {
        HTCLIENT
    }
}
