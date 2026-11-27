use anyhow::Result;
use eframe::egui;
use std::sync::mpsc::{self, Receiver};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

const ICON_ID: &str = "meme-soundboard";
const SHOW_ID: &str = "soundboard.show";
const STOP_ID: &str = "soundboard.stop";
const QUIT_ID: &str = "soundboard.quit";

pub enum Action {
    Show,
    Stop,
    Quit,
}

pub struct Tray {
    icon: Option<TrayIcon>,
    pub actions: Receiver<Action>,
}

impl Tray {
    pub fn new(ctx: &egui::Context) -> Self {
        let (send, actions) = mpsc::channel();
        let click_send = send.clone();
        let click_ctx = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    id, button: MouseButton::Left, button_state: MouseButtonState::Up, ..
                } if id.0 == ICON_ID
            ) && click_send.send(Action::Show).is_ok()
            {
                click_ctx.request_repaint();
            }
        }));
        let menu_ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = match event.id.0.as_str() {
                SHOW_ID => Action::Show,
                STOP_ID => Action::Stop,
                QUIT_ID => Action::Quit,
                _ => return,
            };
            if send.send(action).is_ok() {
                menu_ctx.request_repaint();
            }
        }));
        Self {
            icon: None,
            actions,
        }
    }

    pub fn enabled(&self) -> bool {
        self.icon.is_some()
    }

    pub fn enable(&mut self, enabled: bool) -> Result<()> {
        if !enabled {
            self.icon = None;
        } else if self.icon.is_none() {
            let menu = Menu::with_items(&[
                &MenuItem::with_id(SHOW_ID, "Open Meme Soundboard", true, None),
                &MenuItem::with_id(STOP_ID, "Stop playback", true, None),
                &PredefinedMenuItem::separator(),
                &MenuItem::with_id(QUIT_ID, "Quit", true, None),
            ])?;
            let logo = crate::window_icon();
            let icon = Icon::from_rgba(logo.rgba, logo.width, logo.height)?;
            self.icon = Some(
                TrayIconBuilder::new()
                    .with_id(ICON_ID)
                    .with_icon(icon)
                    .with_tooltip("Meme Soundboard")
                    .with_menu(Box::new(menu))
                    .with_menu_on_left_click(false)
                    .build()?,
            );
        }
        Ok(())
    }
}
