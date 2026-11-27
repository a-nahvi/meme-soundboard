use eframe::egui::Color32;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::ptr::null_mut;
use windows_sys::Win32::{
    Foundation::HWND,
    Graphics::Dwm::{
        DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
    },
};

pub struct Caption {
    window: HWND,
    appearance: Option<[u32; 3]>,
}

impl Caption {
    pub fn new(context: &eframe::CreationContext<'_>) -> Self {
        let window = match context.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Win32(window)) => window.hwnd.get() as HWND,
            _ => null_mut(),
        };
        Self {
            window,
            appearance: None,
        }
    }

    pub fn apply(&mut self, dark: bool, background: Color32, foreground: Color32) {
        let appearance = [u32::from(dark), colorref(background), colorref(foreground)];
        if self.window.is_null() || self.appearance == Some(appearance) {
            return;
        }
        self.appearance = Some(appearance);
        let attributes = [
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            DWMWA_CAPTION_COLOR,
            DWMWA_TEXT_COLOR,
        ];
        // Let Windows render and handle the title bar while using the app theme's colors.
        for (attribute, value) in attributes.into_iter().zip(appearance) {
            unsafe {
                DwmSetWindowAttribute(
                    self.window,
                    attribute as u32,
                    (&raw const value).cast(),
                    size_of::<u32>() as u32,
                );
            }
        }
    }
}

fn colorref(color: Color32) -> u32 {
    u32::from_le_bytes([color.r(), color.g(), color.b(), 0])
}
