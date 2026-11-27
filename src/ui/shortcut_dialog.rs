use super::{App, design, library::render_error};
use eframe::egui::{self, Event, Key, Margin, Modifiers, Ui};
use global_hotkey::hotkey::HotKey;
use shadcn_egui::{
    components::{
        button::{Button, ButtonSize, ButtonVariant},
        dialog::{Dialog, footer},
        input::Input,
    },
    icons::lucide,
};

pub(super) enum ShortcutTarget {
    Clip(String),
    Stop,
}

pub(super) struct ShortcutDialog {
    target: ShortcutTarget,
    name: String,
    current: String,
    candidate: Option<String>,
    pub(super) recording: bool,
    manual: bool,
    typed: String,
    error: Option<String>,
}

impl ShortcutDialog {
    pub(super) fn new(target: ShortcutTarget, name: String, current: String) -> Self {
        Self {
            target,
            name,
            current,
            candidate: None,
            recording: true,
            manual: false,
            typed: String::new(),
            error: None,
        }
    }

    pub(super) fn record(&mut self, key: String) {
        self.candidate = Some(key);
        self.recording = false;
        self.error = None;
    }

    fn read_key(&mut self, ctx: &egui::Context) {
        if !self.recording || !ctx.input(|input| input.focused) {
            return;
        }
        let pressed = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                Event::Key {
                    key,
                    physical_key,
                    modifiers,
                    pressed: true,
                    repeat: false,
                } => {
                    let chord = chord_for(*key, *modifiers)
                        .or_else(|| physical_key.and_then(|key| chord_for(key, *modifiers)))?;
                    Some((*key, *modifiers, chord))
                }
                _ => None,
            })
        });
        if let Some((key, modifiers, chord)) = pressed {
            ctx.input_mut(|input| input.consume_key(modifiers, key));
            self.record(chord);
        }
    }

    fn capture_area(&mut self, ui: &mut Ui) {
        let p = design::palette(ui);
        let frame = design::panel(ui)
            .fill(p.background)
            .inner_margin(Margin::same(16));
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            if self.manual {
                ui.vertical(|ui| {
                    let label = ui.label("Key combination");
                    ui.add(
                        Input::new(&mut self.typed)
                            .hint("Ctrl+Alt+1 or Numpad1")
                            .max_width(f32::INFINITY),
                    )
                    .labelled_by(label.id);
                });
            } else {
                ui.horizontal(|ui| {
                    design::icon(ui, &lucide::KEYBOARD, 18.0, p.primary);
                    if self.recording {
                        ui.label("Press a key or combination");
                    } else if let Some(key) = &self.candidate {
                        design::keycaps(ui, key);
                    }
                });
            }
        });
        ui.horizontal(|ui| {
            if !self.manual
                && ui
                    .add(
                        Button::new("Type manually")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm),
                    )
                    .clicked()
            {
                self.typed = self
                    .candidate
                    .clone()
                    .unwrap_or_else(|| self.current.clone());
                self.manual = true;
                self.recording = false;
                self.error = None;
            } else if !self.recording
                && ui
                    .add(
                        Button::new("Record keys")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .icon(lucide::KEYBOARD),
                    )
                    .clicked()
            {
                self.manual = false;
                self.recording = true;
                self.error = None;
            }
        });
        if self.recording && !self.current.is_empty() {
            ui.horizontal(|ui| {
                design::muted(ui, "Current");
                design::keycaps(ui, &self.current);
            });
        }
    }

    fn pending(&self) -> Option<&str> {
        let key = if self.manual {
            Some(self.typed.trim())
        } else if self.recording {
            None
        } else {
            self.candidate.as_deref()
        };
        key.filter(|key| !key.is_empty())
    }
}

impl App {
    pub(super) fn show_shortcut_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.shortcut_dialog.take() else {
            return;
        };
        dialog.read_key(ctx);
        let title = match dialog.target {
            ShortcutTarget::Clip(_) => "Playback shortcut",
            ShortcutTarget::Stop => "Stop shortcut",
        };
        let name = dialog.name.clone();
        let mut open = true;
        let mut binding = None;
        let mut cancel = false;
        Dialog::new("shortcut-dialog", title)
            .description(&name)
            .width(430.0)
            .show(ctx, &mut open, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    dialog.capture_area(ui);
                    render_error(ui, dialog.error.as_deref());
                });
                footer(ui, |ui| {
                    if !dialog.current.is_empty()
                        && ui
                            .add(Button::new("Remove").variant(ButtonVariant::Ghost))
                            .clicked()
                    {
                        binding = Some(String::new());
                    }
                    cancel = ui
                        .add(Button::new("Cancel").variant(ButtonVariant::Outline))
                        .clicked();
                    if ui
                        .add_enabled(
                            dialog.pending().is_some(),
                            Button::new("Save shortcut").icon(lucide::CHECK),
                        )
                        .clicked()
                    {
                        binding = dialog.pending().map(str::to_owned);
                    }
                });
            });

        if let Some(key) = binding {
            match self.save_shortcut(&dialog.target, &key) {
                Ok(()) => {
                    self.message(if key.is_empty() {
                        "Shortcut removed"
                    } else {
                        "Shortcut saved"
                    });
                    open = false;
                }
                Err(err) => {
                    self.store.log(&format!("Shortcut: {err:#}"));
                    dialog.error = Some(err.to_string());
                }
            }
        }
        if let Some(manager) = &self.shortcuts {
            manager
                .capture(open && !cancel && dialog.recording && ctx.input(|input| input.focused));
        }
        if open && !cancel {
            self.shortcut_dialog = Some(dialog);
        }
    }
}

fn chord_for(key: Key, modifiers: Modifiers) -> Option<String> {
    if key == Key::Tab
        || (key == Key::Escape && !(modifiers.ctrl || modifiers.alt || modifiers.shift))
    {
        return None;
    }
    let mut parts = Vec::with_capacity(4);
    if modifiers.ctrl {
        parts.push("Ctrl");
    }
    if modifiers.alt {
        parts.push("Alt");
    }
    if modifiers.shift {
        parts.push("Shift");
    }
    parts.push(match key {
        Key::Plus | Key::Equals => "=",
        Key::OpenBracket | Key::OpenCurlyBracket => "[",
        Key::CloseBracket | Key::CloseCurlyBracket => "]",
        Key::Backtick => "`",
        Key::Colon => ";",
        Key::Pipe => "\\",
        Key::Questionmark => "/",
        _ => key.name(),
    });
    let chord = parts.join("+");
    chord.parse::<HotKey>().ok().map(|_| chord)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_legacy_compatible_chords_and_single_keys() {
        let key = chord_for(
            Key::Num1,
            Modifiers {
                ctrl: true,
                alt: true,
                ..Default::default()
            },
        );
        assert_eq!(key.as_deref(), Some("Ctrl+Alt+1"));
        assert_eq!(chord_for(Key::F8, Modifiers::NONE).as_deref(), Some("F8"));
        assert_eq!(
            chord_for(Key::OpenBracket, Modifiers::CTRL).as_deref(),
            Some("Ctrl+[")
        );
        assert_eq!(chord_for(Key::Escape, Modifiers::NONE), None);
        assert_eq!(chord_for(Key::Tab, Modifiers::NONE), None);
    }
}
