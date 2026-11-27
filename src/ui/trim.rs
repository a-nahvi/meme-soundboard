use super::{App, LIBRARY_FILE, clip_action, design, library::render_error};
use crate::{
    audio::{ClipAudioInfo, Player, Source},
    storage::ClipTrim,
};
use anyhow::{Context, Result};
use eframe::egui::{
    self, Align, DragValue, Layout, PointerButton, Rect, RichText, Sense, Stroke, StrokeKind, Ui,
    pos2, vec2,
};
use shadcn_egui::{
    components::{
        button::{Button, ButtonVariant},
        dialog::Dialog,
    },
    icons::lucide,
};

pub(super) struct TrimDialog {
    request: u64,
    id: String,
    name: String,
    saved: Option<ClipTrim>,
    draft: Option<ClipTrim>,
    info: Option<ClipAudioInfo>,
    error: Option<String>,
    preview_generation: Option<u64>,
    preview_after_edit: bool,
}

enum TrimAction {
    Preview,
    StopPreview,
    Revert,
    Save,
}

impl TrimDialog {
    pub(super) fn receive(&mut self, request: u64, result: Result<ClipAudioInfo, String>) {
        if request != self.request {
            return;
        }
        match result {
            Ok(info) => {
                self.draft = Some(
                    self.saved
                        .and_then(|trim| trim.bounded(info.duration_ms))
                        .unwrap_or_else(|| ClipTrim::full(info.duration_ms)),
                );
                self.info = Some(info);
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn changed_range(&self) -> Option<Option<ClipTrim>> {
        let info = self.info.as_ref()?;
        let draft = self.draft?;
        let trimmed = (draft != ClipTrim::full(info.duration_ms)).then_some(draft);
        (trimmed != self.saved).then_some(trimmed)
    }

    fn stop_preview(&mut self, player: &Player) {
        if let Some(generation) = self.preview_generation.take() {
            player.stop_clip(true, generation);
        }
    }
}

impl App {
    pub(super) fn open_trim(&mut self, index: usize) {
        let clip = &self.clips[index];
        self.trim_request = self.trim_request.wrapping_add(1);
        let mut dialog = TrimDialog {
            request: self.trim_request,
            id: clip.id.clone(),
            name: clip.name.clone(),
            saved: clip.trim,
            draft: None,
            info: None,
            error: None,
            preview_generation: None,
            preview_after_edit: false,
        };
        if let Err(err) = self
            .store
            .clip_path(&clip.file)
            .and_then(|path| self.audio.inspect_clip(path, dialog.request))
        {
            dialog.error = Some(err.to_string());
        }
        self.trim_dialog = Some(dialog);
    }

    pub(super) fn show_trim_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.trim_dialog.take() else {
            return;
        };
        let Some(index) = self.clips.iter().position(|clip| clip.id == dialog.id) else {
            dialog.stop_preview(&self.audio.player);
            return;
        };
        let preview_elapsed = self
            .current_playback(true)
            .filter(|play| Some(play.generation) == dialog.preview_generation)
            .map(|play| {
                play.timeline
                    .as_ref()
                    .map_or(0, |timeline| timeline.progress().elapsed_ms)
            });
        let can_preview = self.route.headphones.is_some();
        let previous_draft = dialog.draft;
        let mut open = true;
        let mut action = None;
        let mut editing = false;
        let name = dialog.name.clone();
        Dialog::new("trim-clip", "Trim clip")
            .description(&name)
            .width(520.0)
            .show(ctx, &mut open, |ui| {
                if let (Some(info), Some(draft)) = (&dialog.info, &mut dialog.draft) {
                    editing = range_editor(ui, info, draft, preview_elapsed);
                } else if dialog.error.is_none() {
                    design::muted(ui, "Reading clip…");
                }
                render_error(ui, dialog.error.as_deref());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let previewing = preview_elapsed.is_some() && dialog.draft == previous_draft;
                    let (label, icon, preview_action) = if previewing {
                        ("Stop preview", lucide::SQUARE, TrimAction::StopPreview)
                    } else {
                        ("Preview", lucide::PLAY, TrimAction::Preview)
                    };
                    if ui
                        .add_enabled(
                            dialog.draft.is_some() && (previewing || can_preview),
                            design::toolbar_button(label, icon, ButtonVariant::Outline),
                        )
                        .on_hover_text("Listen on headphones or speakers")
                        .on_disabled_hover_text("Choose an output in Audio devices")
                        .clicked()
                    {
                        action = Some(preview_action);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let changed = dialog.changed_range().is_some();
                        if ui.add_enabled(changed, Button::new("Save trim")).clicked() {
                            action = Some(TrimAction::Save);
                        }
                        if ui
                            .add_enabled(
                                dialog.saved.is_some() || changed,
                                Button::new("Revert").variant(ButtonVariant::Ghost),
                            )
                            .on_hover_text("Restore the full clip")
                            .clicked()
                        {
                            action = Some(TrimAction::Revert);
                        }
                    });
                });
            });
        if !open {
            dialog.stop_preview(&self.audio.player);
            return;
        }
        if dialog.draft != previous_draft {
            dialog.stop_preview(&self.audio.player);
            dialog.preview_after_edit = true;
        }
        let auto_preview =
            !editing && std::mem::take(&mut dialog.preview_after_edit) && can_preview;
        let action = action.or_else(|| auto_preview.then_some(TrimAction::Preview));
        match action {
            Some(TrimAction::Save) => {
                if let Some(trim) = dialog.changed_range() {
                    match self.set_clip_trim(&dialog.id, trim) {
                        Ok(()) => {
                            dialog.stop_preview(&self.audio.player);
                            self.message(if trim.is_some() {
                                "Trim saved"
                            } else {
                                "Full clip restored"
                            });
                            if can_preview {
                                self.play_clip(index, true);
                            }
                            return;
                        }
                        Err(err) => dialog.error = Some(err.to_string()),
                    }
                }
            }
            Some(TrimAction::Revert) => match self.set_clip_trim(&dialog.id, None) {
                Ok(()) => {
                    dialog.stop_preview(&self.audio.player);
                    dialog.preview_after_edit = false;
                    dialog.saved = None;
                    dialog.draft = dialog
                        .info
                        .as_ref()
                        .map(|info| ClipTrim::full(info.duration_ms));
                    if dialog.info.is_some() {
                        dialog.error = None;
                    }
                    self.message("Full clip restored");
                }
                Err(err) => dialog.error = Some(err.to_string()),
            },
            Some(TrimAction::Preview) => {
                dialog.preview_after_edit = false;
                let result = self
                    .store
                    .clip_path(&self.clips[index].file)
                    .and_then(|path| {
                        self.request_play(
                            Source::File(path),
                            Some(dialog.id.clone()),
                            dialog.name.clone(),
                            true,
                            dialog.draft,
                        )
                    });
                match result {
                    Ok(generation) => dialog.preview_generation = Some(generation),
                    Err(err) => dialog.error = Some(err.to_string()),
                }
            }
            Some(TrimAction::StopPreview) => {
                dialog.stop_preview(&self.audio.player);
                dialog.preview_after_edit = false;
            }
            None => {}
        }
        self.trim_dialog = Some(dialog);
    }

    fn set_clip_trim(&mut self, id: &str, trim: Option<ClipTrim>) -> Result<()> {
        let mut clips = self.clips.clone();
        let clip = clips
            .iter_mut()
            .find(|clip| clip.id == id)
            .context("This clip was removed.")?;
        if clip.trim == trim {
            return Ok(());
        }
        clip.trim = trim;
        if let Some(key) = clip.hotkey.clone()
            && let Some(manager) = &mut self.shortcuts
            && manager.is_registered(id)
        {
            let action = clip_action(&self.store, clip)?;
            manager.set(id, &key, action, || self.store.save(LIBRARY_FILE, &clips))?;
        } else {
            self.store.save(LIBRARY_FILE, &clips)?;
        }
        self.clips = clips;
        self.stop_clip(id, false);
        self.stop_clip(id, true);
        Ok(())
    }
}

fn range_editor(
    ui: &mut Ui,
    info: &ClipAudioInfo,
    draft: &mut ClipTrim,
    preview_elapsed: Option<u32>,
) -> bool {
    let duration = info.duration_ms;
    let p = design::palette(ui);
    let previous = *draft;
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), 92.0), Sense::CLICK | Sense::DRAG);
    let wave = rect.shrink2(vec2(14.0, 12.0));
    let x_for = |time: u32| wave.left() + wave.width() * time as f32 / duration as f32;
    let time_at = |x: f32| {
        (duration as f32 * ((x - wave.left()) / wave.width()).clamp(0.0, 1.0)).round() as u32
    };
    let holding =
        response.is_pointer_button_down_on() && ui.input(|input| input.pointer.primary_down());
    let handle = response.id.with("trim-boundary");
    let move_start = ui.data_mut(|data| {
        if holding {
            data.get_temp::<bool>(handle)
        } else {
            data.remove_temp::<bool>(handle)
        }
    });
    if (holding
        || response.clicked_by(PointerButton::Primary)
        || response.drag_stopped_by(PointerButton::Primary))
        && let Some(position) = response.interact_pointer_pos()
    {
        let move_start = move_start.unwrap_or_else(|| {
            (position.x - x_for(draft.start_ms)).abs() <= (position.x - x_for(draft.end_ms)).abs()
        });
        if holding {
            ui.data_mut(|data| data.insert_temp(handle, move_start));
        }
        let time = time_at(position.x);
        if move_start {
            draft.start_ms = time.min(draft.end_ms - 1);
        } else {
            draft.end_ms = time.max(draft.start_ms + 1);
        }
    }
    response
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text("Click or drag to set a boundary");
    ui.horizontal(|ui| {
        let time_label = |ui: &mut Ui, time| {
            ui.label(
                RichText::new(design::seconds_label(time))
                    .color(p.muted_foreground)
                    .size(11.0),
            );
        };
        time_label(ui, 0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            time_label(ui, duration);
        });
    });
    let typing_or_dragging = ui
        .horizontal(|ui| {
            let start = time_field(ui, "Start", &mut draft.start_ms, 0..=draft.end_ms - 1);
            ui.add_space(8.0);
            let end = time_field(ui, "End", &mut draft.end_ms, draft.start_ms + 1..=duration);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} selected",
                        design::seconds_label(draft.duration_ms())
                    ))
                    .strong()
                    .color(p.primary),
                );
            });
            start || end
        })
        .inner;

    let selection = Rect::from_min_max(
        pos2(x_for(draft.start_ms), wave.top()),
        pos2(x_for(draft.end_ms), wave.bottom()),
    );
    let painter = ui.painter();
    painter.rect(
        rect,
        9,
        p.muted,
        Stroke::new(1.0, p.border),
        StrokeKind::Inside,
    );
    painter.rect_filled(selection, 2, p.accent);
    let selected = painter.with_clip_rect(selection);
    let bin_width = wave.width() / info.peaks.len() as f32;
    for (bin, peak) in info.peaks.iter().enumerate() {
        let x = wave.left() + (bin as f32 + 0.5) * bin_width;
        let height = (peak.clamp(0.0, 1.0).sqrt() * wave.height() * 0.82).max(1.5);
        let bar = Rect::from_center_size(
            pos2(x, wave.center().y),
            vec2((bin_width - 1.0).max(1.0), height),
        );
        painter.rect_filled(bar, 1, p.muted_foreground.gamma_multiply(0.55));
        if bar.intersects(selection) {
            selected.rect_filled(bar, 1, p.primary);
        }
    }
    for x in [selection.left(), selection.right()] {
        painter.vline(x, wave.y_range(), Stroke::new(1.5, p.primary));
        painter.rect_filled(
            Rect::from_center_size(pos2(x, wave.center().y), vec2(9.0, 24.0)),
            4,
            p.primary,
        );
        painter.vline(
            x,
            (wave.center().y - 4.0)..=(wave.center().y + 4.0),
            Stroke::new(1.0, p.primary_foreground),
        );
    }
    if *draft == previous
        && let Some(elapsed) = preview_elapsed
    {
        let x = x_for(draft.start_ms + elapsed.min(draft.duration_ms()));
        painter.vline(
            x,
            (wave.top() - 4.0)..=(wave.bottom() + 4.0),
            Stroke::new(1.5, p.foreground),
        );
    }
    holding || typing_or_dragging
}

fn time_field(
    ui: &mut Ui,
    label: &str,
    time_ms: &mut u32,
    range: std::ops::RangeInclusive<u32>,
) -> bool {
    let mut seconds = f64::from(*time_ms) / 1000.0;
    let label = ui.label(label);
    let response = ui
        .add(
            DragValue::new(&mut seconds)
                .min_size(vec2(83.0, 28.0))
                .speed(0.005)
                .range(f64::from(*range.start()) / 1000.0..=f64::from(*range.end()) / 1000.0)
                .fixed_decimals(3)
                .suffix(" s"),
        )
        .labelled_by(label.id);
    if response.changed() {
        *time_ms = ((seconds * 1000.0).round() as u32).clamp(*range.start(), *range.end());
    }
    response.has_focus() || response.is_pointer_button_down_on()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adjusting_a_waveform_boundary_commits_its_release_position() {
        let context = egui::Context::default();
        design::install(&context, crate::storage::Appearance::Light);
        let info = ClipAudioInfo {
            duration_ms: 10_000,
            peaks: vec![0.5; 40],
        };
        let mut draft = ClipTrim::full(info.duration_ms);
        let mut wave = Rect::NOTHING;
        let mut draw = |events| {
            let mut editing = false;
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(550.0, 230.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    wave = Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), 92.0))
                        .shrink2(vec2(14.0, 12.0));
                    editing = range_editor(ui, &info, &mut draft, None);
                },
            );
            output.textures_delta.clear();
            (editing, draft, wave)
        };
        let (_, _, wave) = draw(vec![]);
        let at = |fraction| pos2(wave.left() + wave.width() * fraction, wave.center().y);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let (editing, _, _) = draw(vec![
            egui::Event::PointerMoved(at(0.1)),
            button(at(0.1), true),
        ]);
        assert!(editing);
        let (editing, draft, _) = draw(vec![egui::Event::PointerMoved(at(0.4))]);
        assert!(editing);
        assert_eq!(draft.start_ms, 4000);
        let (editing, draft, _) = draw(vec![
            egui::Event::PointerMoved(at(0.6)),
            button(at(0.6), false),
        ]);
        assert!(!editing);
        assert_eq!(
            draft,
            ClipTrim {
                start_ms: 6000,
                end_ms: 10_000,
            }
        );
    }
}
