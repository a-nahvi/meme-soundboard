use super::{
    App, STOP_ID, SettingsTab, design, library::shortcut_button, shortcut_dialog::ShortcutTarget,
};
use crate::{
    audio::{Endpoint, Source},
    storage::{Appearance, Settings},
};
use eframe::egui::{self, Align, Layout, Margin, RichText, Ui};
use shadcn_egui::{
    components::{
        button::{Button, ButtonSize, ButtonVariant},
        card::{Card, CardSize, card_content, card_header, card_header_with_action},
        icon::Glyph,
        select::Select,
        slider::Slider,
        switch::Switch,
    },
    icons::lucide,
};
use std::{ops::RangeInclusive, process::Command, sync::atomic::Ordering::Relaxed};

const ROW_HEIGHT: f32 = 36.0;
const LABEL_WIDTH: f32 = 140.0;

impl App {
    pub(super) fn settings(&mut self, ui: &mut Ui, tab: SettingsTab) {
        design::title(ui, "Settings");
        ui.add_space(12.0);
        egui::ScrollArea::vertical()
            .id_salt("settings-scroll")
            .auto_shrink([false, true])
            .content_margin(Margin::same(2))
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(660.0));
                match tab {
                    SettingsTab::Audio => self.routing(ui),
                    SettingsTab::Mix => self.mix(ui),
                    SettingsTab::Shortcuts => self.stop_shortcut(ui),
                    SettingsTab::Appearance => self.appearance(ui),
                    SettingsTab::Updates => self.updates.ui(ui, &self.store.root),
                    SettingsTab::Diagnostics => self.diagnostics(ui),
                };
            });
    }

    fn routing(&mut self, ui: &mut Ui) {
        Card::new().size(CardSize::Sm).show(ui, |ui| {
            card_header_with_action(ui, "Audio devices", "Select the devices for voice chat.", |ui| {
                let glyph = if self.refreshing { Glyph::Spinner } else { lucide::REFRESH_CW.into() };
                if ui.add_enabled(!self.refreshing, Button::new("Refresh audio devices")
                    .variant(ButtonVariant::Outline).size(ButtonSize::Icon).icon(glyph))
                    .on_hover_text("Refresh audio devices").clicked()
                {
                    self.refreshing = true;
                    self.audio.refresh();
                }
            });
            card_content(ui, |ui| {
                let p = design::palette(ui);
                let mut changed = switch_row(ui, "Local playback only", &mut self.settings.local_playback_only);
                if !self.settings.local_playback_only {
                    changed |= device_row(ui, "Microphone", "System default", &self.devices.inputs,
                        &mut self.settings.microphone, None, |_| true);
                    let alias = self.devices.soundboard_microphone.as_ref()
                        .map(|device| (device.output_id.as_str(), "Soundboard Microphone"));
                    changed |= device_row(ui, "Voice output", "Detect virtual output", &self.devices.outputs,
                        &mut self.settings.cable, alias, |device| self.settings.headphones.as_ref() != Some(&device.id));
                }
                let cable = if self.settings.local_playback_only {
                    None
                } else {
                    self.settings.cable.as_ref().or(self.route.cable.as_ref())
                };
                changed |= device_row(ui, "Headphones", "System default", &self.devices.outputs,
                    &mut self.settings.headphones, None, |device| !device.is_virtual() && cable != Some(&device.id));
                if changed { self.settings_changed(ui.ctx()); }

                ui.add_space(9.0);
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(self.route.headphones.is_some(), design::toolbar_button(
                        "Preview test", lucide::HEADPHONES, ButtonVariant::Outline,
                    )).clicked() {
                        self.test_tone(true);
                    }
                    if ui.add_enabled(self.route.cable.is_some(), design::toolbar_button(
                        "Send test to chat", lucide::PLAY, ButtonVariant::Outline,
                    )).clicked() {
                        self.test_tone(false);
                    }
                });
                let hint = if !self.route_checked {
                    "Checking audio devices…".into()
                } else if self.settings.local_playback_only {
                    "Clips play through your headphones or speakers.".into()
                } else if let Some(mic) = self.devices.soundboard_microphone.as_ref().filter(|mic|
                    self.route.cable.as_ref() == Some(&mic.output_id)) {
                    format!("Choose {} as the microphone in your game or chat app.", mic.recording.name)
                } else if self.route.cable.is_none() {
                    "Clips play locally. To use voice chat, select Soundboard Microphone in Setup or choose an existing virtual output.".into()
                } else if self.devices.outputs.iter().any(|device|
                    Some(&device.id) == self.route.cable.as_ref()
                    && device.name.to_ascii_lowercase().starts_with("cable input")) {
                    "Choose CABLE Output as the microphone in your game or chat app.".into()
                } else {
                    "Choose the recording side of your virtual audio device in the game or chat app.".into()
                };
                ui.label(RichText::new(hint).size(12.0).color(p.muted_foreground));
            });
        });
    }

    fn mix(&mut self, ui: &mut Ui) {
        Card::new().size(CardSize::Sm).show(ui, |ui| {
            card_header(ui, "Mix", "Balance your voice and clips.");
            card_content(ui, |ui| {
                let settings = &mut self.settings;
                let mut changed = mix_controls(ui, settings);
                changed |= switch_row(ui, "Voice-activated mic", &mut self.settings.voice_gate);
                if self.settings.voice_gate {
                    let threshold = format!("{:.0} dB", self.settings.threshold_db);
                    changed |= slider_row(
                        ui,
                        "Speaking threshold",
                        &mut self.settings.threshold_db,
                        -65.0..=-18.0,
                        1.0,
                        &threshold,
                    );
                }
                changed |= switch_row(
                    ui,
                    "Hear played clips locally",
                    &mut self.settings.monitor_clips,
                );
                if changed {
                    self.settings_changed(ui.ctx());
                }
            });
        });
    }

    fn stop_shortcut(&mut self, ui: &mut Ui) {
        Card::new().size(CardSize::Sm).show(ui, |ui| {
            card_header(
                ui,
                "Shortcuts",
                "Set playback keys directly on clips in Library.",
            );
            card_content(ui, |ui| {
                setting_row(ui, |ui| {
                    ui.label("Stop playback");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if shortcut_button(
                            ui,
                            Some(&self.settings.stop_key),
                            self.shortcut_registered(STOP_ID),
                        )
                        .clicked()
                        {
                            self.open_shortcut(ShortcutTarget::Stop, ui.ctx());
                        }
                    });
                });
                design::muted(
                    ui,
                    "Shortcuts work on the desktop and in other apps while Soundboard is running.",
                );
                if let Some(manager) = &self.shortcuts {
                    let (registered, triggered) = manager.status();
                    design::muted(
                        ui,
                        format!("{registered} registered · {triggered} triggered this session"),
                    );
                }
                if ui
                    .add(design::toolbar_button(
                        "Windows startup settings",
                        lucide::ARROW_UP_RIGHT,
                        ButtonVariant::Outline,
                    ))
                    .clicked()
                {
                    ui.ctx()
                        .open_url(egui::OpenUrl::same_tab("ms-settings:startupapps"));
                }
            });
        });
    }

    fn appearance(&mut self, ui: &mut Ui) {
        const MODES: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];
        Card::new().size(CardSize::Sm).show(ui, |ui| {
            card_header(ui, "Appearance", "Theme and window behavior.");
            card_content(ui, |ui| {
                let mut mode = MODES
                    .iter()
                    .position(|mode| *mode == self.settings.appearance)
                    .unwrap_or_default();
                setting_row(ui, |ui| {
                    let label = ui.label("Theme");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add(
                            Select::new("Theme", &mut mode, &["System", "Light", "Dark"])
                                .width(160.0),
                        )
                        .labelled_by(label.id);
                    });
                });
                if self.settings.appearance != MODES[mode] {
                    self.settings.appearance = MODES[mode];
                    design::set_mode(ui.ctx(), self.settings.appearance);
                    self.queue_settings_save(ui.ctx());
                }
                let mut enabled = self.settings.close_to_tray;
                if switch_row(ui, "Close to tray", &mut enabled) {
                    match self.tray.enable(enabled) {
                        Ok(()) => {
                            self.settings.close_to_tray = enabled;
                            self.queue_settings_save(ui.ctx());
                        }
                        Err(err) => {
                            self.error(format!("Couldn't create the Windows tray icon. {err}"))
                        }
                    }
                }
                if self.settings.close_to_tray {
                    design::muted(ui, "Use Quit from the tray menu to exit Soundboard.");
                }
            });
        });
    }

    fn diagnostics(&mut self, ui: &mut Ui) {
        Card::new().size(CardSize::Sm).show(ui, |ui| {
            card_header(
                ui,
                "Diagnostics",
                "Open your clip files, settings and soundboard.log.",
            );
            card_content(ui, |ui| {
                let underruns = self.audio.meter.underruns.load(Relaxed);
                design::muted(
                    ui,
                    format!("{underruns} mic buffer interruptions this session"),
                );
                let device_xruns = self.audio.meter.device_xruns.load(Relaxed);
                design::muted(
                    ui,
                    format!("{device_xruns} audio device buffer interruptions this session"),
                );
                if ui
                    .add(design::toolbar_button(
                        "Open data folder",
                        lucide::FOLDER_OPEN,
                        ButtonVariant::Outline,
                    ))
                    .clicked()
                    && let Err(err) = Command::new("explorer.exe").arg(&self.store.root).spawn()
                {
                    self.error(format!("Couldn't open the data folder. {err}"));
                }
            });
        });
    }

    fn test_tone(&mut self, preview: bool) {
        if let Err(err) = self.request_play(
            Source::Memory(test_wav(), "wav".into()),
            None,
            "Test tone".into(),
            preview,
            None,
        ) {
            self.error(err.to_string());
        }
    }
}

fn device_row(
    ui: &mut Ui,
    label: &str,
    default: &str,
    endpoints: &[Endpoint],
    setting: &mut Option<String>,
    alias: Option<(&str, &str)>,
    include: impl Fn(&Endpoint) -> bool,
) -> bool {
    let devices: Vec<_> = endpoints.iter().filter(|device| include(device)).collect();
    let mut names = Vec::with_capacity(devices.len() + 2);
    names.push(default);
    names.extend(devices.iter().map(|device| {
        alias
            .filter(|(id, _)| *id == device.id)
            .map_or(device.name.as_str(), |(_, name)| name)
    }));
    let mut index = match setting.as_ref() {
        Some(id) => match devices.iter().position(|device| &device.id == id) {
            Some(position) => position + 1,
            None => {
                names.push("Device unavailable");
                names.len() - 1
            }
        },
        None => 0,
    };
    let previous = index;
    setting_row(ui, |ui| {
        let title = design::column_label(ui, LABEL_WIDTH, ROW_HEIGHT, egui::Label::new(label));
        ui.add(Select::new(label, &mut index, &names).width(ui.available_width()))
            .labelled_by(title.id);
    });
    if previous == index {
        return false;
    }
    if index == 0 {
        *setting = None;
        return true;
    }
    if let Some(device) = devices.get(index - 1) {
        *setting = Some(device.id.clone());
        return true;
    }
    false
}

fn mix_controls(ui: &mut Ui, settings: &mut Settings) -> bool {
    let mic = format!("{:.0}%", settings.mic_volume * 100.0);
    let clips = format!("{:.0}%", settings.clip_volume * 100.0);
    let changed = slider_row(
        ui,
        "Microphone",
        &mut settings.mic_volume,
        0.0..=2.0,
        0.01,
        &mic,
    );
    slider_row(
        ui,
        "Clips",
        &mut settings.clip_volume,
        0.0..=1.5,
        0.01,
        &clips,
    ) || changed
}

fn slider_row(
    ui: &mut Ui,
    name: &str,
    value: &mut f32,
    range: RangeInclusive<f32>,
    step: f32,
    text: &str,
) -> bool {
    setting_row(ui, |ui| {
        const VALUE_WIDTH: f32 = 57.0;
        let label = design::column_label(ui, LABEL_WIDTH, ROW_HEIGHT, egui::Label::new(name));
        let width = ui.available_width() - VALUE_WIDTH - ui.spacing().item_spacing.x;
        let response = ui
            .allocate_ui_with_layout(
                egui::vec2(width, ROW_HEIGHT),
                Layout::left_to_right(Align::Center),
                |ui| {
                    ui.add(Slider::new(value, range).step(step).label(name))
                        .labelled_by(label.id)
                },
            )
            .inner;
        ui.allocate_ui_with_layout(
            egui::vec2(VALUE_WIDTH, ROW_HEIGHT),
            Layout::right_to_left(Align::Center),
            |ui| design::muted(ui, text),
        );
        response.changed()
    })
}

fn switch_row(ui: &mut Ui, name: &str, value: &mut bool) -> bool {
    setting_row(ui, |ui| {
        let label = ui.label(name);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let toggle = ui.add(Switch::new(value)).labelled_by(label.id);
            toggle.changed()
        })
        .inner
    })
}

fn setting_row<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> R {
    let size = egui::vec2(ui.available_width(), ROW_HEIGHT);
    ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), contents)
        .inner
}

fn test_wav() -> Vec<u8> {
    const RATE: u32 = 24_000;
    const SAMPLES: u32 = RATE / 2;
    let mut bytes = Vec::with_capacity(44 + SAMPLES as usize * 2);
    bytes.extend(b"RIFF");
    bytes.extend((36 + SAMPLES * 2).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16_u32.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(RATE.to_le_bytes());
    bytes.extend((RATE * 2).to_le_bytes());
    bytes.extend(2_u16.to_le_bytes());
    bytes.extend(16_u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend((SAMPLES * 2).to_le_bytes());
    for i in 0..SAMPLES {
        let time = i as f32 / RATE as f32;
        let amp = (time * 150.0).min(1.0) * (-8.0 * time).exp();
        let wave = (time * std::f32::consts::TAU * 660.0).sin() * 0.7
            + (time * std::f32::consts::TAU * 990.0).sin() * 0.3;
        bytes.extend(((amp * wave * 12_500.0) as i16).to_le_bytes());
    }
    bytes
}
