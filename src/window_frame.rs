use eframe::egui::{Color32, Rect};
use std::{mem::size_of_val, ptr::null_mut, sync::Mutex};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::{
        Dwm::{DWMWA_CAPTION_COLOR, DwmSetWindowAttribute},
        Gdi::ScreenToClient,
    },
    UI::{
        HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
        Shell::DefSubclassProc,
        WindowsAndMessaging::{
            GetClientRect, HTCAPTION, HTCLIENT, SM_CXPADDEDBORDER, SM_CYSIZEFRAME,
            SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
            WM_NCCALCSIZE, WM_NCHITTEST,
        },
    },
};

struct Caption {
    rect: [i32; 4],
    window: usize,
    color: Option<u32>,
}

static CAPTION: Mutex<Caption> = Mutex::new(Caption {
    rect: [0; 4],
    window: 0,
    color: None,
});

pub fn apply(window: HWND) {
    if let Ok(mut caption) = CAPTION.lock() {
        caption.window = window as usize;
        caption.color = None;
    }
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

pub fn set_caption(rect: Rect, pixels_per_point: f32, fill: Color32) {
    let color = u32::from_le_bytes([fill.r(), fill.g(), fill.b(), 0]);
    let window = {
        let Ok(mut caption) = CAPTION.lock() else {
            return;
        };
        caption.rect = [rect.left(), rect.top(), rect.right(), rect.bottom()]
            .map(|point| (point * pixels_per_point).round() as i32);
        if caption.window == 0 || caption.color == Some(color) {
            return;
        }
        caption.color = Some(color);
        caption.window as HWND
    };
    // Match the exposed part of the native caption to the custom header's fill.
    unsafe {
        DwmSetWindowAttribute(
            window,
            DWMWA_CAPTION_COLOR as u32,
            (&raw const color).cast(),
            size_of_val(&color) as u32,
        );
    }
}

pub unsafe fn window_message(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    match message {
        WM_NCCALCSIZE if lparam != 0 => {
            // Both forms of this message start with the proposed window rectangle.
            let rect = lparam as *mut RECT;
            let window_top = unsafe { (*rect).top };
            unsafe { DefSubclassProc(window, message, wparam, lparam) };
            let dpi = unsafe { GetDpiForWindow(window) };
            let border = unsafe {
                GetSystemMetricsForDpi(SM_CYSIZEFRAME, dpi)
                    + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
            };
            // Keep the window manager's frame on all sides. Only reclaim its caption height.
            unsafe { (*rect).top = (*rect).top.min(window_top + border) };
            Some(0)
        }
        WM_NCHITTEST => hit_test_client(window, lparam),
        _ => None,
    }
}

fn hit_test_client(window: HWND, lparam: LPARAM) -> Option<LRESULT> {
    let mut point = POINT {
        x: lparam as i16 as i32,
        y: (lparam >> 16) as i16 as i32,
    };
    let mut client = RECT::default();
    if unsafe { ScreenToClient(window, &mut point) } == 0
        || unsafe { GetClientRect(window, &mut client) } == 0
        || !(client.left..client.right).contains(&point.x)
        || !(client.top..client.bottom).contains(&point.y)
    {
        return None;
    }
    let [left, top, right, bottom] = CAPTION
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .rect;
    let hit = if (left..right).contains(&point.x) && (top..bottom).contains(&point.y) {
        HTCAPTION
    } else {
        HTCLIENT
    };
    Some(hit as LRESULT)
}
