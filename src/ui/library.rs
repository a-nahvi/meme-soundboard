use super::{App, LIBRARY_FILE, clip_count, design, icons::Icon, shortcut_dialog::ShortcutTarget};
use crate::{audio::ClipProgress, storage::Clip};
use eframe::egui::{
    self, CornerRadius, Frame, Margin, RichText, Sense, Stroke, Ui, UiBuilder, Vec2,
};
use shadcn_egui::components::{
    badge::{Badge, BadgeVariant},
    button::{ButtonSize, ButtonVariant},
};
use std::{fs, process::Command};

const ROW_HEIGHT: f32 = 62.0;
const SHORTCUT_WIDTH: f32 = 156.0;
const COMPACT_SHORTCUT_WIDTH: f32 = 112.0;
const ROW_INSET: f32 = 12.0;
const ACTION_SIZE: f32 = 32.0;
const ACTION_GAP: f32 = 4.0;
const FORMAT_WIDTH: f32 = 52.0;
const LIST_INSET: i8 = 6;
const EDGE_RADIUS: u8 = 5;

enum LibraryAction {
    Play(usize, bool),
    Stop(usize, bool),
    Seek(usize, bool, u32),
    Trim(usize),
    Shortcut(usize),
    Favorite(usize),
    Edit(usize, ClipEdit),
}

#[derive(Clone, Copy)]
enum ClipEdit {
    Rename,
    Delete,
}

struct RowPlayback {
    chat: bool,
    preview: bool,
    progress: Option<(ClipProgress, bool)>,
}

pub(super) struct ClipDetails {
    id: String,
    name: String,
    error: Option<String>,
    edit: ClipEdit,
}

impl App {
    pub(super) fn library(&mut self, ui: &mut Ui) {
        let language = self.settings.language;
        ui.horizontal(|ui| {
            ui.with_layout(design::column(ui), |ui| {
                design::title(ui, language.text("Library", "کتابخانه"));
                design::muted(ui, clip_count(self.clips.len(), language));
            });
            ui.with_layout(design::end_row(ui), |ui| {
                let label = if self.importing {
                    language.text("Importing…", "در حال وارد کردن…")
                } else {
                    language.text("Import audio", "وارد کردن صدا")
                };
                if ui
                    .add_enabled(
                        !self.importing,
                        design::toolbar_button(label, Icon::PLUS, ButtonVariant::Default)
                            .size(ButtonSize::Lg)
                            .busy(self.importing),
                    )
                    .clicked()
                {
                    self.import_files(ui.ctx());
                }
                let open_folder = language.text("Open clips folder", "باز کردن پوشهٔ کلیپ‌ها");
                if ui
                    .add(
                        design::toolbar_button(open_folder, Icon::FOLDER, ButtonVariant::Outline)
                            .size(ButtonSize::IconLg),
                    )
                    .on_hover_text(open_folder)
                    .clicked()
                {
                    let folder = self.store.clips_dir();
                    if let Err(err) = fs::create_dir_all(&folder)
                        .and_then(|()| Command::new("explorer.exe").arg(folder).spawn())
                    {
                        self.error(
                            format!("Couldn't open the clips folder. {err}"),
                            "باز کردن پوشهٔ کلیپ‌ها ممکن نشد.",
                        );
                    }
                }
            });
        });
        ui.add_space(13.0);
        if self.clips.is_empty() {
            empty(ui, false, false);
            return;
        }

        let p = design::palette(ui);
        ui.horizontal(|ui| {
            design::icon(ui, Icon::SEARCH, 16.0, p.muted_foreground);
            let search = design::text_input(
                ui,
                &mut self.filter,
                language.text("Find a clip…", "جست‌وجوی کلیپ…"),
                300.0,
            );
            if search.changed() {
                self.update_filter();
            }
            if !self.filter.is_empty()
                && design::icon_button(
                    ui,
                    language.text("Clear search", "پاک کردن جست‌وجو"),
                    Icon::CLOSE,
                    ButtonVariant::Ghost,
                )
                .clicked()
            {
                self.filter.clear();
                self.update_filter();
                search.request_focus();
            }
            ui.with_layout(design::end_row(ui), |ui| {
                let variant = if self.favorites {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Outline
                };
                let icon = if self.favorites {
                    Icon::STAR_FILLED
                } else {
                    Icon::STAR
                };
                let filter = ui.add(design::toolbar_button(
                    language.text("Favorites", "علاقه‌مندی‌ها"),
                    icon,
                    variant,
                ));
                if filter
                    .on_hover_text(if self.favorites {
                        language.text("Show all clips", "نمایش همهٔ کلیپ‌ها")
                    } else {
                        language.text("Show favorite clips", "نمایش کلیپ‌های موردعلاقه")
                    })
                    .clicked()
                {
                    self.favorites = !self.favorites;
                    self.update_filter();
                }
            });
        });
        ui.add_space(8.0);

        if self.visible_clips.is_empty() {
            empty(ui, true, self.favorites);
            return;
        }
        let mut action = None;
        let chat = self.playing(false);
        let preview = self.playing(true);
        design::panel(ui)
            .inner_margin(Margin::same(LIST_INSET))
            .show(ui, |ui| {
                // The scroll viewport is inset past the card's curved corners.
                ui.shrink_clip_rect(ui.max_rect());
                ui.set_width(ui.available_width());
                let spacing = ui.spacing_mut();
                spacing.item_spacing.y = 0.0;
                spacing.scroll.bar_inner_margin = 6.0;
                spacing.scroll.bar_outer_margin = 1.0;
                spacing.scroll.kinetic.rubber_band = false;
                egui::ScrollArea::vertical()
                    .id_salt("clip-list")
                    .max_height(ui.available_height())
                    .min_scrolled_height(0.0)
                    .content_margin(Margin::ZERO)
                    .auto_shrink([false, true])
                    .show_rows(ui, ROW_HEIGHT, self.visible_clips.len(), |ui, rows| {
                        for row in rows {
                            let index = self.visible_clips[row];
                            let clip = &self.clips[index];
                            let progress = [(chat, false), (preview, true)].into_iter().find_map(
                                |(lane, preview)| {
                                    let (play, progress) = lane?;
                                    (play.id.as_deref() == Some(clip.id.as_str()))
                                        .then_some((progress, preview))
                                },
                            );
                            let playback = RowPlayback {
                                chat: self.playback_active(&clip.id, false),
                                preview: self.playback_active(&clip.id, true),
                                progress,
                            };
                            action = clip_row(
                                ui,
                                index,
                                clip,
                                self.shortcut_registered(&clip.id),
                                self.missing.contains(&clip.id),
                                playback,
                                [row == 0, row + 1 == self.visible_clips.len()],
                            )
                            .or(action.take());
                        }
                    });
            });

        match action {
            Some(LibraryAction::Play(index, preview)) => self.play_clip(index, preview),
            Some(LibraryAction::Stop(index, preview)) => {
                let id = self.clips[index].id.clone();
                self.stop_clip(&id, preview);
                ui.ctx().request_repaint();
            }
            Some(LibraryAction::Seek(index, preview, elapsed_ms)) => {
                if self.playback_active(&self.clips[index].id, preview) {
                    self.seek_playback(preview, elapsed_ms);
                }
            }
            Some(LibraryAction::Trim(index)) => self.open_trim(index),
            Some(LibraryAction::Shortcut(index)) => {
                self.open_shortcut(ShortcutTarget::Clip(self.clips[index].id.clone()), ui.ctx());
            }
            Some(LibraryAction::Favorite(index)) => self.toggle_favorite(index),
            Some(LibraryAction::Edit(index, edit)) => {
                let clip = &self.clips[index];
                self.clip_details = Some(ClipDetails {
                    id: clip.id.clone(),
                    name: clip.name.clone(),
                    error: None,
                    edit,
                });
            }
            None => {}
        }
    }

    fn toggle_favorite(&mut self, index: usize) {
        let mut updated = self.clips.clone();
        updated[index].favorite = !updated[index].favorite;
        match self.store.save(LIBRARY_FILE, &updated) {
            Ok(()) => {
                self.clips = updated;
                self.update_filter();
            }
            Err(err) => self.error(err.to_string(), "ذخیرهٔ علاقه‌مندی ممکن نشد."),
        }
    }

    pub(super) fn show_clip_details(&mut self, ctx: &egui::Context) {
        let language = self.settings.language;
        let Some(mut details) = self.clip_details.take() else {
            return;
        };
        let Some(clip) = self.clips.iter().find(|clip| clip.id == details.id) else {
            return;
        };
        let source = clip
            .credit
            .clone()
            .or_else(|| (clip.source != "Local file").then(|| clip.source.clone()));
        let mut open = true;
        let mut submitted = false;
        let mut cancelled = false;
        if matches!(details.edit, ClipEdit::Delete) {
            let name = &clip.name;
            let description = language.text(
                format!("“{name}” will be removed from the clips folder."),
                format!("«{name}» از پوشهٔ کلیپ‌ها حذف می‌شود."),
            );
            design::dialog(
                ctx,
                "delete-clip",
                language.text("Delete clip?", "کلیپ حذف شود؟"),
                416.0,
                &mut open,
                |ui| {
                    design::muted(ui, description);
                    render_error(ui, details.error.as_deref());
                    ui.with_layout(design::end_row(ui), |ui| {
                        if ui
                            .add(design::toolbar_button(
                                language.text("Delete", "حذف"),
                                Icon::DELETE,
                                ButtonVariant::Destructive,
                            ))
                            .clicked()
                        {
                            submitted = true;
                        }
                        if ui
                            .add(design::action_button(
                                language.text("Keep clip", "نگه داشتن کلیپ"),
                                ButtonVariant::Outline,
                            ))
                            .clicked()
                        {
                            cancelled = true;
                        }
                    });
                },
            );
        } else {
            design::dialog(
                ctx,
                "clip-details",
                language.text("Rename clip", "تغییر نام کلیپ"),
                416.0,
                &mut open,
                |ui| {
                    let name = ui.label(language.text("Name", "نام"));
                    design::text_input(ui, &mut details.name, "", f32::INFINITY)
                        .labelled_by(name.id);
                    if let Some(credit) = &source {
                        design::muted(ui, credit.as_str());
                    }
                    render_error(ui, details.error.as_deref());
                    ui.with_layout(design::end_row(ui), |ui| {
                        if ui
                            .add(design::action_button(
                                language.text("Rename file", "تغییر نام فایل"),
                                ButtonVariant::Default,
                            ))
                            .clicked()
                        {
                            submitted = true;
                        }
                    });
                },
            );
        }

        if submitted {
            let (result, english, persian, failure) = match details.edit {
                ClipEdit::Delete => (
                    self.remove_clip(&details.id),
                    "Clip deleted",
                    "کلیپ حذف شد",
                    "حذف کلیپ ممکن نشد. گزارش عیب‌یابی را ببینید.",
                ),
                ClipEdit::Rename => (
                    self.rename_clip(&details.id, &details.name),
                    "Clip renamed",
                    "نام کلیپ تغییر کرد",
                    "تغییر نام ممکن نشد. یک نام معتبر و یکتا وارد کنید یا گزارش عیب‌یابی را ببینید.",
                ),
            };
            match result {
                Ok(()) => {
                    self.message(english, persian);
                    open = false;
                }
                Err(err) => {
                    self.store.log(&format!("{err:#}"));
                    details.error = Some(language.text(err.to_string(), failure.to_owned()));
                }
            }
        }
        if open && !cancelled {
            self.clip_details = Some(details);
        }
    }
}

fn clip_row(
    ui: &mut Ui,
    i: usize,
    clip: &Clip,
    registered: bool,
    missing: bool,
    playback: RowPlayback,
    [first, last]: [bool; 2],
) -> Option<LibraryAction> {
    let language = design::language(ui);
    let active = playback.chat || playback.preview;
    let p = design::palette(ui);
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_HEIGHT), Sense::hover());
    if active {
        let top = if first { EDGE_RADIUS } else { 0 };
        let bottom = if last { EDGE_RADIUS } else { 0 };
        ui.painter().rect_filled(
            rect,
            CornerRadius {
                nw: top,
                ne: top,
                sw: bottom,
                se: bottom,
            },
            p.accent,
        );
    }
    if !first {
        ui.painter()
            .hline(rect.x_range(), rect.top(), Stroke::new(1.0, p.border));
    }
    let mut row = ui.new_child(
        UiBuilder::new()
            .id_salt(&clip.id)
            .max_rect(rect.shrink2(egui::vec2(ROW_INSET, 0.0)))
            .layout(design::row(ui)),
    );
    let mut action = None;
    let (play_icon, play_label, play_action) = if active {
        (
            Icon::STOP,
            language.text("Stop clip", "توقف کلیپ"),
            LibraryAction::Stop(i, !playback.chat),
        )
    } else {
        (
            Icon::PLAY,
            language.text("Play into voice chat", "پخش در گفت‌وگوی صوتی"),
            LibraryAction::Play(i, false),
        )
    };
    let name = &clip.name;
    let accessible_play_label = match (active, language.is_rtl()) {
        (true, false) => format!("Stop {name}"),
        (false, false) => format!("Play {name} into voice chat"),
        (true, true) => format!("توقف «{name}»"),
        (false, true) => format!("پخش «{name}» در گفت‌وگوی صوتی"),
    };
    let missing_file = language.text(
        "File missing from clips folder",
        "فایل در پوشهٔ کلیپ‌ها پیدا نشد",
    );
    let play = design::toolbar_button(
        &accessible_play_label,
        play_icon,
        if active {
            ButtonVariant::Default
        } else {
            ButtonVariant::Secondary
        },
    )
    .size(design::ROUND_CONTROL);
    if row
        .add_enabled(active || !missing, play)
        .on_hover_text(play_label)
        .on_disabled_hover_text(missing_file)
        .clicked()
    {
        action = Some(play_action);
    }
    row.add_space(4.0);
    let compact = rect.width() < 640.0;
    let shortcut_width = if compact {
        COMPACT_SHORTCUT_WIDTH
    } else {
        SHORTCUT_WIDTH
    };
    let actions_width = 5.0 * (ACTION_SIZE + ACTION_GAP);
    let text_width = (row.available_width()
        - actions_width
        - shortcut_width
        - FORMAT_WIDTH
        - row.spacing().item_spacing.x
        - 2.0 * ACTION_GAP)
        .max(88.0);
    let label = egui::Label::new(RichText::new(&clip.name).strong());
    let trimmed_duration = clip
        .trim
        .map(|trim| trim.duration_ms())
        .filter(|&duration| duration != 0);
    if playback.progress.is_some() || missing || trimmed_duration.is_some() {
        row.allocate_ui_with_layout(Vec2::new(text_width, 36.0), design::column(&row), |ui| {
            ui.set_width(text_width);
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.add(label.truncate()).on_hover_text(&clip.name);
            if let Some((progress, preview)) = playback.progress {
                if let Some(elapsed_ms) = design::timeline(ui, progress, text_width.min(320.0)) {
                    action = Some(LibraryAction::Seek(i, preview, elapsed_ms));
                }
            } else if missing {
                ui.label(
                    RichText::new(language.text("File missing", "فایل پیدا نشد"))
                        .size(11.0)
                        .color(p.muted_foreground),
                );
            } else if let Some(duration) = trimmed_duration {
                ui.label(
                    RichText::new(design::seconds_label(duration))
                        .size(11.0)
                        .color(p.muted_foreground),
                )
                .on_hover_text(language.text("Saved trim duration", "مدت برش ذخیره‌شده"));
            }
        });
    } else {
        design::column_label(&mut row, text_width, 28.0, label).on_hover_text(&clip.name);
    }

    row.with_layout(design::end_row(&row), |ui| {
        ui.spacing_mut().item_spacing.x = ACTION_GAP;
        if design::icon_button(
            ui,
            language.text("Delete clip", "حذف کلیپ"),
            Icon::DELETE,
            ButtonVariant::Destructive,
        )
        .clicked()
        {
            action = Some(LibraryAction::Edit(i, ClipEdit::Delete));
        }
        if design::icon_button(
            ui,
            language.text("Rename clip", "تغییر نام کلیپ"),
            Icon::RENAME,
            ButtonVariant::Ghost,
        )
        .clicked()
        {
            action = Some(LibraryAction::Edit(i, ClipEdit::Rename));
        }
        let trim_label = if clip.trim.is_some() {
            language.text("Edit saved trim", "ویرایش برش ذخیره‌شده")
        } else {
            language.text("Trim clip", "برش کلیپ")
        };
        if ui
            .add_enabled(
                !missing,
                design::toolbar_button(
                    trim_label,
                    Icon::TRIM,
                    if clip.trim.is_some() {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    },
                )
                .size(ButtonSize::IconSm),
            )
            .on_hover_text(trim_label)
            .clicked()
        {
            action = Some(LibraryAction::Trim(i));
        }
        if favorite_button(ui, clip.favorite).clicked() {
            action = Some(LibraryAction::Favorite(i));
        }
        let (preview_icon, preview_label, preview_variant) = if playback.preview {
            (
                Icon::STOP,
                language.text("Stop preview", "توقف پیش‌شنوی"),
                ButtonVariant::Secondary,
            )
        } else {
            (
                Icon::HEADPHONES,
                language.text("Preview on headphones", "پیش‌شنوی با هدفون"),
                ButtonVariant::Ghost,
            )
        };
        if ui
            .add_enabled(
                playback.preview || !missing,
                design::toolbar_button(preview_label, preview_icon, preview_variant)
                    .size(ButtonSize::IconSm),
            )
            .on_hover_text(preview_label)
            .on_disabled_hover_text(missing_file)
            .clicked()
        {
            action = Some(if playback.preview {
                LibraryAction::Stop(i, true)
            } else {
                LibraryAction::Play(i, true)
            });
        }
        ui.allocate_ui_with_layout(
            Vec2::new(shortcut_width, ACTION_SIZE),
            design::end_row(ui),
            |ui| {
                ui.set_min_width(shortcut_width);
                if playback_shortcut_button(ui, clip.hotkey.as_deref(), registered, Some(compact))
                    .clicked()
                {
                    action = Some(LibraryAction::Shortcut(i));
                }
            },
        );
        if let Some(format) = clip
            .file
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_ascii_uppercase())
        {
            ui.allocate_ui_with_layout(
                Vec2::new(FORMAT_WIDTH, ACTION_SIZE),
                design::end_row(ui),
                |ui| {
                    ui.set_min_width(FORMAT_WIDTH);
                    ui.add(Badge::new(format).variant(BadgeVariant::Outline));
                },
            );
        }
    });
    action
}

fn favorite_button(ui: &mut Ui, favorite: bool) -> egui::Response {
    let language = design::language(ui);
    if favorite {
        let label = language.text("Remove favorite", "حذف از علاقه‌مندی‌ها");
        let color = design::favorite_color(ui);
        ui.add(
            design::toolbar_button(label, Icon::STAR_FILLED, ButtonVariant::Secondary)
                .size(ButtonSize::IconSm)
                .color(color),
        )
        .on_hover_text(label)
    } else {
        design::icon_button(
            ui,
            language.text("Add favorite", "افزودن به علاقه‌مندی‌ها"),
            Icon::STAR,
            ButtonVariant::Ghost,
        )
    }
}

pub(super) fn shortcut_button(ui: &mut Ui, key: Option<&str>, registered: bool) -> egui::Response {
    playback_shortcut_button(ui, key, registered, None)
}

fn playback_shortcut_button(
    ui: &mut Ui,
    key: Option<&str>,
    registered: bool,
    row_compact: Option<bool>,
) -> egui::Response {
    let language = design::language(ui);
    let compact = row_compact.unwrap_or_default();
    let key = key.filter(|value| !value.is_empty());
    let icon_only = row_compact.is_some() && (key.is_none() || !registered);
    let (label, tip, icon) = match key {
        Some(value) if !registered => (
            if icon_only {
                language.text(
                    format!("Fix shortcut ({value})"),
                    format!("اصلاح میان‌بر ({value})"),
                )
            } else {
                language.text("Fix shortcut", "اصلاح میان‌بر").into()
            },
            language.text(
                format!("{value} is unavailable. Choose another shortcut."),
                format!("میان‌بر {value} در دسترس نیست. میان‌بر دیگری انتخاب کنید."),
            ),
            Icon::WARNING,
        ),
        Some(value) => (
            if compact && value.len() <= 10 {
                value.to_owned()
            } else if !compact && value.len() <= 16 {
                value.replace('+', " + ")
            } else {
                language.text("Shortcut", "میان‌بر").into()
            },
            language.text(
                format!("Change shortcut ({value})"),
                format!("تغییر میان‌بر ({value})"),
            ),
            Icon::KEYBOARD,
        ),
        None => (
            language.text("Set shortcut", "تنظیم میان‌بر").into(),
            language
                .text(
                    "Assign a global playback shortcut",
                    "تعیین میان‌بر سراسری پخش",
                )
                .into(),
            Icon::KEYBOARD,
        ),
    };
    let size = if icon_only {
        ButtonSize::IconSm
    } else {
        ButtonSize::Sm
    };
    ui.add(design::toolbar_button(&label, icon, ButtonVariant::Outline).size(size))
        .on_hover_text(tip)
}

fn empty(ui: &mut Ui, filtered: bool, favorites: bool) {
    let p = design::palette(ui);
    let language = design::language(ui);
    let (title, hint, icon) = if !filtered {
        (
            language.text("No clips yet", "هنوز کلیپی ندارید"),
            language.text(
                "Import audio or drop files into the clips folder.",
                "صدا وارد کنید یا فایل‌ها را در پوشهٔ کلیپ‌ها قرار دهید.",
            ),
            Icon::MUSIC,
        )
    } else if favorites {
        (
            language.text("No favorite clips here", "کلیپ موردعلاقه‌ای پیدا نشد"),
            language.text(
                "Star a clip in Library or clear your filter.",
                "به کلیپی در کتابخانه ستاره بدهید یا فیلتر را پاک کنید.",
            ),
            Icon::STAR,
        )
    } else {
        (
            language.text("No matching clips", "کلیپی پیدا نشد"),
            language.text("Try a different name.", "نام دیگری را جست‌وجو کنید."),
            Icon::SEARCH,
        )
    };
    design::panel(ui)
        .inner_margin(Margin::symmetric(14, 25))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.add_space(18.0);
                Frame::new()
                    .fill(p.secondary)
                    .corner_radius(12)
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| design::icon(ui, icon, 24.0, p.ring));
                ui.add_space(6.0);
                ui.label(RichText::new(title).size(19.0).strong());
                design::muted(ui, hint);
                ui.add_space(19.0);
            });
        });
}

pub(super) fn render_error(ui: &mut Ui, error: Option<&str>) {
    if let Some(error) = error {
        ui.label(RichText::new(error).color(design::palette(ui).destructive));
    }
}
