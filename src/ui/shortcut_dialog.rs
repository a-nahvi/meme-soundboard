use super::{App, design, icons::Icon, library::render_error};
use eframe::egui::{self, Event, Key, Margin, Modifiers, Ui};
use global_hotkey::hotkey::HotKey;
use shadcn_egui::components::button::{ButtonSize, ButtonVariant};

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
        let language = design::language(ui);
        let p = design::palette(ui);
        let frame = design::panel(ui)
            .fill(p.background)
            .inner_margin(Margin::same(16));
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            if self.manual {
                ui.with_layout(design::column(ui), |ui| {
                    let label = ui.label(language.text("Key combination", "ترکیب کلیدها"));
                    design::text_input(
                        ui,
                        &mut self.typed,
                        language.text("Ctrl+Alt+1 or Numpad1", "Ctrl+Alt+1 یا Numpad1"),
                        f32::INFINITY,
                    )
                    .labelled_by(label.id);
                });
            } else {
                ui.horizontal(|ui| {
                    design::icon(ui, Icon::KEYBOARD, 18.0, p.primary);
                    if self.recording {
                        ui.label(language.text(
                            "Press a key or combination",
                            "یک کلید یا ترکیب کلیدها را فشار دهید",
                        ));
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
                        design::action_button(
                            language.text("Type manually", "ورود دستی"),
                            ButtonVariant::Ghost,
                        )
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
                        design::toolbar_button(
                            language.text("Record keys", "ضبط کلیدها"),
                            Icon::KEYBOARD,
                            ButtonVariant::Ghost,
                        )
                        .size(ButtonSize::Sm),
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
                design::muted(ui, language.text("Current", "میان‌بر فعلی"));
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
        let language = self.settings.language;
        let title = match dialog.target {
            ShortcutTarget::Clip(_) => language.text("Playback shortcut", "میان‌بر پخش"),
            ShortcutTarget::Stop => language.text("Stop shortcut", "میان‌بر توقف"),
        };
        let mut open = true;
        let mut binding = None;
        let mut cancel = false;
        design::dialog(ctx, "shortcut-dialog", title, 430.0, &mut open, |ui| {
            design::muted(ui, dialog.name.as_str());
            dialog.capture_area(ui);
            render_error(ui, dialog.error.as_deref());
            ui.with_layout(design::end_row(ui), |ui| {
                if ui
                    .add_enabled(
                        dialog.pending().is_some(),
                        design::toolbar_button(
                            language.text("Save shortcut", "ذخیرهٔ میان‌بر"),
                            Icon::CHECK,
                            ButtonVariant::Default,
                        ),
                    )
                    .clicked()
                {
                    binding = dialog.pending().map(str::to_owned);
                }
                cancel = ui
                    .add(design::action_button(
                        language.text("Cancel", "انصراف"),
                        ButtonVariant::Outline,
                    ))
                    .clicked();
                if !dialog.current.is_empty()
                    && ui
                        .add(design::action_button(
                            language.text("Remove", "حذف میان‌بر"),
                            ButtonVariant::Ghost,
                        ))
                        .clicked()
                {
                    binding = Some(String::new());
                }
            });
        });

        if let Some(key) = binding {
            match self.save_shortcut(&dialog.target, &key) {
                Ok(()) => {
                    if key.is_empty() {
                        self.message("Shortcut removed", "میان‌بر حذف شد");
                    } else {
                        self.message("Shortcut saved", "میان‌بر ذخیره شد");
                    }
                    open = false;
                }
                Err(err) => {
                    self.store.log(&format!("Shortcut: {err:#}"));
                    dialog.error = Some(language.text(
                        err.to_string(),
                        "ذخیرهٔ میان‌بر ممکن نشد. ترکیب دیگری را امتحان کنید یا گزارش عیب‌یابی را ببینید.".to_owned(),
                    ));
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
