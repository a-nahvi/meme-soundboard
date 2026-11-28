use super::{
    App, STOP_ID, SettingsTab, design, icons::Icon, library::shortcut_button,
    shortcut_dialog::ShortcutTarget,
};
use crate::{
    audio::{Endpoint, Source},
    language::Language,
    storage::{Appearance, Settings},
};
use eframe::egui::{self, Margin, RichText, Ui};
use shadcn_egui::components::{
    button::{ButtonSize, ButtonVariant},
    slider::Slider,
    switch::Switch,
};
use std::{ops::RangeInclusive, process::Command, sync::atomic::Ordering::Relaxed};

const ROW_HEIGHT: f32 = 36.0;
const LABEL_WIDTH: f32 = 140.0;

impl App {
    pub(super) fn settings(&mut self, ui: &mut Ui, tab: SettingsTab) {
        let language = self.settings.language;
        design::title(ui, language.text("Settings", "تنظیمات"));
        ui.add_space(12.0);
        egui::ScrollArea::vertical()
            .id_salt("settings-scroll")
            .auto_shrink([false, true])
            .content_margin(Margin::same(2))
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width().min(660.0), 0.0),
                    design::column(ui),
                    |ui| match tab {
                        SettingsTab::Audio => self.routing(ui),
                        SettingsTab::Mix => self.mix(ui),
                        SettingsTab::Shortcuts => self.stop_shortcut(ui),
                        SettingsTab::Appearance => self.appearance(ui),
                        SettingsTab::Updates => self.updates.ui(ui, &self.store),
                        SettingsTab::Diagnostics => self.diagnostics(ui),
                    },
                );
            });
    }

    fn routing(&mut self, ui: &mut Ui) {
        let language = self.settings.language;
        design::settings_card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width() - 50.0, 0.0),
                    design::column(ui),
                    |ui| {
                        design::section_heading(
                            ui,
                            language.text("Audio devices", "دستگاه‌های صوتی"),
                            language.text(
                                "Select the devices for voice chat.",
                                "دستگاه‌های گفت‌وگوی صوتی را انتخاب کنید.",
                            ),
                        )
                    },
                );
                ui.with_layout(design::end_row(ui), |ui| {
                    let refresh =
                        language.text("Refresh audio devices", "تازه‌کردن فهرست دستگاه‌های صوتی");
                    if ui
                        .add_enabled(
                            !self.refreshing,
                            design::toolbar_button(refresh, Icon::REFRESH, ButtonVariant::Outline)
                                .size(ButtonSize::Icon)
                                .busy(self.refreshing),
                        )
                        .on_hover_text(refresh)
                        .clicked()
                    {
                        self.refreshing = true;
                        self.audio.refresh();
                    }
                });
            });
            let p = design::palette(ui);
            let default = language.text("System default", "پیش‌فرض سیستم");
            let mut changed = switch_row(
                ui,
                language.text("Local playback only", "فقط پخش محلی"),
                &mut self.settings.local_playback_only,
            );
            if !self.settings.local_playback_only {
                changed |= device_row(
                    ui,
                    language.text("Microphone", "میکروفون"),
                    default,
                    &self.devices.inputs,
                    &mut self.settings.microphone,
                    None,
                    |_| true,
                );
                let alias = self
                    .devices
                    .soundboard_microphone
                    .as_ref()
                    .map(|device| (device.output_id.as_str(), "Soundboard Microphone"));
                changed |= device_row(
                    ui,
                    language.text("Voice output", "خروجی گفت‌وگو"),
                    language.text("Detect virtual output", "تشخیص خروجی مجازی"),
                    &self.devices.outputs,
                    &mut self.settings.cable,
                    alias,
                    |device| self.settings.headphones.as_ref() != Some(&device.id),
                );
            }
            let cable = if self.settings.local_playback_only {
                None
            } else {
                self.settings.cable.as_ref().or(self.route.cable.as_ref())
            };
            changed |= device_row(
                ui,
                language.text("Headphones", "هدفون"),
                default,
                &self.devices.outputs,
                &mut self.settings.headphones,
                None,
                |device| !device.is_virtual() && cable != Some(&device.id),
            );
            if changed {
                self.settings_changed(ui.ctx());
            }

            ui.add_space(9.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        self.route.headphones.is_some(),
                        design::toolbar_button(
                            language.text("Preview test", "آزمایش پیش‌شنوی"),
                            Icon::HEADPHONES,
                            ButtonVariant::Outline,
                        ),
                    )
                    .clicked()
                {
                    self.test_tone(true);
                }
                if ui
                    .add_enabled(
                        self.route.cable.is_some(),
                        design::toolbar_button(
                            language.text("Send test to chat", "آزمایش خروجی گفت‌وگو"),
                            Icon::PLAY,
                            ButtonVariant::Outline,
                        ),
                    )
                    .clicked()
                {
                    self.test_tone(false);
                }
            });
            let hint = if !self.route_checked {
                language
                    .text("Checking audio devices…", "در حال بررسی دستگاه‌های صوتی…")
                    .into()
            } else if self.settings.local_playback_only {
                language
                    .text(
                        "Clips play through your headphones or speakers.",
                        "کلیپ‌ها از هدفون یا بلندگوی شما پخش می‌شوند.",
                    )
                    .into()
            } else if let Some(mic) = self
                .devices
                .soundboard_microphone
                .as_ref()
                .filter(|mic| self.route.cable.as_ref() == Some(&mic.output_id))
            {
                let name = &mic.recording.name;
                language.text(
                    format!("Choose {name} as the microphone in your game or chat app."),
                    format!("در بازی یا برنامهٔ گفت‌وگو، «{name}» را به‌عنوان میکروفون انتخاب کنید."),
                )
            } else if self.route.cable.is_none() {
                language.text(
                        "Clips play locally. To use voice chat, select Soundboard Microphone in Setup or choose an existing virtual output.",
                        "کلیپ‌ها به‌صورت محلی پخش می‌شوند. برای گفت‌وگوی صوتی، Soundboard Microphone را در نصب‌کننده فعال کنید یا یک خروجی مجازی موجود را انتخاب کنید.",
                    ).into()
            } else if self.devices.outputs.iter().any(|device| {
                Some(&device.id) == self.route.cable.as_ref()
                    && device.name.to_ascii_lowercase().starts_with("cable input")
            }) {
                language
                    .text(
                        "Choose CABLE Output as the microphone in your game or chat app.",
                        "در بازی یا برنامهٔ گفت‌وگو، CABLE Output را به‌عنوان میکروفون انتخاب کنید.",
                    )
                    .into()
            } else {
                language.text("Choose the recording side of your virtual audio device in the game or chat app.", "ورودی ضبط دستگاه صوتی مجازی را در بازی یا برنامهٔ گفت‌وگو انتخاب کنید.").into()
            };
            ui.label(RichText::new(hint).size(12.0).color(p.muted_foreground));
        });
    }

    fn mix(&mut self, ui: &mut Ui) {
        let language = self.settings.language;
        design::settings_card(ui, |ui| {
            design::section_heading(
                ui,
                language.text("Mix", "میکس صدا"),
                language.text(
                    "Balance your voice and clips.",
                    "بلندی صدای میکروفون و کلیپ‌ها را تنظیم کنید.",
                ),
            );
            let settings = &mut self.settings;
            let mut changed = mix_controls(ui, settings);
            changed |= switch_row(
                ui,
                language.text("Voice-activated mic", "فعال‌سازی میکروفون با صدا"),
                &mut self.settings.voice_gate,
            );
            if self.settings.voice_gate {
                let threshold = format!("{:.0} dB", self.settings.threshold_db);
                changed |= slider_row(
                    ui,
                    language.text("Speaking threshold", "آستانهٔ تشخیص صدا"),
                    &mut self.settings.threshold_db,
                    -65.0..=-18.0,
                    1.0,
                    &threshold,
                );
            }
            changed |= switch_row(
                ui,
                language.text(
                    "Hear played clips locally",
                    "شنیدن کلیپ‌ها در هدفون یا بلندگو",
                ),
                &mut self.settings.monitor_clips,
            );
            if changed {
                self.settings_changed(ui.ctx());
            }
        });
    }

    fn stop_shortcut(&mut self, ui: &mut Ui) {
        let language = self.settings.language;
        design::settings_card(ui, |ui| {
            design::section_heading(
                ui,
                language.text("Shortcuts", "میان‌برها"),
                language.text(
                    "Set playback keys directly on clips in Library.",
                    "میان‌بر هر کلیپ را در کتابخانه تنظیم کنید.",
                ),
            );
            setting_row(ui, |ui| {
                ui.label(language.text("Stop playback", "توقف پخش"));
                ui.with_layout(design::end_row(ui), |ui| {
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
                    language.text("Shortcuts work on the desktop and in other apps while Soundboard is running.", "تا وقتی برنامه باز است، میان‌برها روی دسکتاپ و در برنامه‌های دیگر هم کار می‌کنند."),
                );
            if let Some(manager) = &self.shortcuts {
                let (registered, triggered) = manager.status();
                design::muted(
                    ui,
                    language.text(
                        format!("{registered} registered · {triggered} triggered this session"),
                        format!(
                            "{registered} میان‌بر ثبت شده · {triggered} بار استفاده در این نشست"
                        ),
                    ),
                );
            }
            if ui
                .add(design::toolbar_button(
                    language.text("Windows startup settings", "تنظیمات اجرای خودکار ویندوز"),
                    Icon::EXTERNAL_LINK,
                    ButtonVariant::Outline,
                ))
                .clicked()
            {
                ui.ctx()
                    .open_url(egui::OpenUrl::same_tab("ms-settings:startupapps"));
            }
        });
    }

    fn appearance(&mut self, ui: &mut Ui) {
        const MODES: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];
        const LANGUAGES: [Language; 2] = [Language::English, Language::Persian];
        let language = self.settings.language;
        let mut selected_language = usize::from(language.is_rtl());
        design::settings_card(ui, |ui| {
            design::section_heading(
                ui,
                language.text("Appearance", "ظاهر و زبان"),
                language.text(
                    "Language, theme and window behavior.",
                    "زبان، پوسته و رفتار پنجره را تنظیم کنید.",
                ),
            );
            setting_row(ui, |ui| {
                let label = ui.label(language.text("Language", "زبان"));
                ui.with_layout(design::end_row(ui), |ui| {
                    design::select(
                        ui,
                        "interface-language",
                        &mut selected_language,
                        &["English", "فارسی"],
                        160.0,
                    )
                    .labelled_by(label.id);
                });
            });
            let mut mode = MODES
                .iter()
                .position(|mode| *mode == self.settings.appearance)
                .unwrap_or_default();
            setting_row(ui, |ui| {
                let label = ui.label(language.text("Theme", "پوسته"));
                let modes = language.text(["System", "Light", "Dark"], ["سیستم", "روشن", "تیره"]);
                ui.with_layout(design::end_row(ui), |ui| {
                    design::select(ui, "appearance-theme", &mut mode, &modes, 160.0)
                        .labelled_by(label.id);
                });
            });
            if self.settings.appearance != MODES[mode] {
                self.settings.appearance = MODES[mode];
                design::set_mode(ui.ctx(), self.settings.appearance);
                self.queue_settings_save(ui.ctx());
            }
            let mut enabled = self.settings.close_to_tray;
            if switch_row(
                ui,
                language.text("Close to tray", "رفتن به ناحیهٔ اعلان هنگام بستن"),
                &mut enabled,
            ) {
                match self.tray.enable(enabled, language) {
                    Ok(()) => {
                        self.settings.close_to_tray = enabled;
                        self.queue_settings_save(ui.ctx());
                    }
                    Err(err) => {
                        self.error(
                            format!("Couldn't create the Windows tray icon. {err}"),
                            "ایجاد نماد برنامه در ناحیهٔ اعلان ویندوز ممکن نشد.",
                        );
                    }
                }
            }
            if self.settings.close_to_tray {
                design::muted(
                    ui,
                    language.text(
                        "Use Quit from the tray menu to exit Soundboard.",
                        "برای بستن کامل برنامه، در منوی ناحیهٔ اعلان «خروج» را انتخاب کنید.",
                    ),
                );
            }
        });
        if language != LANGUAGES[selected_language] {
            self.change_language(ui.ctx(), LANGUAGES[selected_language]);
        }
    }

    fn diagnostics(&mut self, ui: &mut Ui) {
        let language = self.settings.language;
        design::settings_card(ui, |ui| {
            design::section_heading(
                ui,
                language.text("Diagnostics", "عیب‌یابی"),
                language.text(
                    "Open your clip files, settings and soundboard.log.",
                    "به فایل‌های کلیپ، تنظیمات و گزارش soundboard.log دسترسی داشته باشید.",
                ),
            );
            let underruns = self.audio.meter.underruns.load(Relaxed);
            design::muted(
                ui,
                language.text(
                    format!("{underruns} mic buffer interruptions this session"),
                    format!("{underruns} وقفه در بافر میکروفون در این نشست"),
                ),
            );
            let device_xruns = self.audio.meter.device_xruns.load(Relaxed);
            design::muted(
                ui,
                language.text(
                    format!("{device_xruns} audio device buffer interruptions this session"),
                    format!("{device_xruns} وقفه در بافر دستگاه صوتی در این نشست"),
                ),
            );
            if ui
                .add(design::toolbar_button(
                    language.text("Open data folder", "باز کردن پوشهٔ داده‌ها"),
                    Icon::FOLDER,
                    ButtonVariant::Outline,
                ))
                .clicked()
                && let Err(err) = Command::new("explorer.exe").arg(&self.store.root).spawn()
            {
                self.error(
                    format!("Couldn't open the data folder. {err}"),
                    "باز کردن پوشهٔ داده‌ها ممکن نشد.",
                );
            }
        });
    }

    fn test_tone(&mut self, preview: bool) {
        if let Err(err) = self.request_play(
            Source::Memory(test_wav(), "wav".into()),
            None,
            self.settings
                .language
                .text("Test tone", "صدای آزمایشی")
                .into(),
            preview,
            None,
        ) {
            self.error(err.to_string(), "پخش صدای آزمایشی ممکن نشد.");
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
                names.push(design::language(ui).text("Device unavailable", "دستگاه در دسترس نیست"));
                names.len() - 1
            }
        },
        None => 0,
    };
    let previous = index;
    setting_row(ui, |ui| {
        let title = design::column_label(ui, LABEL_WIDTH, ROW_HEIGHT, egui::Label::new(label));
        design::select(ui, label, &mut index, &names, ui.available_width()).labelled_by(title.id);
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
    let language = settings.language;
    let mic = format!("{:.0}%", settings.mic_volume * 100.0);
    let clips = format!("{:.0}%", settings.clip_volume * 100.0);
    let changed = slider_row(
        ui,
        language.text("Microphone", "میکروفون"),
        &mut settings.mic_volume,
        0.0..=2.0,
        0.01,
        &mic,
    );
    slider_row(
        ui,
        language.text("Clips", "کلیپ‌ها"),
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
            .allocate_ui_with_layout(egui::vec2(width, ROW_HEIGHT), design::row(ui), |ui| {
                ui.add(Slider::new(value, range).step(step).label(name))
                    .labelled_by(label.id)
            })
            .inner;
        ui.allocate_ui_with_layout(
            egui::vec2(VALUE_WIDTH, ROW_HEIGHT),
            design::end_row(ui),
            |ui| design::muted(ui, text),
        );
        response.changed()
    })
}

fn switch_row(ui: &mut Ui, name: &str, value: &mut bool) -> bool {
    setting_row(ui, |ui| {
        let label = ui.label(name);
        ui.with_layout(design::end_row(ui), |ui| {
            let toggle = ui.add(Switch::new(value)).labelled_by(label.id);
            toggle.changed()
        })
        .inner
    })
}

fn setting_row<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> R {
    let size = egui::vec2(ui.available_width(), ROW_HEIGHT);
    ui.allocate_ui_with_layout(size, design::row(ui), contents)
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
