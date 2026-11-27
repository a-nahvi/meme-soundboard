use super::{App, LIBRARY_FILE, clip_count, design, shortcut_dialog::ShortcutTarget};
use crate::{audio::ClipProgress, storage::Clip};
use eframe::egui::{
    self, Align, CornerRadius, Frame, Layout, Margin, RichText, Sense, Stroke, Ui, UiBuilder, Vec2,
};
use shadcn_egui::{
    components::{
        badge::{Badge, BadgeVariant},
        button::{Button, ButtonSize, ButtonVariant},
        dialog::{Dialog, footer},
        icon::{Glyph, Seg},
        input::Input,
    },
    icons::lucide,
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
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                design::title(ui, "Library");
                design::muted(ui, clip_count(self.clips.len()));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (label, icon) = if self.importing {
                    ("Importing…", Glyph::Spinner)
                } else {
                    ("Import audio", lucide::PLUS.into())
                };
                if ui
                    .add_enabled(
                        !self.importing,
                        Button::new(label).size(ButtonSize::Lg).icon(icon),
                    )
                    .clicked()
                {
                    self.import_files(ui.ctx());
                }
                if ui
                    .add(
                        Button::new("Open clips folder")
                            .icon(lucide::FOLDER_OPEN)
                            .variant(ButtonVariant::Outline)
                            .size(ButtonSize::IconLg),
                    )
                    .on_hover_text("Open clips folder")
                    .clicked()
                {
                    let folder = self.store.clips_dir();
                    if let Err(err) = fs::create_dir_all(&folder)
                        .and_then(|()| Command::new("explorer.exe").arg(folder).spawn())
                    {
                        self.error(format!("Couldn't open the clips folder. {err}"));
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
            design::icon(ui, &lucide::SEARCH, 16.0, p.muted_foreground);
            let search = ui.add(
                Input::new(&mut self.filter)
                    .hint("Find a clip…")
                    .max_width(300.0),
            );
            if search.changed() {
                self.update_filter();
            }
            if !self.filter.is_empty()
                && design::icon_button(ui, "Clear search", lucide::X, ButtonVariant::Ghost)
                    .clicked()
            {
                self.filter.clear();
                self.update_filter();
                search.request_focus();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let variant = if self.favorites {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Outline
                };
                let filter = ui.add(design::toolbar_button("Favorites", lucide::STAR, variant));
                if filter
                    .on_hover_text(if self.favorites {
                        "Show all clips"
                    } else {
                        "Show favorite clips"
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
            Err(err) => self.error(err.to_string()),
        }
    }

    pub(super) fn show_clip_details(&mut self, ctx: &egui::Context) {
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
            let description = format!("“{}” will be removed from the clips folder.", clip.name);
            Dialog::new("delete-clip", "Delete clip?")
                .description(&description)
                .width(416.0)
                .show(ctx, &mut open, |ui| {
                    render_error(ui, details.error.as_deref());
                    footer(ui, |ui| {
                        if ui
                            .add(Button::new("Keep clip").variant(ButtonVariant::Outline))
                            .clicked()
                        {
                            cancelled = true;
                        }
                        if ui
                            .add(
                                Button::new("Delete")
                                    .variant(ButtonVariant::Destructive)
                                    .icon(lucide::TRASH_2),
                            )
                            .clicked()
                        {
                            submitted = true;
                        }
                    });
                });
        } else {
            Dialog::new("clip-details", "Rename clip")
                .width(416.0)
                .show(ctx, &mut open, |ui| {
                    ui.vertical(|ui| {
                        let name = ui.label("Name");
                        ui.add(Input::new(&mut details.name).max_width(f32::INFINITY))
                            .labelled_by(name.id);
                        if let Some(credit) = &source {
                            design::muted(ui, credit.as_str());
                        }
                        render_error(ui, details.error.as_deref());
                    });
                    footer(ui, |ui| {
                        if ui.add(Button::new("Rename file")).clicked() {
                            submitted = true;
                        }
                    });
                });
        }

        if submitted {
            let (result, message) = match details.edit {
                ClipEdit::Delete => (self.remove_clip(&details.id), "Clip deleted"),
                ClipEdit::Rename => (self.rename_clip(&details.id, &details.name), "Clip renamed"),
            };
            match result {
                Ok(()) => {
                    self.message(message);
                    open = false;
                }
                Err(err) => details.error = Some(err.to_string()),
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
            .layout(Layout::left_to_right(Align::Center)),
    );
    let mut action = None;
    let (play_icon, play_label, play_action) = if active {
        (
            lucide::SQUARE,
            "Stop clip",
            LibraryAction::Stop(i, !playback.chat),
        )
    } else {
        (
            lucide::PLAY,
            "Play into voice chat",
            LibraryAction::Play(i, false),
        )
    };
    let accessible_play_label = if active {
        format!("Stop {}", clip.name)
    } else {
        format!("Play {} into voice chat", clip.name)
    };
    let play = Button::new(accessible_play_label)
        .icon(play_icon)
        .size(design::ROUND_CONTROL)
        .variant(if active {
            ButtonVariant::Default
        } else {
            ButtonVariant::Secondary
        });
    if row
        .add_enabled(active || !missing, play)
        .on_hover_text(play_label)
        .on_disabled_hover_text("File missing from clips folder")
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
        row.allocate_ui_with_layout(
            Vec2::new(text_width, 36.0),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(text_width);
                ui.spacing_mut().item_spacing.y = 3.0;
                ui.add(label.truncate()).on_hover_text(&clip.name);
                if let Some((progress, preview)) = playback.progress {
                    if let Some(elapsed_ms) = design::timeline(ui, progress, text_width.min(320.0))
                    {
                        action = Some(LibraryAction::Seek(i, preview, elapsed_ms));
                    }
                } else if missing {
                    ui.label(
                        RichText::new("File missing")
                            .size(11.0)
                            .color(p.muted_foreground),
                    );
                } else if let Some(duration) = trimmed_duration {
                    ui.label(
                        RichText::new(design::seconds_label(duration))
                            .size(11.0)
                            .color(p.muted_foreground),
                    )
                    .on_hover_text("Saved trim duration");
                }
            },
        );
    } else {
        design::column_label(&mut row, text_width, 28.0, label).on_hover_text(&clip.name);
    }

    row.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = ACTION_GAP;
        if design::icon_button(
            ui,
            "Delete clip",
            lucide::TRASH_2,
            ButtonVariant::Destructive,
        )
        .clicked()
        {
            action = Some(LibraryAction::Edit(i, ClipEdit::Delete));
        }
        if design::icon_button(ui, "Rename clip", lucide::PENCIL, ButtonVariant::Ghost).clicked() {
            action = Some(LibraryAction::Edit(i, ClipEdit::Rename));
        }
        let trim_label = if clip.trim.is_some() {
            "Edit saved trim"
        } else {
            "Trim clip"
        };
        if ui
            .add_enabled(
                !missing,
                design::toolbar_button(
                    trim_label,
                    lucide::SCISSORS,
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
            (lucide::SQUARE, "Stop preview", ButtonVariant::Secondary)
        } else {
            (
                lucide::HEADPHONES,
                "Preview on headphones",
                ButtonVariant::Ghost,
            )
        };
        if ui
            .add_enabled(
                playback.preview || !missing,
                Button::new(preview_label)
                    .icon(preview_icon)
                    .size(ButtonSize::IconSm)
                    .variant(preview_variant),
            )
            .on_hover_text(preview_label)
            .on_disabled_hover_text("File missing from clips folder")
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
            Layout::right_to_left(Align::Center),
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
                Layout::right_to_left(Align::Center),
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
    let (label, variant) = if favorite {
        ("Remove favorite", ButtonVariant::Secondary)
    } else {
        ("Add favorite", ButtonVariant::Ghost)
    };
    let response = design::icon_button(ui, label, lucide::STAR, variant);
    if favorite && ui.is_rect_visible(response.rect) {
        // Fill the Lucide outline with a triangle fan using its path endpoints.
        let start = response.rect.center() - Vec2::splat(8.0);
        let color = design::favorite_color(ui);
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(response.rect.center(), color);
        for segment in lucide::STAR.layers().flat_map(|layer| layer.segs) {
            let (x, y) = match segment {
                Seg::M(x, y) | Seg::L(x, y) | Seg::Q(_, _, x, y) | Seg::C(_, _, _, _, x, y) => {
                    (x, y)
                }
                Seg::Z => continue,
            };
            mesh.colored_vertex(start + Vec2::new(x, y) * (16.0 / lucide::STAR.size), color);
        }
        let perimeter = mesh.vertices.len() as u32 - 1;
        for index in 1..=perimeter {
            mesh.add_triangle(0, index, index % perimeter + 1);
        }
        ui.painter().add(egui::Shape::mesh(mesh));
    }
    response
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
    let compact = row_compact.unwrap_or_default();
    let key = key.filter(|value| !value.is_empty());
    let icon_only = row_compact.is_some() && (key.is_none() || !registered);
    let (label, tip, icon) = match key {
        Some(value) if !registered => (
            if icon_only {
                format!("Fix shortcut ({value})")
            } else {
                "Fix shortcut".into()
            },
            format!("{value} is unavailable. Choose another shortcut."),
            lucide::TRIANGLE_ALERT,
        ),
        Some(value) => (
            if compact && value.len() <= 10 {
                value.to_owned()
            } else if !compact && value.len() <= 16 {
                value.replace('+', " + ")
            } else {
                "Shortcut".into()
            },
            format!("Change shortcut ({value})"),
            lucide::KEYBOARD,
        ),
        None => (
            "Set shortcut".into(),
            "Assign a global playback shortcut".into(),
            lucide::KEYBOARD,
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
    let (title, hint, icon) = if !filtered {
        (
            "No clips yet",
            "Import audio or drop files into the clips folder.",
            lucide::MUSIC_2,
        )
    } else if favorites {
        (
            "No favorite clips here",
            "Star a clip in Library or clear your filter.",
            lucide::STAR,
        )
    } else {
        ("No matching clips", "Try a different name.", lucide::SEARCH)
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
                    .show(ui, |ui| design::icon(ui, &icon, 24.0, p.ring));
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
