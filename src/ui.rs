mod design;
mod icons;
mod library;
mod settings;
mod shortcut_dialog;
mod trim;
mod updates;

use crate::{
    audio::{self, ClipProgress, Devices, Engine, Route, Source},
    folder::{FolderSync, Snapshot},
    native_caption::Caption,
    shortcuts::{self, Action, Shortcuts},
    storage::{AUDIO_EXTENSIONS, Clip, ClipTrim, LIBRARY_FILE, Settings, Store, file_key},
    tray::{self, Tray},
};
use anyhow::{Context, Result, bail};
use eframe::egui::{self, Align, Align2, Frame, Layout, Margin, RichText, Stroke, Ui};
use icons::Icon;
use shadcn_egui::components::button::{ButtonSize, ButtonVariant};
use shortcut_dialog::{ShortcutDialog, ShortcutTarget};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{atomic::Ordering::Relaxed, mpsc},
    thread,
    time::{Duration, Instant},
};

const STOP_ID: &str = "stop";
const SETTINGS_FILE: &str = "settings.json";

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Library,
    Settings(SettingsTab),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Audio,
    Mix,
    Shortcuts,
    Appearance,
    Updates,
    Diagnostics,
}

struct Notice {
    text: String,
    error: bool,
    expires: Instant,
}

struct Playback {
    id: Option<String>,
    title: String,
    generation: u64,
    pending_until: Option<Instant>,
    timeline: Option<PlaybackTimeline>,
    seek_revision: u32,
}

struct PlaybackTimeline {
    elapsed_ms: f64,
    duration_ms: u32,
    last_frame: Instant,
    finished_at: Option<Instant>,
    seek_revision: u32,
}

impl PlaybackTimeline {
    fn new(duration_ms: u32, now: Instant) -> Self {
        Self {
            elapsed_ms: 0.0,
            duration_ms,
            last_frame: now,
            finished_at: None,
            seek_revision: 0,
        }
    }

    fn advance(&mut self, rendered_ms: u32, now: Instant) {
        let target = f64::from(rendered_ms.min(self.duration_ms));
        let delta = now.saturating_duration_since(self.last_frame).as_secs_f64() * 1000.0;
        // The device reports a buffer at a time. Catch up between callbacks while
        // keeping the indicator monotonic and bounded by rendered audio.
        let catch_up = ((target - self.elapsed_ms - 16.0) / 48.0).clamp(0.0, 1.0);
        let next = (self.elapsed_ms + delta * (1.0 + catch_up)).min(target);
        self.elapsed_ms = self.elapsed_ms.max(next);
        self.last_frame = now;
    }

    fn progress(&self) -> ClipProgress {
        ClipProgress {
            elapsed_ms: self.elapsed_ms as u32,
            duration_ms: self.duration_ms,
            seek_revision: self.seek_revision,
        }
    }

    fn sync(&mut self, progress: ClipProgress, now: Instant) {
        if self.seek_revision != progress.seek_revision {
            self.seek_revision = progress.seek_revision;
            self.elapsed_ms = f64::from(progress.elapsed_ms);
            self.last_frame = now;
            self.finished_at = None;
        } else {
            self.advance(progress.elapsed_ms, now);
        }
    }
}

struct ImportBatch {
    store: Store,
    clips: Vec<Clip>,
    failures: Vec<String>,
}

impl Drop for ImportBatch {
    fn drop(&mut self) {
        for clip in &self.clips {
            let _ = self.store.remove_clip_file(clip);
        }
    }
}

pub struct App {
    store: Store,
    settings: Settings,
    clips: Vec<Clip>,
    visible_clips: Vec<usize>,
    folder: FolderSync,
    missing: HashSet<String>,
    filter: String,
    favorites: bool,
    page: Page,
    animate: bool,
    logo: egui::TextureHandle,
    caption: Caption,
    tray: Tray,
    hide_on_first_frame: bool,
    quitting: bool,
    audio: Engine,
    devices: Devices,
    route: Route,
    route_checked: bool,
    refreshing: bool,
    shortcuts: Option<Shortcuts>,
    shortcut_events: mpsc::Receiver<shortcuts::Event>,
    shortcut_dialog: Option<ShortcutDialog>,
    clip_details: Option<library::ClipDetails>,
    trim_dialog: Option<trim::TrimDialog>,
    trim_request: u64,
    import_updates: mpsc::Receiver<ImportBatch>,
    import_send: mpsc::Sender<ImportBatch>,
    importing: bool,
    notice: Option<Notice>,
    on_air: Option<Playback>,
    preview: Option<Playback>,
    settings_save: Option<Instant>,
    updates: updates::Updates,
}

impl App {
    pub fn new(
        context: &eframe::CreationContext<'_>,
        store: Store,
        background: bool,
    ) -> Result<Self> {
        let ctx = &context.egui_ctx;
        let mut settings: Settings = store.read(SETTINGS_FILE)?;
        let clips = store.read_library()?;
        let wake = ctx.clone();
        let folder = FolderSync::start(store.clone(), &clips, move || wake.request_repaint());
        let wake = ctx.clone();
        let audio = Engine::start(settings.clone(), move || wake.request_repaint());
        let (shortcut_send, shortcut_events) = mpsc::channel();
        let (shortcuts, shortcut_error) =
            match Shortcuts::new(audio.player.clone(), shortcut_send, ctx.clone()) {
                Ok(manager) => (Some(manager), None),
                Err(err) => (None, Some(format!("{err:#}"))),
            };
        let (import_send, import_updates) = mpsc::channel();
        let animate = design::install(ctx, settings.appearance);
        let mut tray = Tray::new(ctx);
        let tray_error = tray.enable(settings.close_to_tray || background).err();
        if tray_error.is_some() {
            settings.close_to_tray = false;
            if background {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            }
        }
        let brand = crate::window_icon();
        let logo = ctx.load_texture(
            "meme-soundboard-logo",
            egui::ColorImage::from_rgba_unmultiplied(
                [brand.width as usize, brand.height as usize],
                &brand.rgba,
            ),
            egui::TextureOptions::LINEAR,
        );
        let mut app = Self {
            store,
            settings,
            clips,
            visible_clips: Vec::new(),
            folder,
            missing: HashSet::new(),
            filter: String::new(),
            favorites: false,
            page: Page::Library,
            animate,
            logo,
            caption: Caption::new(context),
            tray,
            hide_on_first_frame: background && tray_error.is_none(),
            quitting: false,
            audio,
            devices: Devices::default(),
            route: Route::default(),
            route_checked: false,
            refreshing: false,
            shortcuts,
            shortcut_events,
            shortcut_dialog: None,
            clip_details: None,
            trim_dialog: None,
            trim_request: 0,
            import_updates,
            import_send,
            importing: false,
            notice: None,
            on_air: None,
            preview: None,
            settings_save: None,
            updates: updates::Updates::default(),
        };
        app.update_filter();
        app.register_saved_shortcuts();
        if let Some(error) = shortcut_error {
            app.error(error);
        }
        if let Some(error) = tray_error {
            app.error(format!("Couldn't create the Windows tray icon. {error}"));
        }
        Ok(app)
    }

    fn register_saved_shortcuts(&mut self) {
        let Some(manager) = &mut self.shortcuts else {
            return;
        };
        if let Err(err) = manager.set(STOP_ID, &self.settings.stop_key, Action::Stop, || Ok(())) {
            self.store.log(&format!("Stop shortcut: {err:#}"));
        }
        for clip in &self.clips {
            let Some(key) = &clip.hotkey else {
                continue;
            };
            let result = clip_action(&self.store, clip)
                .and_then(|action| manager.set(&clip.id, key, action, || Ok(())));
            if let Err(err) = result {
                self.store
                    .log(&format!("Shortcut for {}: {err:#}", clip.name));
            }
        }
    }

    fn receive_updates(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.audio.events.try_recv() {
            match event {
                audio::Event::Devices(devices) => self.devices = devices,
                audio::Event::Route(route) => {
                    self.route_checked = true;
                    self.route = route;
                }
                audio::Event::Refreshed => {
                    self.refreshing = false;
                    self.message("Audio devices refreshed");
                }
                audio::Event::PlaybackQueued {
                    preview,
                    generation,
                } => {
                    if let Some(play) = self.playback_slot(preview).as_mut()
                        && play.generation == generation
                        && play.pending_until.is_some()
                    {
                        play.pending_until = Some(Instant::now() + Duration::from_millis(400));
                    }
                }
                audio::Event::ClipInspected { request, result } => {
                    if let Some(dialog) = &mut self.trim_dialog {
                        dialog.receive(request, result);
                    }
                }
                audio::Event::Error(error) => self.error(error),
            }
        }
        while let Ok(event) = self.shortcut_events.try_recv() {
            match event {
                shortcuts::Event::Playing {
                    id,
                    title,
                    generation,
                } => {
                    self.play_requested(Some(id), title, false, generation);
                }
                shortcuts::Event::Stopped => {}
                shortcuts::Event::Captured(key) => {
                    if let Some(dialog) = &mut self.shortcut_dialog
                        && dialog.recording
                    {
                        dialog.record(key);
                    }
                }
                shortcuts::Event::Error(error) => self.error(error),
            }
        }
        while let Ok(batch) = self.import_updates.try_recv() {
            self.finish_import(batch);
        }
        let mut latest = None;
        let mut failures = Vec::new();
        for mut snapshot in self.folder.events.try_iter() {
            failures.append(&mut snapshot.failures);
            latest = Some(snapshot);
        }
        if !self.importing
            && let Some(mut snapshot) = latest
        {
            snapshot.failures = failures;
            self.sync_folder(snapshot);
        }

        let now = Instant::now();
        if let Some(due) = self.settings_save {
            if now >= due {
                self.settings_save = None;
                if let Err(err) = self.store.save(SETTINGS_FILE, &self.settings) {
                    self.error(err.to_string());
                }
            } else {
                ctx.request_repaint_after(due - now);
            }
        }
        if let Some(notice) = &self.notice {
            if now >= notice.expires {
                self.notice = None;
            } else {
                ctx.request_repaint_after(notice.expires - now);
            }
        }
        if let Some(shortcuts) = &self.shortcuts {
            let recording = self
                .shortcut_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.recording);
            shortcuts.capture(
                recording
                    && ctx.input(|input| {
                        let viewport = input.viewport();
                        viewport.visible() != Some(false)
                            && viewport.focused.unwrap_or(input.focused)
                    }),
            );
        }
    }

    fn sync_folder(&mut self, snapshot: Snapshot) {
        self.missing = self
            .clips
            .iter()
            .filter(|clip| !snapshot.present.contains(&file_key(&clip.file)))
            .map(|clip| clip.id.clone())
            .collect();
        let existing: HashSet<_> = self.clips.iter().map(|clip| file_key(&clip.file)).collect();
        let mut new = snapshot
            .new
            .into_iter()
            .filter(|clip| !existing.contains(&file_key(&clip.file)))
            .collect::<Vec<_>>();
        if !new.is_empty() {
            let added = new.len();
            self.clips.append(&mut new);
            if let Err(err) = self.store.save(LIBRARY_FILE, &self.clips) {
                self.clips.truncate(self.clips.len() - added);
                self.error(format!("Couldn't add files from the clips folder. {err:#}"));
            } else {
                self.folder.set_known(&self.clips);
                self.update_filter();
                self.message(format!("Added {} from the clips folder", clip_count(added)));
            }
        }
        if !snapshot.failures.is_empty() {
            for failure in &snapshot.failures {
                self.store.log(&format!("Clips folder: {failure}"));
            }
            self.error(format!(
                "Couldn't add {} from the clips folder. See Diagnostics.",
                clip_count(snapshot.failures.len())
            ));
        }
    }

    fn update_filter(&mut self) {
        let filter = self.filter.trim().to_lowercase();
        self.visible_clips.clear();
        for (index, clip) in self.clips.iter().enumerate() {
            if (!self.favorites || clip.favorite)
                && (filter.is_empty() || clip.name.to_lowercase().contains(&filter))
            {
                self.visible_clips.push(index);
            }
        }
    }

    fn import_files(&mut self, ctx: &egui::Context) {
        if self.importing {
            return;
        }
        let Some(files) = rfd::FileDialog::new()
            .add_filter("Audio clips", AUDIO_EXTENSIONS)
            .set_title("Import audio clips")
            .pick_files()
        else {
            return;
        };
        self.start_import(files, ctx);
    }

    fn start_import(&mut self, files: Vec<PathBuf>, ctx: &egui::Context) {
        self.importing = true;
        let store = self.store.clone();
        let send = self.import_send.clone();
        let wake = ctx.clone();
        thread::spawn(move || {
            let mut batch = ImportBatch {
                store,
                clips: Vec::new(),
                failures: Vec::new(),
            };
            for file in files {
                match batch.store.import(&file, audio::validate) {
                    Ok(clip) => batch.clips.push(clip),
                    Err(err) => batch.failures.push(format!("{}: {err:#}", file.display())),
                }
            }
            if send.send(batch).is_ok() {
                wake.request_repaint();
            }
        });
    }

    fn finish_import(&mut self, mut batch: ImportBatch) {
        self.importing = false;
        self.folder.refresh();
        let count = batch.clips.len();
        if count > 0 {
            let mut updated = self.clips.clone();
            updated.extend(batch.clips.iter().cloned());
            if let Err(err) = self.store.save(LIBRARY_FILE, &updated) {
                self.error(format!("Import failed. {err:#}"));
                return;
            }
            batch.clips.clear();
            self.clips = updated;
            self.folder.set_known(&self.clips);
            self.update_filter();
        }
        if batch.failures.is_empty() {
            if count > 0 {
                self.message(format!("Imported {}", clip_count(count)));
            }
        } else {
            for failure in &batch.failures {
                self.store.log(failure);
            }
            let failed = clip_count(batch.failures.len());
            self.error(format!(
                "Imported {}. Couldn't open {failed}. See the diagnostic log in Settings.",
                clip_count(count)
            ));
        }
    }

    fn message(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice {
            text: text.into(),
            error: false,
            expires: Instant::now() + Duration::from_secs(4),
        });
    }

    fn error(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.store.log(&text);
        self.notice = Some(Notice {
            text,
            error: true,
            expires: Instant::now() + Duration::from_secs(10),
        });
    }

    fn navigate(&mut self, page: Page) {
        self.page = page;
    }

    fn settings_changed(&mut self, ctx: &egui::Context) {
        self.audio.settings(&self.settings);
        self.queue_settings_save(ctx);
    }

    fn queue_settings_save(&mut self, ctx: &egui::Context) {
        let debounce = Duration::from_millis(280);
        self.settings_save = Some(Instant::now() + debounce);
        ctx.request_repaint_after(debounce);
    }

    fn play_clip(&mut self, index: usize, preview: bool) {
        let clip = &self.clips[index];
        let id = clip.id.clone();
        let title = clip.name.clone();
        let trim = clip.trim;
        let result = self
            .store
            .clip_path(&clip.file)
            .and_then(|path| self.request_play(Source::File(path), Some(id), title, preview, trim));
        if let Err(err) = result {
            self.error(err.to_string());
        }
    }

    fn request_play(
        &mut self,
        source: Source,
        id: Option<String>,
        title: String,
        preview: bool,
        trim: Option<ClipTrim>,
    ) -> Result<u64> {
        let generation = self.audio.player.play(source, preview, trim)?;
        self.play_requested(id, title, preview, generation);
        Ok(generation)
    }

    fn play_requested(
        &mut self,
        id: Option<String>,
        title: String,
        preview: bool,
        generation: u64,
    ) {
        if !self.audio.player.is_current(preview, generation) {
            return;
        }
        *self.playback_slot(preview) = Some(Playback {
            id,
            title,
            generation,
            pending_until: Some(Instant::now() + Duration::from_secs(2)),
            timeline: None,
            seek_revision: 0,
        });
    }

    fn playback_slot(&mut self, preview: bool) -> &mut Option<Playback> {
        if preview {
            &mut self.preview
        } else {
            &mut self.on_air
        }
    }

    fn current_playback(&self, preview: bool) -> Option<&Playback> {
        let play = if preview {
            self.preview.as_ref()
        } else {
            self.on_air.as_ref()
        }?;
        self.audio
            .player
            .is_current(preview, play.generation)
            .then_some(play)
    }

    fn playing(&self, preview: bool) -> Option<(&Playback, ClipProgress)> {
        let play = self.current_playback(preview)?;
        Some((play, play.timeline.as_ref()?.progress()))
    }

    fn update_playback(&mut self) {
        let now = Instant::now();
        for (preview, slot) in [(false, &mut self.on_air), (true, &mut self.preview)] {
            let Some(play) = slot.as_mut() else {
                continue;
            };
            if !self.audio.player.is_current(preview, play.generation) {
                *slot = None;
            } else if let Some(progress) = self.audio.meter.progress(preview, play.generation) {
                play.pending_until = None;
                play.timeline
                    .get_or_insert_with(|| PlaybackTimeline::new(progress.duration_ms, now))
                    .sync(progress, now);
            } else if self.audio.meter.completed(preview, play.generation) {
                if let Some(timeline) = &mut play.timeline {
                    if play.seek_revision != timeline.seek_revision {
                        // A seek into the final device buffer can complete before it is observed.
                        timeline.elapsed_ms = f64::from(timeline.duration_ms);
                    }
                    timeline.advance(timeline.duration_ms, now);
                    if timeline.progress().elapsed_ms == timeline.duration_ms {
                        let finished = *timeline.finished_at.get_or_insert(now);
                        if now.saturating_duration_since(finished) >= Duration::from_millis(50) {
                            *slot = None;
                        }
                    }
                } else {
                    *slot = None;
                }
            } else if play.pending_until.is_none_or(|until| now >= until) {
                *slot = None;
            }
        }
    }

    fn playback_active(&self, id: &str, preview: bool) -> bool {
        self.current_playback(preview)
            .is_some_and(|play| play.id.as_deref() == Some(id))
    }

    fn seek_playback(&mut self, preview: bool, elapsed_ms: u32) {
        let Some((play, progress)) = self.playing(preview) else {
            return;
        };
        let revision = play.seek_revision % audio::MAX_SEEK_REVISION + 1;
        let seek = audio::Seek {
            generation: play.generation,
            preview,
            elapsed_ms: elapsed_ms.min(progress.duration_ms),
            revision,
        };
        if let Err(err) = self.audio.seek(seek) {
            self.error(err.to_string());
        } else if let Some(play) = self.playback_slot(preview) {
            play.seek_revision = revision;
        }
    }

    fn stop_clip(&mut self, id: &str, preview: bool) {
        if let Some(play) = self.current_playback(preview)
            && play.id.as_deref() == Some(id)
        {
            self.audio.player.stop_clip(preview, play.generation);
            *self.playback_slot(preview) = None;
        }
    }

    fn clear_playback(&mut self) {
        self.preview = None;
        self.on_air = None;
    }

    fn stop(&mut self) {
        self.audio.player.stop();
        self.clear_playback();
    }

    fn open_shortcut(&mut self, target: ShortcutTarget, ctx: &egui::Context) {
        let Some(manager) = &self.shortcuts else {
            self.error("Windows shortcuts are unavailable. Restart the soundboard and try again.");
            return;
        };
        let (name, key) = match &target {
            ShortcutTarget::Stop => ("Stop playback".into(), self.settings.stop_key.clone()),
            ShortcutTarget::Clip(id) => {
                let Some(clip) = self.clips.iter().find(|clip| clip.id == *id) else {
                    return;
                };
                (clip.name.clone(), clip.hotkey.clone().unwrap_or_default())
            }
        };
        self.shortcut_dialog = Some(ShortcutDialog::new(target, name, key));
        manager.capture(ctx.input(|input| input.focused));
    }

    fn save_shortcut(&mut self, target: &ShortcutTarget, key: &str) -> Result<()> {
        let manager = self
            .shortcuts
            .as_mut()
            .context("Windows shortcuts are unavailable.")?;
        match target {
            ShortcutTarget::Stop => {
                let mut settings = self.settings.clone();
                settings.stop_key = key.to_owned();
                manager.set(STOP_ID, key, Action::Stop, || {
                    self.store.save(SETTINGS_FILE, &settings)
                })?;
                self.settings = settings;
                self.settings_save = None;
            }
            ShortcutTarget::Clip(id) => {
                let mut clips = self.clips.clone();
                let clip = clips
                    .iter_mut()
                    .find(|clip| clip.id == *id)
                    .context("This clip was removed.")?;
                clip.hotkey = (!key.is_empty()).then(|| key.to_owned());
                let action = clip_action(&self.store, clip)?;
                manager.set(id, key, action, || self.store.save(LIBRARY_FILE, &clips))?;
                self.clips = clips;
            }
        }
        Ok(())
    }

    fn shortcut_registered(&self, id: &str) -> bool {
        self.shortcuts
            .as_ref()
            .is_some_and(|manager| manager.is_registered(id))
    }

    fn rename_clip(&mut self, id: &str, name: &str) -> Result<()> {
        let index = self
            .clips
            .iter()
            .position(|clip| clip.id == id)
            .context("This clip was removed.")?;
        let old = &self.clips[index];
        let renamed = old.renamed(name)?;
        let filename = file_key(&renamed.file);
        if self
            .clips
            .iter()
            .any(|clip| clip.id != id && file_key(&clip.file) == filename)
        {
            bail!("A clip with that filename is already in Library.");
        }
        let mut clips = self.clips.clone();
        clips[index] = renamed;
        let clip = &clips[index];
        let store = &self.store;
        store.rename_clip_file(old, clip, || {
            let save = || store.save(LIBRARY_FILE, &clips);
            if let (Some(manager), Some(key)) = (&mut self.shortcuts, &clip.hotkey)
                && manager.is_registered(id)
            {
                manager.set(id, key, clip_action(store, clip)?, save)
            } else {
                save()
            }
        })?;
        for playback in [&mut self.on_air, &mut self.preview].into_iter().flatten() {
            if playback.id.as_deref() == Some(id) {
                playback.title.clone_from(&clip.name);
            }
        }
        self.clips = clips;
        self.library_changed();
        Ok(())
    }

    fn remove_clip(&mut self, id: &str) -> Result<()> {
        let clip = self
            .clips
            .iter()
            .find(|clip| clip.id == id)
            .context("This clip was removed.")?;
        let updated: Vec<_> = self
            .clips
            .iter()
            .filter(|item| item.id != id)
            .cloned()
            .collect();
        let store = &self.store;
        store.delete_clip(clip, || {
            let save = || store.save(LIBRARY_FILE, &updated);
            if let Some(manager) = &mut self.shortcuts {
                manager.set(id, "", Action::Stop, save)
            } else {
                save()
            }
        })?;
        self.clips = updated;
        self.stop_clip(id, false);
        self.stop_clip(id, true);
        self.missing.remove(id);
        self.library_changed();
        Ok(())
    }

    fn library_changed(&mut self) {
        self.folder.set_known(&self.clips);
        self.folder.refresh();
        self.update_filter();
    }

    fn handle_tray(&mut self, ctx: &egui::Context) {
        use egui::ViewportCommand as Window;

        if crate::update::shutdown_requested() {
            self.quitting = true;
            ctx.send_viewport_cmd(Window::Close);
            return;
        }

        let mut show = crate::update::show_requested();
        while let Ok(action) = self.tray.actions.try_recv() {
            match action {
                tray::Action::Show => show = true,
                tray::Action::Stop => self.stop(),
                tray::Action::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(Window::Close);
                }
            }
        }
        if show && !self.quitting {
            self.hide_on_first_frame = false;
            ctx.send_viewport_cmd(Window::Visible(true));
            ctx.send_viewport_cmd(Window::Minimized(false));
            ctx.send_viewport_cmd(Window::Focus);
        }
        if ctx.input(|input| input.viewport().close_requested()) && self.hide_to_tray(ctx) {
            ctx.send_viewport_cmd(Window::CancelClose);
        }
    }

    fn hide_to_tray(&self, ctx: &egui::Context) -> bool {
        if self.quitting || !self.settings.close_to_tray || !self.tray.enabled() {
            return false;
        }
        if let Some(shortcuts) = &self.shortcuts {
            shortcuts.capture(false);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        true
    }

    fn navigation(&mut self, ui: &mut Ui) {
        let p = design::palette(ui);
        egui::Panel::left("navigation")
            .resizable(false)
            .exact_size(68.0)
            .frame(
                Frame::new()
                    .fill(p.card)
                    .inner_margin(Margin::symmetric(8, 16)),
            )
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add(
                        egui::Image::new((self.logo.id(), egui::vec2(39.0, 39.0)))
                            .alt_text("Meme Soundboard"),
                    )
                    .on_hover_text("Meme Soundboard");
                    ui.add_space(18.0);
                    for (page, label, icon) in [
                        (Page::Library, "Library", Icon::LIBRARY),
                        (
                            Page::Settings(SettingsTab::Audio),
                            "Audio devices",
                            Icon::AUDIO_DEVICES,
                        ),
                        (Page::Settings(SettingsTab::Mix), "Mix", Icon::MIX),
                        (
                            Page::Settings(SettingsTab::Shortcuts),
                            "Shortcuts",
                            Icon::KEYBOARD,
                        ),
                        (
                            Page::Settings(SettingsTab::Appearance),
                            "Appearance",
                            Icon::APPEARANCE,
                        ),
                        (
                            Page::Settings(SettingsTab::Updates),
                            "Updates",
                            Icon::DOWNLOAD,
                        ),
                        (
                            Page::Settings(SettingsTab::Diagnostics),
                            "Diagnostics",
                            Icon::DOCUMENT,
                        ),
                    ] {
                        let variant = if page == self.page {
                            ButtonVariant::Secondary
                        } else {
                            ButtonVariant::Ghost
                        };
                        if ui
                            .add(
                                design::toolbar_button(label, icon, variant).size(ButtonSize::Icon),
                            )
                            .on_hover_text(label)
                            .clicked()
                        {
                            self.navigate(page);
                        }
                        if page == Page::Library {
                            let center = ui.max_rect().center().x;
                            let y = ui.next_widget_position().y + 6.0;
                            ui.painter().hline(
                                center - 13.0..=center + 13.0,
                                y,
                                Stroke::new(1.0, p.border),
                            );
                            ui.add_space(14.0);
                        }
                    }
                });
            });
    }

    fn transport(&mut self, ui: &mut Ui) {
        const MIC_WIDTH: f32 = 146.0;
        const STOP_WIDTH: f32 = 76.0;
        const GAP: f32 = 16.0;
        const HEIGHT: f32 = 42.0;

        let p = design::palette(ui);
        egui::Panel::bottom("transport")
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(p.card)
                    .inner_margin(Margin::symmetric(design::GUTTER, 10)),
            )
            .show(ui, |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), HEIGHT),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        let playback_width =
                            ui.available_width() - MIC_WIDTH - STOP_WIDTH - 2.0 * GAP;
                        ui.spacing_mut().item_spacing.x = GAP;
                        let left_to_right = Layout::left_to_right(Align::Center);
                        let mic = ui.allocate_ui_with_layout(
                            egui::vec2(MIC_WIDTH, HEIGHT),
                            left_to_right,
                            |ui| {
                                ui.set_width(MIC_WIDTH);
                                ui.spacing_mut().item_spacing.x = 10.0;
                                self.mic_control(ui);
                            },
                        );
                        let x = mic.response.rect.right() + GAP * 0.5;
                        let y = mic.response.rect.center().y;
                        ui.painter()
                            .vline(x, y - 14.0..=y + 14.0, Stroke::new(1.0, p.border));
                        ui.allocate_ui_with_layout(
                            egui::vec2(playback_width, HEIGHT),
                            left_to_right,
                            |ui| {
                                ui.set_width(playback_width);
                                ui.spacing_mut().item_spacing.x = 10.0;
                                self.transport_playback(ui);
                            },
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let tooltip = if self.settings.stop_key.is_empty() {
                                "Stop clip playback".into()
                            } else {
                                format!("Stop playback ({})", self.settings.stop_key)
                            };
                            if ui
                                .add_enabled(
                                    self.current_playback(false).is_some()
                                        || self.current_playback(true).is_some(),
                                    design::toolbar_button(
                                        "Stop",
                                        Icon::STOP,
                                        ButtonVariant::Outline,
                                    ),
                                )
                                .on_hover_text(tooltip)
                                .clicked()
                            {
                                self.stop();
                                ui.ctx().request_repaint();
                            }
                        });
                    },
                );
            });
    }

    fn mic_control(&mut self, ui: &mut Ui) {
        let muted = self.settings.mic_muted;
        let (icon, action) = if muted {
            (Icon::MIC_OFF, "Unmute microphone")
        } else {
            (Icon::MIC, "Mute microphone")
        };
        if ui
            .add(
                design::toolbar_button(action, icon, ButtonVariant::Outline)
                    .size(design::ROUND_CONTROL),
            )
            .on_hover_text(action)
            .clicked()
        {
            self.settings.mic_muted = !muted;
            self.settings_changed(ui.ctx());
        }
        let muted = self.settings.mic_muted;
        let has_mic = self.route.microphone.is_some();
        let label = if muted {
            "Mic muted"
        } else if has_mic {
            "Mic"
        } else {
            "Mic offline"
        };
        let live = has_mic && !muted;
        let color = if live && self.audio.meter.speaking.load(Relaxed) {
            design::mic_color(ui)
        } else {
            design::palette(ui).muted_foreground
        };
        let level = if live {
            f32::from_bits(self.audio.meter.level.load(Relaxed))
        } else {
            0.0
        };
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.label(RichText::new(label).size(12.0).color(color));
            design::mic_meter(ui, level, self.animate);
        });
    }

    fn transport_playback(&mut self, ui: &mut Ui) {
        let p = design::palette(ui);
        let on_air = self.playing(false);
        if let Some((play, progress)) = on_air.or_else(|| self.playing(true)) {
            let icon = if on_air.is_some() {
                Icon::WAVEFORM
            } else {
                Icon::HEADPHONES
            };
            let (badge, _) = ui.allocate_exact_size(egui::Vec2::splat(32.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(badge.center(), 16.0, p.secondary);
            icons::paint(
                ui,
                egui::Rect::from_center_size(badge.center(), egui::Vec2::splat(18.0)),
                icon,
                p.secondary_foreground,
            );
            let width = ui.available_width();
            let seek = ui
                .allocate_ui_with_layout(
                    egui::vec2(width, 32.0),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = 3.0;
                        ui.add(
                            egui::Label::new(RichText::new(&play.title).size(13.0).strong())
                                .truncate(),
                        )
                        .on_hover_text(&play.title);
                        design::timeline(ui, progress, width)
                    },
                )
                .inner;
            if let Some(elapsed_ms) = seek {
                self.seek_playback(on_air.is_none(), elapsed_ms);
            }
        } else if self.route.cable.is_some() {
            design::icon(ui, Icon::AUDIO_DEVICES, 16.0, design::mic_color(ui));
            design::muted(ui, "Voice output ready");
        } else if self.settings.local_playback_only && self.route.headphones.is_some() {
            design::icon(ui, Icon::HEADPHONES, 16.0, p.muted_foreground);
            design::muted(ui, "Local playback");
        } else if self.route_checked
            && ui
                .add(design::toolbar_button(
                    "Set up voice output",
                    Icon::AUDIO_DEVICES,
                    ButtonVariant::Ghost,
                ))
                .clicked()
        {
            self.navigate(Page::Settings(SettingsTab::Audio));
        }
    }

    fn toast(&mut self, ctx: &egui::Context) {
        let Some(notice) = &self.notice else {
            return;
        };
        if self.shortcut_dialog.is_some()
            || self.clip_details.is_some()
            || self.trim_dialog.is_some()
        {
            return;
        }
        ctx.request_repaint_after(notice.expires.saturating_duration_since(Instant::now()));
        let mut dismiss = false;
        egui::Area::new(egui::Id::unique("notice"))
            .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-28.0, -73.0))
            .constrain(true)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let p = design::palette(ui);
                let (icon, color) = if notice.error {
                    (Icon::WARNING, p.destructive)
                } else {
                    (Icon::SUCCESS, p.primary)
                };
                Frame::new()
                    .fill(p.popover)
                    .corner_radius(12)
                    .inner_margin(Margin::symmetric(12, 8))
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 5],
                        blur: 18,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(56),
                    })
                    .show(ui, |ui| {
                        let width = 300.0;
                        let icon_size = 18.0;
                        let text_width =
                            width - icon_size - 32.0 - 2.0 * ui.spacing().item_spacing.x;
                        let text = ui.fonts_mut(|fonts| {
                            fonts.layout(
                                notice.text.clone(),
                                egui::FontId::proportional(13.0),
                                p.popover_foreground,
                                text_width,
                            )
                        });
                        let height = text.size().y.max(32.0);
                        let layout = Layout::left_to_right(Align::Center);
                        ui.allocate_ui_with_layout(egui::vec2(width, height), layout, |ui| {
                            design::icon(ui, icon, icon_size, color);
                            ui.allocate_ui_with_layout(
                                egui::vec2(text_width, height),
                                layout,
                                |ui| {
                                    ui.set_min_width(text_width);
                                    ui.add(egui::Label::new(text));
                                },
                            );
                            dismiss = design::icon_button(
                                ui,
                                "Dismiss notification",
                                Icon::CLOSE,
                                ButtonVariant::Ghost,
                            )
                            .clicked();
                        });
                    });
            });
        if dismiss {
            self.notice = None;
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.receive_updates(ctx);
        self.handle_tray(ctx);
        self.update_playback();
    }

    fn ui(&mut self, ui: &mut Ui, _: &mut eframe::Frame) {
        if std::mem::take(&mut self.hide_on_first_frame) {
            // eframe shows a root window after its first paint, then applies viewport commands.
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        let p = design::palette(ui);
        self.caption
            .apply(ui.visuals().dark_mode, p.card, p.card_foreground);
        self.navigation(ui);
        self.transport(ui);
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(p.background)
                    .inner_margin(Margin::symmetric(design::GUTTER, 21)),
            )
            .show(ui, |ui| match self.page {
                Page::Library => self.library(ui),
                Page::Settings(tab) => self.settings(ui, tab),
            });
        let ctx = ui.ctx();
        self.show_shortcut_dialog(ctx);
        self.show_clip_details(ctx);
        self.show_trim_dialog(ctx);
        self.toast(ctx);
        let focused = ctx.input(|input| input.focused);
        if self.on_air.is_some() || self.preview.is_some() {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if focused && self.route.microphone.is_some() && !self.settings.mic_muted {
            ctx.request_repaint_after(Duration::from_millis(32));
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if self.settings_save.is_some()
            && let Err(err) = self.store.save(SETTINGS_FILE, &self.settings)
        {
            self.store
                .log(&format!("Can't save settings on exit: {err:#}"));
        }
    }
}

fn clip_action(store: &Store, clip: &Clip) -> Result<Action> {
    Ok(Action::Play {
        path: store.clip_path(&clip.file)?,
        id: clip.id.clone(),
        title: clip.name.clone(),
        trim: clip.trim,
    })
}

fn clip_count(number: usize) -> String {
    format!("{number} {}", if number == 1 { "clip" } else { "clips" })
}

#[cfg(test)]
mod playback_timeline_tests {
    use super::*;

    #[test]
    fn advances_between_buffers_and_reaches_the_end() {
        let start = Instant::now();
        let mut timeline = PlaybackTimeline::new(96, start);
        assert_eq!(timeline.progress().elapsed_ms, 0);
        timeline.advance(32, start + Duration::from_millis(16));
        let first = timeline.progress().elapsed_ms;
        assert!((1..32).contains(&first));
        timeline.advance(32, start + Duration::from_millis(24));
        assert!((first + 1..=32).contains(&timeline.progress().elapsed_ms));
        timeline.advance(16, start + Duration::from_millis(32));
        assert!(timeline.progress().elapsed_ms >= first);
        timeline.advance(96, start + Duration::from_millis(120));
        assert_eq!(timeline.progress().elapsed_ms, 96);
        timeline.sync(
            ClipProgress {
                elapsed_ms: 12,
                duration_ms: 96,
                seek_revision: 1,
            },
            start + Duration::from_millis(128),
        );
        assert_eq!(timeline.progress().elapsed_ms, 12);
    }
}
