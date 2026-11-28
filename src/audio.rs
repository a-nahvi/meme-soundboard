use crate::storage::{ClipTrim, MAX_CLIP_SECONDS, Settings};
use anyhow::{Context, Result, bail};
use cpal::{
    Device, ErrorKind, FromSample, SampleFormat, SizedSample, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};
use rtrb::{Consumer, Producer, RingBuffer};
use std::{
    io::Cursor,
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{
            AtomicBool, AtomicU32, AtomicU64,
            Ordering::{Acquire, Relaxed, Release},
        },
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error as DecodeError,
    formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};

#[derive(Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub id: String,
    pub name: String,
    virtual_audio: Option<VirtualAudio>,
}

impl Endpoint {
    pub fn is_virtual(&self) -> bool {
        self.virtual_audio.is_some()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VirtualAudio {
    Vac,
    VbCable,
    Other,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SoundboardMicrophone {
    pub output_id: String,
    pub recording: Endpoint,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Devices {
    pub inputs: Vec<Endpoint>,
    pub outputs: Vec<Endpoint>,
    pub soundboard_microphone: Option<SoundboardMicrophone>,
    pub default_mic: Option<String>,
    pub default_headphones: Option<String>,
}

#[derive(Default)]
pub struct Meter {
    pub level: AtomicU32,
    pub speaking: AtomicBool,
    pub underruns: AtomicU64,
    pub device_xruns: AtomicU64,
    pub playing: PlaybackClock,
    pub monitoring: PlaybackClock,
    pub previewing: PlaybackClock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipProgress {
    pub elapsed_ms: u32,
    pub duration_ms: u32,
    pub seek_revision: u32,
}

// A clip is at most two minutes. The packed snapshot keeps a seek and its
// new position together without locking in the device callback.
const TIME_BITS: u32 = (MAX_CLIP_SECONDS as u32 * 1000).ilog2() + 1;
const TIME_MASK: u64 = (1 << TIME_BITS) - 1;
const REVISION_SHIFT: u32 = TIME_BITS * 2;
pub const MAX_SEEK_REVISION: u32 = (u64::MAX >> REVISION_SHIFT) as u32;

#[derive(Default)]
pub struct PlaybackClock {
    generation: AtomicU64,
    position: AtomicU64,
    completed: AtomicU64,
}

impl PlaybackClock {
    fn publish(&self, playback: Option<&Playback>) {
        let Some(playback) = playback else {
            self.generation.store(0, Release);
            return;
        };
        let generation = playback.play.generation;
        let starting = self.generation.load(Relaxed) != generation;
        if starting {
            // Prevent a reader from pairing a new clip's position with the old generation.
            self.generation.store(0, Release);
        }
        let play = &playback.play;
        let ms_per_frame = 1000.0 / f64::from(play.sound.rate);
        let elapsed = ((playback.frame - play.frames.start as f64) * ms_per_frame) as u32;
        let duration = play.duration_ms;
        self.position.store(
            (u64::from(playback.seek_revision) << REVISION_SHIFT)
                | (u64::from(duration) << TIME_BITS)
                | u64::from(elapsed.min(duration)),
            Release,
        );
        if starting {
            self.generation.store(generation, Release);
        }
    }

    pub fn get(&self, generation: u64) -> Option<ClipProgress> {
        if generation == 0 || self.generation.load(Acquire) != generation {
            return None;
        }
        let position = self.position.load(Acquire);
        if self.generation.load(Acquire) != generation {
            return None;
        }
        Some(ClipProgress {
            elapsed_ms: (position & TIME_MASK) as u32,
            duration_ms: ((position >> TIME_BITS) & TIME_MASK) as u32,
            seek_revision: (position >> REVISION_SHIFT) as u32,
        })
    }
}

#[derive(Default)]
struct Parameters {
    mic: AtomicU32,
    clips: AtomicU32,
    threshold: AtomicU32,
    gate: AtomicBool,
    muted: AtomicBool,
    clip_generation: AtomicU64,
    preview_generation: AtomicU64,
}

impl Parameters {
    fn update(&self, settings: &Settings) {
        self.mic
            .store(settings.mic_volume.clamp(0.0, 2.0).to_bits(), Relaxed);
        self.clips
            .store(settings.clip_volume.clamp(0.0, 1.5).to_bits(), Relaxed);
        self.threshold.store(
            10.0_f32
                .powf(settings.threshold_db.clamp(-65.0, -18.0) / 20.0)
                .to_bits(),
            Relaxed,
        );
        self.gate.store(settings.voice_gate, Relaxed);
        self.muted.store(settings.mic_muted, Relaxed);
    }

    fn generation(&self, preview: bool) -> &AtomicU64 {
        if preview {
            &self.preview_generation
        } else {
            &self.clip_generation
        }
    }

    fn cancel(&self, preview: bool, generation: u64) {
        let _ = self.generation(preview).compare_exchange(
            generation,
            generation.wrapping_add(1),
            Relaxed,
            Relaxed,
        );
    }
}

impl Meter {
    pub fn progress(&self, preview: bool, generation: u64) -> Option<ClipProgress> {
        if preview {
            self.previewing.get(generation)
        } else {
            self.playing
                .get(generation)
                .or_else(|| self.monitoring.get(generation))
        }
    }

    pub fn completed(&self, preview: bool, generation: u64) -> bool {
        let finished = |clock: &PlaybackClock| clock.completed.load(Acquire) == generation;
        if preview {
            finished(&self.previewing)
        } else {
            finished(&self.playing) || finished(&self.monitoring)
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Route {
    pub microphone: Option<String>,
    pub cable: Option<String>,
    pub headphones: Option<String>,
}

pub enum Event {
    Devices(Devices),
    Route(Route),
    Refreshed,
    PlaybackQueued {
        preview: bool,
        generation: u64,
    },
    ClipInspected {
        request: u64,
        result: Result<ClipAudioInfo, String>,
    },
    Error(Failure, String),
}

pub enum Failure {
    Clip,
    NoPreviewOutput,
    NoPlaybackOutput,
    Microphone,
    VoiceOutput,
    Headphones,
    Reconnect,
}

type EventSink = Arc<dyn Fn(Event) + Send + Sync>;

struct Sound {
    rate: u32,
    frames: Vec<[f32; 2]>,
}

pub struct ClipAudioInfo {
    pub duration_ms: u32,
    pub peaks: Vec<f32>,
}

impl Sound {
    fn duration_of(&self, frames: usize) -> u32 {
        ((frames as u64 * 1000).div_ceil(u64::from(self.rate)) as u32).max(1)
    }

    fn frame_at(&self, time_ms: u32) -> usize {
        (u64::from(time_ms) * u64::from(self.rate)).div_ceil(1000) as usize
    }

    fn info(&self) -> ClipAudioInfo {
        const BINS: usize = 144;
        let len = self.frames.len();
        let peaks = (0..BINS)
            .map(|bin| {
                let start = bin * len / BINS;
                let end = ((bin + 1) * len / BINS).max(start + 1);
                let samples = &self.frames[start..end];
                samples
                    .iter()
                    .step_by((samples.len() / 96).max(1))
                    .fold(0.0_f32, |peak, [left, right]| {
                        peak.max(left.abs()).max(right.abs())
                    })
            })
            .collect();
        ClipAudioInfo {
            duration_ms: self.duration_of(len),
            peaks,
        }
    }
}

struct CachedClip {
    path: PathBuf,
    modified: SystemTime,
    size: u64,
    sound: Arc<Sound>,
}

#[derive(Clone)]
struct Play {
    sound: Arc<Sound>,
    generation: u64,
    preview: bool,
    frames: Range<usize>,
    duration_ms: u32,
}

impl Play {
    fn new(
        sound: Arc<Sound>,
        generation: u64,
        preview: bool,
        trim: Option<ClipTrim>,
    ) -> Result<Self> {
        let len = sound.frames.len();
        let full_duration = sound.duration_of(len);
        let (frames, duration_ms) = if let Some(trim) = trim {
            let trim = trim.bounded(full_duration).context(
                "The saved trim is outside this audio file. Revert the trim to play it.",
            )?;
            let start = sound.frame_at(trim.start_ms);
            let end = sound.frame_at(trim.end_ms).min(len);
            (start..end, trim.duration_ms())
        } else {
            (0..len, full_duration)
        };
        anyhow::ensure!(
            !frames.is_empty(),
            "The selected part of the clip is too short"
        );
        Ok(Self {
            sound,
            generation,
            preview,
            frames,
            duration_ms,
        })
    }
}

#[derive(Clone, Copy)]
pub struct Seek {
    pub generation: u64,
    pub preview: bool,
    pub elapsed_ms: u32,
    pub revision: u32,
}

enum RenderCommand {
    Play(Play),
    Seek(Seek),
}

pub enum Source {
    File(PathBuf),
    Memory(Vec<u8>, String),
}

struct Decode {
    source: Source,
    generation: u64,
    preview: bool,
    trim: Option<ClipTrim>,
}

enum DecodeRequest {
    Play(Decode),
    Inspect { path: PathBuf, request: u64 },
}

enum Command {
    Settings(Settings),
    Refresh,
    Decoded(Play),
    Seek(Seek),
    Shutdown,
}

#[derive(Clone)]
pub struct Player {
    decode: mpsc::SyncSender<DecodeRequest>,
    params: Arc<Parameters>,
}

impl Player {
    pub fn play(&self, source: Source, preview: bool, trim: Option<ClipTrim>) -> Result<u64> {
        let generation = self.params.generation(preview).fetch_add(1, Relaxed) + 1;
        self.decode
            .try_send(DecodeRequest::Play(Decode {
                source,
                generation,
                preview,
                trim,
            }))
            .context("The audio decoder is busy")?;
        Ok(generation)
    }

    pub fn stop(&self) {
        self.params.clip_generation.fetch_add(1, Relaxed);
        self.params.preview_generation.fetch_add(1, Relaxed);
    }

    pub fn is_current(&self, preview: bool, generation: u64) -> bool {
        self.params.generation(preview).load(Relaxed) == generation
    }

    pub fn stop_clip(&self, preview: bool, generation: u64) {
        self.params.cancel(preview, generation);
    }
}

pub struct Engine {
    commands: Sender<Command>,
    pub events: Receiver<Event>,
    pub meter: Arc<Meter>,
    pub player: Player,
}

impl Engine {
    pub fn start(settings: Settings, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (commands, control) = mpsc::channel();
        let (decode, jobs) = mpsc::sync_channel::<DecodeRequest>(12);
        let (events, receiver) = mpsc::channel();
        let events: EventSink = Arc::new(move |event| {
            if events.send(event).is_ok() {
                wake();
            }
        });
        let meter = Arc::new(Meter::default());
        let params = Arc::new(Parameters::default());
        params.update(&settings);

        let decoded = commands.clone();
        let decode_events = events.clone();
        let decode_params = params.clone();
        thread::spawn(move || {
            // Decode away from the device callbacks. Also retain the last file for repeated hotkeys.
            let mut cached: Option<CachedClip> = None;
            while let Ok(request) = jobs.recv() {
                let job = match request {
                    DecodeRequest::Play(job) => job,
                    DecodeRequest::Inspect { path, request } => {
                        let result = load_sound(&path, &mut cached)
                            .map(|sound| sound.info())
                            .map_err(|err| format!("Can't read clip. {err:#}"));
                        decode_events(Event::ClipInspected { request, result });
                        continue;
                    }
                };
                let current =
                    || decode_params.generation(job.preview).load(Relaxed) == job.generation;
                if !current() {
                    continue;
                }
                let sound = match job.source {
                    Source::File(path) => load_sound(&path, &mut cached),
                    Source::Memory(bytes, extension) => {
                        decode_audio(Box::new(Cursor::new(bytes)), &extension).map(Arc::new)
                    }
                };
                if current() {
                    match sound
                        .and_then(|sound| Play::new(sound, job.generation, job.preview, job.trim))
                    {
                        Ok(play) => {
                            let _ = decoded.send(Command::Decoded(play));
                        }
                        Err(err) => {
                            decode_params.cancel(job.preview, job.generation);
                            decode_events(Event::Error(
                                Failure::Clip,
                                format!("Can't play clip. {err:#}"),
                            ));
                        }
                    }
                }
            }
        });

        let device_meter = meter.clone();
        let device_params = params.clone();
        thread::spawn(move || {
            run_device_loop(control, events, settings, device_params, device_meter)
        });
        Self {
            commands,
            events: receiver,
            meter,
            player: Player { decode, params },
        }
    }

    pub fn settings(&self, settings: &Settings) {
        self.player.params.update(settings);
        let _ = self.commands.send(Command::Settings(settings.clone()));
    }

    pub fn refresh(&self) {
        let _ = self.commands.send(Command::Refresh);
    }

    pub fn inspect_clip(&self, path: PathBuf, request: u64) -> Result<()> {
        self.player
            .decode
            .try_send(DecodeRequest::Inspect { path, request })
            .context("The audio decoder is busy")
    }

    pub fn seek(&self, seek: Seek) -> Result<()> {
        self.commands
            .send(Command::Seek(seek))
            .context("Audio playback is unavailable")
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
    }
}

fn load_sound(path: &Path, cached: &mut Option<CachedClip>) -> Result<Arc<Sound>> {
    let file = std::fs::File::open(path).context("Can't open the clip")?;
    let stamp = file
        .metadata()
        .ok()
        .and_then(|meta| Some((meta.modified().ok()?, meta.len())));
    if let Some(cached) = cached
        .as_ref()
        .filter(|clip| clip.path == path && stamp == Some((clip.modified, clip.size)))
    {
        return Ok(cached.sound.clone());
    }
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
    let sound = Arc::new(decode_audio(Box::new(file), extension)?);
    *cached = stamp.map(|(modified, size)| CachedClip {
        path: path.to_owned(),
        modified,
        size,
        sound: sound.clone(),
    });
    Ok(sound)
}

pub fn validate(bytes: &[u8], extension: &str) -> Result<()> {
    decode_audio(Box::new(Cursor::new(bytes.to_vec())), extension).map(|_| ())
}

pub fn validate_file(file: std::fs::File, extension: &str) -> Result<()> {
    decode_audio(Box::new(file), extension).map(|_| ())
}

fn decode_audio(
    source: Box<dyn symphonia::core::io::MediaSource>,
    extension: &str,
) -> Result<Sound> {
    let mut hint = Hint::new();
    hint.with_extension(extension);
    let stream = MediaSourceStream::new(source, Default::default());
    let mut format = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?
        .format;
    let track = format.default_track().context("No audio track was found")?;
    let track_id = track.id;
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let mut frames = Vec::new();
    let mut rate = 0;
    let mut original_rate = 0;
    let mut stride = 1;
    let mut frame_index = 0;
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(DecodeError::IoError(err)) if err.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(err) => return Err(err.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet)?;
        let spec = *decoded.spec();
        if rate == 0 {
            if !(8_000..=384_000).contains(&spec.rate) || spec.channels.count() == 0 {
                bail!("Unsupported sample rate or channel layout");
            }
            original_rate = spec.rate;
            stride = spec.rate.div_ceil(48_000) as usize;
            rate = spec.rate / stride as u32;
        }
        if original_rate != spec.rate {
            bail!("The clip changes its sample rate");
        }
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        for samples in buffer.samples().chunks_exact(spec.channels.count()) {
            if frame_index >= original_rate as usize * MAX_CLIP_SECONDS {
                bail!("Clips must be two minutes or shorter");
            }
            if frame_index % stride == 0 {
                let left = clean(samples[0]);
                let right = clean(samples.get(1).copied().unwrap_or(left));
                frames.push([left, right]);
            }
            frame_index += 1;
        }
    }
    if frames.is_empty() {
        bail!("The clip contains no playable audio");
    }
    Ok(Sound { rate, frames })
}

fn clean(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

fn endpoint(device: &Device) -> Option<Endpoint> {
    let id = device.id().ok()?.to_string();
    let description = device.description().ok()?;
    let name = description.name().to_owned();
    let virtual_audio = classify_virtual_audio(&name, description.driver().unwrap_or_default());
    Some(Endpoint {
        id,
        name,
        virtual_audio,
    })
}

fn classify_virtual_audio(name: &str, adapter: &str) -> Option<VirtualAudio> {
    let name = name.to_ascii_lowercase();
    let adapter = adapter.to_ascii_lowercase();
    if adapter.trim() == "virtual audio cable" || name.ends_with(" (virtual audio cable)") {
        Some(VirtualAudio::Vac)
    } else if name.starts_with("cable input") || name.starts_with("cable output") {
        Some(VirtualAudio::VbCable)
    } else if [
        "vb-audio",
        "vb-cable",
        "voicemeeter",
        "virtual audio cable",
        "cable input",
        "cable output",
    ]
    .into_iter()
    .any(|tag| name.contains(tag) || adapter.contains(tag))
    {
        Some(VirtualAudio::Other)
    } else {
        None
    }
}

fn single_vac_endpoint(endpoints: &[Endpoint]) -> Option<&Endpoint> {
    let mut vac = endpoints
        .iter()
        .filter(|item| item.virtual_audio == Some(VirtualAudio::Vac));
    let device = vac.next()?;
    // The Lite package has one cable. Multiple devices can expose different VAC lines.
    vac.next().is_none().then_some(device)
}

fn soundboard_microphone(
    inputs: &[Endpoint],
    outputs: &[Endpoint],
) -> Option<SoundboardMicrophone> {
    Some(SoundboardMicrophone {
        recording: single_vac_endpoint(inputs)?.clone(),
        output_id: single_vac_endpoint(outputs)?.id.clone(),
    })
}

pub fn soundboard_microphone_available() -> bool {
    enumerate(&cpal::default_host())
        .soundboard_microphone
        .is_some()
}

fn enumerate(host: &cpal::Host) -> Devices {
    let mut inputs: Vec<_> = host
        .input_devices()
        .map(|devices| devices.filter_map(|item| endpoint(&item)).collect())
        .unwrap_or_default();
    let outputs: Vec<_> = host
        .output_devices()
        .map(|devices| devices.filter_map(|item| endpoint(&item)).collect())
        .unwrap_or_default();
    let soundboard_microphone = soundboard_microphone(&inputs, &outputs);
    inputs.retain(|item| !item.is_virtual());
    let default_mic = host
        .default_input_device()
        .and_then(|item| endpoint(&item))
        .filter(|item| !item.is_virtual())
        .map(|item| item.id);
    let default_headphones = host
        .default_output_device()
        .and_then(|item| endpoint(&item))
        .filter(|item| !item.is_virtual())
        .map(|item| item.id);
    Devices {
        inputs,
        outputs,
        soundboard_microphone,
        default_mic,
        default_headphones,
    }
}

fn device_id(
    options: &[Endpoint],
    requested: &Option<String>,
    default: Option<&String>,
) -> Option<String> {
    let id = requested.as_ref().or(default)?;
    options
        .iter()
        .find(|item| &item.id == id)
        .map(|item| item.id.clone())
}

fn select_route(devices: &Devices, settings: &Settings) -> Route {
    let preferred = devices
        .soundboard_microphone
        .as_ref()
        .map(|device| &device.output_id);
    let cable = if settings.local_playback_only {
        None
    } else {
        let default = preferred.or_else(|| {
            devices
                .outputs
                .iter()
                .filter(|item| item.is_virtual())
                .min_by_key(|item| item.virtual_audio != Some(VirtualAudio::VbCable))
                .map(|item| &item.id)
        });
        device_id(&devices.outputs, &settings.cable, default).or_else(|| preferred.cloned())
    };
    let headphones = device_id(
        &devices.outputs,
        &settings.headphones,
        devices.default_headphones.as_ref().or_else(|| {
            devices
                .outputs
                .iter()
                .find(|item| !item.is_virtual() && Some(&item.id) != cable.as_ref())
                .map(|item| &item.id)
        }),
    )
    .filter(|id| {
        Some(id) != cable.as_ref()
            && devices
                .outputs
                .iter()
                .any(|item| &item.id == id && !item.is_virtual())
    });
    let microphone = cable.as_ref().and_then(|_| {
        device_id(
            &devices.inputs,
            &settings.microphone,
            devices
                .default_mic
                .as_ref()
                .or_else(|| devices.inputs.first().map(|item| &item.id)),
        )
    });
    Route {
        microphone,
        cable,
        headphones,
    }
}

struct Running {
    _streams: Vec<Stream>,
    game: Option<Producer<RenderCommand>>,
    headphones: Option<Producer<RenderCommand>>,
    retired: Consumer<Arc<Sound>>,
    preview_retired: Consumer<Arc<Sound>>,
    failed: Arc<AtomicBool>,
}

struct StreamSignals<'a> {
    params: &'a Arc<Parameters>,
    meter: &'a Arc<Meter>,
    events: &'a EventSink,
}

fn run_device_loop(
    commands: Receiver<Command>,
    events: EventSink,
    mut settings: Settings,
    params: Arc<Parameters>,
    meter: Arc<Meter>,
) {
    let host = cpal::default_host();
    let mut devices = None;
    let mut configured = None;
    let mut active = Route::default();
    let mut running: Option<Running> = None;
    let mut poll_at = Instant::now();
    let mut refresh_requested = false;
    let signals = StreamSignals {
        params: &params,
        meter: &meter,
        events: &events,
    };

    loop {
        if let Some(state) = &mut running {
            while state.retired.pop().is_ok() {}
            while state.preview_retired.pop().is_ok() {}
        }
        if Instant::now() >= poll_at {
            poll_at = Instant::now() + Duration::from_millis(2500);
            let current = enumerate(&host);
            if devices.as_ref() != Some(&current) {
                events(Event::Devices(current.clone()));
                devices = Some(current.clone());
            }
            let wanted = select_route(&current, &settings);
            if configured.as_ref() != Some(&wanted)
                || (refresh_requested && active != wanted)
                || running
                    .as_ref()
                    .is_some_and(|state| state.failed.load(Relaxed))
            {
                running.take();
                meter.level.store(0, Relaxed);
                meter.speaking.store(false, Relaxed);
                meter.playing.publish(None);
                meter.monitoring.publish(None);
                meter.previewing.publish(None);
                let (state, opened) = open_route(&host, &wanted, &signals);
                running = Some(state);
                configured = Some(wanted);
                active = opened;
                events(Event::Route(active.clone()));
            }
            if std::mem::take(&mut refresh_requested) {
                events(Event::Refreshed);
            }
        }
        match commands.recv_timeout(
            poll_at
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(200)),
        ) {
            Ok(Command::Settings(updated)) => {
                let routes_changed = settings.microphone != updated.microphone
                    || settings.cable != updated.cable
                    || settings.local_playback_only != updated.local_playback_only
                    || settings.headphones != updated.headphones;
                settings = updated;
                if routes_changed {
                    poll_at = Instant::now();
                }
            }
            Ok(Command::Refresh) => {
                poll_at = Instant::now();
                refresh_requested = true;
            }
            Ok(Command::Seek(seek)) => {
                if params.generation(seek.preview).load(Relaxed) != seek.generation {
                    continue;
                }
                if let Some(state) = &mut running {
                    if !seek.preview
                        && let Some(queue) = &mut state.game
                    {
                        let _ = queue.push(RenderCommand::Seek(seek));
                    }
                    if let Some(queue) = &mut state.headphones {
                        let _ = queue.push(RenderCommand::Seek(seek));
                    }
                }
            }
            Ok(Command::Decoded(play)) => {
                let Play {
                    preview,
                    generation,
                    ..
                } = play;
                if params.generation(preview).load(Relaxed) != generation {
                    continue;
                }
                if let Some(state) = &mut running {
                    let queued = if play.preview {
                        if let Some(queue) = &mut state.headphones {
                            queue.push(RenderCommand::Play(play)).is_ok()
                        } else {
                            events(Event::Error(Failure::NoPreviewOutput, "Choose a headphone or speaker output in Settings to preview clips.".into()));
                            false
                        }
                    } else if let Some(queue) = &mut state.game {
                        let monitored = settings.monitor_clips
                            && state.headphones.as_mut().is_some_and(|monitor| {
                                monitor.push(RenderCommand::Play(play.clone())).is_ok()
                            });
                        queue.push(RenderCommand::Play(play)).is_ok() || monitored
                    } else {
                        if let Some(queue) = &mut state.headphones {
                            queue.push(RenderCommand::Play(play)).is_ok()
                        } else {
                            events(Event::Error(
                                Failure::NoPlaybackOutput,
                                "Choose a headphone, speaker or voice output in Audio devices."
                                    .into(),
                            ));
                            false
                        }
                    };
                    if queued {
                        events(Event::PlaybackQueued {
                            preview,
                            generation,
                        });
                    } else {
                        params.cancel(preview, generation);
                    }
                } else {
                    params.cancel(preview, generation);
                }
            }
            Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn open_route(host: &cpal::Host, wanted: &Route, signals: &StreamSignals<'_>) -> (Running, Route) {
    let mut active = Route::default();
    let mut streams = Vec::new();
    let failed = Arc::new(AtomicBool::new(false));
    let (retire, retired) = RingBuffer::new(48);
    let (preview_retire, preview_retired) = RingBuffer::new(48);
    let find = |id: &str| {
        host.device_by_id(&id.parse().ok()?)
            .context("Device is unavailable")
            .ok()
    };
    let report = |kind, label: &str, err: anyhow::Error| {
        (signals.events)(Event::Error(kind, format!("{label}. {err:#}")));
    };
    let mic = wanted.microphone.as_deref().and_then(find);
    let mut input = None;
    if let Some(device) = mic {
        match open_input(&device, signals, &failed) {
            Ok((stream, reader)) => {
                active.microphone = wanted.microphone.clone();
                streams.push(stream);
                input = Some(reader);
            }
            Err(err) => report(Failure::Microphone, "Can't open the microphone", err),
        }
    }
    let mut game = None;
    if let Some(device) = wanted.cable.as_deref().and_then(find) {
        match open_output(&device, input, false, retire, signals, &failed) {
            Ok((stream, queue)) => {
                active.cable = wanted.cable.clone();
                streams.push(stream);
                game = Some(queue);
            }
            Err(err) => report(Failure::VoiceOutput, "Can't open the virtual cable", err),
        }
    }
    let mut headphones = None;
    if let Some(device) = wanted.headphones.as_deref().and_then(find) {
        match open_output(&device, None, true, preview_retire, signals, &failed) {
            Ok((stream, queue)) => {
                active.headphones = wanted.headphones.clone();
                streams.push(stream);
                headphones = Some(queue);
            }
            Err(err) => report(Failure::Headphones, "Can't open the headphone output", err),
        }
    }
    (
        Running {
            _streams: streams,
            game,
            headphones,
            retired,
            preview_retired,
            failed,
        },
        active,
    )
}

struct Microphone {
    queue: Consumer<f32>,
    rate: u32,
    target: usize,
    pair: [f32; 2],
    fraction: f64,
    ready: bool,
}

impl Microphone {
    fn prepare(&mut self, output_rate: u32) -> f64 {
        let samples = self.queue.slots();
        if samples > self.target * 4 {
            for _ in 0..samples - self.target * 2 {
                let _ = self.queue.pop();
            }
            self.ready = false;
        }
        if !self.ready && self.queue.slots() >= self.target {
            self.pair = [
                self.queue.pop().unwrap_or_default(),
                self.queue.pop().unwrap_or_default(),
            ];
            self.fraction = 0.0;
            self.ready = true;
        }
        // The hardware microphone and the virtual cable have independent clocks.
        let offset = self.queue.slots() as f64 / self.target as f64 - 1.0;
        f64::from(self.rate) / f64::from(output_rate)
            * (1.0 + (offset * 0.003).clamp(-0.007, 0.007))
    }

    fn next(&mut self, step: f64, underruns: &AtomicU64) -> f32 {
        if !self.ready {
            return 0.0;
        }
        let sample = self.pair[0] + (self.pair[1] - self.pair[0]) * self.fraction as f32;
        self.fraction += step;
        while self.fraction >= 1.0 {
            self.fraction -= 1.0;
            match self.queue.pop() {
                Ok(next) => self.pair = [self.pair[1], next],
                Err(_) => {
                    self.ready = false;
                    underruns.fetch_add(1, Relaxed);
                    break;
                }
            }
        }
        sample
    }
}

macro_rules! with_samples {
    ($format:expr, $factory:ident, $($arg:expr),+ $(,)?) => {
        match $format {
            SampleFormat::F32 => $factory::<f32>($($arg),+),
            SampleFormat::F64 => $factory::<f64>($($arg),+),
            SampleFormat::I8 => $factory::<i8>($($arg),+),
            SampleFormat::I16 => $factory::<i16>($($arg),+),
            SampleFormat::I24 => $factory::<cpal::I24>($($arg),+),
            SampleFormat::I32 => $factory::<i32>($($arg),+),
            SampleFormat::U8 => $factory::<u8>($($arg),+),
            SampleFormat::U16 => $factory::<u16>($($arg),+),
            SampleFormat::U32 => $factory::<u32>($($arg),+),
            format => bail!("Unsupported device sample format {format}"),
        }
    };
}

fn open_input(
    device: &Device,
    signals: &StreamSignals<'_>,
    failed: &Arc<AtomicBool>,
) -> Result<(Stream, Microphone)> {
    let supported = device.default_input_config()?;
    let format = supported.sample_format();
    let config = supported.config();
    let rate = config.sample_rate;
    let target = (rate as usize * 28 / 1000).max(8);
    let (queue, reader) = RingBuffer::new(target * 8);
    let mic = Microphone {
        queue: reader,
        rate,
        target,
        pair: [0.0; 2],
        fraction: 0.0,
        ready: false,
    };
    let stream = with_samples!(format, input_stream, device, config, queue, signals, failed)?;
    stream.play()?;
    Ok((stream, mic))
}

fn input_stream<T: SizedSample>(
    device: &Device,
    config: StreamConfig,
    mut queue: Producer<f32>,
    signals: &StreamSignals<'_>,
    failed: &Arc<AtomicBool>,
) -> Result<Stream>
where
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels);
    let rate = config.sample_rate;
    let meter = signals.meter.clone();
    let params = signals.params.clone();
    let mut hold: usize = 0;
    let mut envelope: f32 = 1.0;
    let stream = device.build_input_stream::<T, _, _>(
        config,
        move |data, _| {
            let samples = data.chunks_exact(channels);
            let count = samples.len();
            let power: f32 = samples
                .clone()
                .map(|frame| {
                    let sample = mic_sample(frame);
                    sample * sample
                })
                .sum();
            let rms = (power / count.max(1) as f32).sqrt();
            let threshold = f32::from_bits(params.threshold.load(Relaxed));
            hold = hold.saturating_sub(count);
            if rms >= threshold {
                hold = rate as usize / 5;
            }
            meter.level.store(rms.to_bits(), Relaxed);
            meter.speaking.store(hold > 0, Relaxed);
            let mic_open = !params.muted.load(Relaxed) && (!params.gate.load(Relaxed) || hold > 0);
            let volume = f32::from_bits(params.mic.load(Relaxed));
            let target = if mic_open { 1.0 } else { 0.0 };
            let speed = 1.0 / (rate as f32 * if mic_open { 0.004 } else { 0.016 });
            for frame in samples {
                envelope += (target - envelope).clamp(-speed, speed);
                let _ = queue.push(mic_sample(frame) * volume * envelope);
            }
        },
        error_handler(failed, signals.meter, signals.events),
        None,
    )?;
    Ok(stream)
}

fn mic_sample<T: SizedSample>(frame: &[T]) -> f32
where
    f32: FromSample<T>,
{
    let left = clean(f32::from_sample_(frame[0]));
    frame.get(1).map_or(left, |right| {
        (left + clean(f32::from_sample_(*right))) * 0.5
    })
}

struct Playback {
    play: Play,
    frame: f64,
    origin_frame: f64,
    rendered_frames: u32,
    seek_revision: u32,
}

impl Playback {
    fn new(play: Play) -> Self {
        let frame = play.frames.start as f64;
        Self {
            play,
            frame,
            origin_frame: frame,
            rendered_frames: 0,
            seek_revision: 0,
        }
    }

    fn next(&mut self, rate: u32) -> Option<[f32; 2]> {
        let index = self.frame as usize;
        if index >= self.play.frames.end {
            return None;
        }
        let sound = &self.play.sound;
        let begin = *sound.frames.get(index)?;
        let end = if index + 1 < self.play.frames.end {
            sound.frames[index + 1]
        } else {
            [0.0; 2]
        };
        let blend = self.frame.fract() as f32;
        // Counting output frames keeps a trim boundary stable across sample rates.
        self.rendered_frames += 1;
        self.frame = self.origin_frame
            + f64::from(self.rendered_frames) * f64::from(sound.rate) / f64::from(rate);
        Some(std::array::from_fn(|channel| {
            begin[channel] + (end[channel] - begin[channel]) * blend
        }))
    }

    fn seek(&mut self, seek: Seek) {
        let frame = self
            .play
            .frames
            .start
            .saturating_add(self.play.sound.frame_at(seek.elapsed_ms));
        self.frame = frame.min(self.play.frames.end - 1) as f64;
        self.origin_frame = self.frame;
        self.rendered_frames = 0;
        self.seek_revision = seek.revision;
    }
}

struct Output {
    render: Consumer<RenderCommand>,
    retired: Producer<Arc<Sound>>,
    active: Option<Playback>,
    mic: Option<Microphone>,
    preview: bool,
}

impl Output {
    fn retire(&mut self, sound: Arc<Sound>) {
        // The control thread releases decoded buffers off the audio thread.
        let _ = self.retired.push(sound);
    }

    fn is_current(play: &Play, params: &Parameters) -> bool {
        params.generation(play.preview).load(Relaxed) == play.generation
    }

    fn prepare(&mut self, params: &Parameters) {
        if self
            .active
            .as_ref()
            .is_some_and(|active| !Self::is_current(&active.play, params))
        {
            let active = self.active.take().unwrap();
            self.retire(active.play.sound);
        }
        while let Ok(command) = self.render.pop() {
            match command {
                RenderCommand::Play(play) if Self::is_current(&play, params) => {
                    if let Some(old) = self.active.replace(Playback::new(play)) {
                        self.retire(old.play.sound);
                    }
                }
                RenderCommand::Play(play) => self.retire(play.sound),
                RenderCommand::Seek(seek) => {
                    if let Some(active) = &mut self.active
                        && active.play.generation == seek.generation
                        && active.play.preview == seek.preview
                    {
                        active.seek(seek);
                    }
                }
            }
        }
    }

    fn next(&mut self, rate: u32, meter: &Meter) -> [f32; 2] {
        if let Some(active) = &mut self.active {
            if let Some(samples) = active.next(rate) {
                return samples;
            }
            let active = self.active.take().unwrap();
            let clock = if active.play.preview {
                &meter.previewing
            } else if self.preview {
                &meter.monitoring
            } else {
                &meter.playing
            };
            clock.completed.store(active.play.generation, Release);
            self.retire(active.play.sound);
        }
        [0.0; 2]
    }
}

fn open_output(
    device: &Device,
    mic: Option<Microphone>,
    preview: bool,
    retired: Producer<Arc<Sound>>,
    signals: &StreamSignals<'_>,
    failed: &Arc<AtomicBool>,
) -> Result<(Stream, Producer<RenderCommand>)> {
    let supported = device.default_output_config()?;
    let (queue, render) = RingBuffer::new(16);
    let output = Output {
        render,
        retired,
        active: None,
        mic,
        preview,
    };
    let stream = with_samples!(
        supported.sample_format(),
        output_stream,
        device,
        supported.config(),
        output,
        signals,
        failed
    )?;
    stream.play()?;
    Ok((stream, queue))
}

fn output_stream<T: SizedSample + FromSample<f32>>(
    device: &Device,
    config: StreamConfig,
    mut output: Output,
    signals: &StreamSignals<'_>,
    failed: &Arc<AtomicBool>,
) -> Result<Stream> {
    let channels = usize::from(config.channels);
    let rate = config.sample_rate;
    let params = signals.params.clone();
    let meter = signals.meter.clone();
    let stream = device.build_output_stream::<T, _, _>(
        config,
        move |data, _| {
            output.prepare(&params);
            let step = output
                .mic
                .as_mut()
                .map(|mic| mic.prepare(rate))
                .unwrap_or(1.0);
            let clip_gain = f32::from_bits(params.clips.load(Relaxed));
            for frame in data.chunks_exact_mut(channels) {
                let [left, right] = output.next(rate, &meter);
                let mic = output
                    .mic
                    .as_mut()
                    .map(|mic| mic.next(step, &meter.underruns))
                    .unwrap_or(0.0);
                if channels == 1 {
                    frame[0] = T::from_sample_(mix(mic, (left + right) * 0.5, clip_gain));
                } else {
                    frame.fill(T::EQUILIBRIUM);
                    frame[0] = T::from_sample_(mix(mic, left, clip_gain));
                    frame[1] = T::from_sample_(mix(mic, right, clip_gain));
                }
            }
            let play = output.active.as_ref();
            if output.preview {
                meter
                    .previewing
                    .publish(play.filter(|entry| entry.play.preview));
                meter
                    .monitoring
                    .publish(play.filter(|entry| !entry.play.preview));
            } else {
                meter.playing.publish(play);
            }
        },
        error_handler(failed, signals.meter, signals.events),
        None,
    )?;
    Ok(stream)
}

fn error_handler(
    failed: &Arc<AtomicBool>,
    meter: &Arc<Meter>,
    events: &EventSink,
) -> impl FnMut(cpal::Error) + Send + 'static {
    let failed = failed.clone();
    let meter = meter.clone();
    let events = events.clone();
    move |err| {
        // WASAPI continues delivering samples after reporting a discontinuity.
        if err.kind() == ErrorKind::Xrun {
            meter.device_xruns.fetch_add(1, Relaxed);
            return;
        }
        if !failed.swap(true, Relaxed) {
            events(Event::Error(
                Failure::Reconnect,
                format!("Audio stream interrupted. Reconnecting. {err}"),
            ));
        }
    }
}

fn mix(voice: f32, clip: f32, gain: f32) -> f32 {
    let sum = voice + clip * gain;
    let excess = (sum.abs() - 0.80).max(0.0);
    if excess == 0.0 {
        sum
    } else {
        sum.signum() * (0.80 + 0.20 * excess / (0.20 + excess))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_microphone_routes_voice_and_clips_without_vb_cable() {
        let device = |id: &str, name: &str, adapter: &str| Endpoint {
            id: id.into(),
            name: name.into(),
            virtual_audio: classify_virtual_audio(name, adapter),
        };
        let microphone = device("mic", "Microphone (USB Audio)", "USB Audio");
        let headphones = device("phones", "Headphones (USB Audio)", "USB Audio");
        let vac_out = device(
            "vac-out",
            "Line 1 (Virtual Audio Cable)",
            "Virtual Audio Cable",
        );
        let vac_in = device("vac-in", "Soundboard Microphone", "Virtual Audio Cable");
        let pair = soundboard_microphone(
            &[microphone.clone(), vac_in],
            &[headphones.clone(), vac_out.clone()],
        )
        .unwrap();
        assert_eq!(pair.recording.id, "vac-in");
        assert_eq!(pair.output_id, "vac-out");
        let mut devices = Devices {
            inputs: vec![microphone],
            outputs: vec![headphones, vac_out],
            soundboard_microphone: Some(pair),
            ..Default::default()
        };
        let mut settings = Settings {
            cable: Some("removed-vb-cable".into()),
            ..Settings::default()
        };
        let route = select_route(&devices, &settings);
        assert_eq!(route.microphone.as_deref(), Some("mic"));
        assert_eq!(route.cable.as_deref(), Some("vac-out"));
        assert_eq!(route.headphones.as_deref(), Some("phones"));

        devices.outputs.push(device(
            "vb-out",
            "CABLE Input (VB-Audio Virtual Cable)",
            "VB-Audio Virtual Cable",
        ));
        settings.cable = Some("vb-out".into());
        assert_eq!(
            select_route(&devices, &settings).cable.as_deref(),
            Some("vb-out")
        );
        settings.local_playback_only = true;
        let local = select_route(&devices, &settings);
        assert!(local.cable.is_none() && local.microphone.is_none());
        assert_eq!(local.headphones.as_deref(), Some("phones"));
    }

    #[test]
    #[ignore = "requires the optional signed VAC Lite device on Windows"]
    fn soundboard_microphone_delivers_both_signals_to_windows_capture() -> Result<()> {
        let host = cpal::default_host();
        let devices = enumerate(&host);
        let virtual_mic = devices
            .soundboard_microphone
            .as_ref()
            .context("Enable VAC Lite playback and recording before running this test")?;
        let settings = Settings::default();
        assert_eq!(
            select_route(&devices, &settings).cable.as_deref(),
            Some(virtual_mic.output_id.as_str())
        );
        let recording = host
            .device_by_id(&virtual_mic.recording.id.parse()?)
            .context("The virtual recording endpoint is unavailable")?;
        let playback = host
            .device_by_id(&virtual_mic.output_id.parse()?)
            .context("The virtual playback endpoint is unavailable")?;
        let params = Arc::new(Parameters::default());
        params.update(&settings);
        let meter = Arc::new(Meter::default());
        let events: EventSink = Arc::new(|event| {
            if let Event::Error(_, message) = event {
                eprintln!("{message}");
            }
        });
        let signals = StreamSignals {
            params: &params,
            meter: &meter,
            events: &events,
        };
        let failed = Arc::new(AtomicBool::new(false));
        let (_capture_stream, mut capture) = open_input(&recording, &signals, &failed)?;
        let rate = capture.rate;
        let tone = |frame: u32, frequency: f32| {
            (std::f32::consts::TAU * frequency * frame as f32 / rate as f32).sin()
        };
        let (mut voice, input) = RingBuffer::new((rate * 3) as usize);
        for frame in 0..(rate * 3) {
            voice.push(0.20 * tone(frame, 220.0)).unwrap();
        }
        let microphone = Microphone {
            queue: input,
            rate,
            target: rate as usize,
            pair: [0.0; 2],
            fraction: 0.0,
            ready: false,
        };
        let (retire, _retired) = RingBuffer::new(4);
        let (_output_stream, mut commands) = open_output(
            &playback,
            Some(microphone),
            false,
            retire,
            &signals,
            &failed,
        )?;
        let clip = Arc::new(Sound {
            rate,
            frames: (0..(rate * 2))
                .map(|frame| [0.28 * tone(frame, 840.0); 2])
                .collect(),
        });
        params.clip_generation.store(1, Relaxed);
        assert!(
            commands
                .push(RenderCommand::Play(Play::new(clip, 1, false, None)?))
                .is_ok()
        );
        let mut samples = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !failed.load(Relaxed) {
            while let Ok(sample) = capture.queue.pop() {
                samples.push(sample);
            }
            thread::sleep(Duration::from_millis(8));
        }
        let level = |window: &[f32], frequency: f32| {
            let (sin, cos) =
                window
                    .iter()
                    .enumerate()
                    .fold((0.0, 0.0), |(sin, cos), (frame, &sample)| {
                        let (s, c) = (std::f32::consts::TAU * frequency * frame as f32
                            / rate as f32)
                            .sin_cos();
                        (sin + s * sample, cos + c * sample)
                    });
            2.0 * sin.hypot(cos) / window.len() as f32
        };
        let both = samples
            .chunks_exact((rate / 10) as usize)
            .filter(|window| level(window, 220.0) > 0.05 && level(window, 840.0) > 0.05)
            .count();
        assert!(
            both >= 5,
            "Only {both} capture windows contained both signals."
        );
        println!("{} captured both test signals.", virtual_mic.recording.name);
        Ok(())
    }

    #[test]
    fn trim_limits_samples_and_seeking_uses_the_trimmed_timeline() {
        let sound = Arc::new(Sound {
            rate: 1000,
            frames: (0..6).map(|sample| [sample as f32; 2]).collect(),
        });
        let play = Play::new(
            sound.clone(),
            1,
            false,
            Some(ClipTrim {
                start_ms: 2,
                end_ms: 5,
            }),
        )
        .unwrap();
        let mut playback = Playback::new(play);
        assert_eq!(playback.next(1000), Some([2.0; 2]));

        playback.seek(Seek {
            generation: 1,
            preview: false,
            elapsed_ms: 2,
            revision: 1,
        });
        let clock = PlaybackClock::default();
        clock.publish(Some(&playback));
        assert_eq!(
            clock.get(1),
            Some(ClipProgress {
                elapsed_ms: 2,
                duration_ms: 3,
                seek_revision: 1
            })
        );
        assert_eq!(playback.next(1000), Some([4.0; 2]));
        assert_eq!(playback.next(1000), None);

        playback.seek(Seek {
            generation: 1,
            preview: false,
            elapsed_ms: 0,
            revision: 2,
        });
        assert_eq!(playback.next(1000), Some([2.0; 2]));
        assert_eq!(Play::new(sound, 2, false, None).unwrap().frames, 0..6);
    }

    #[test]
    fn one_second_selection_plays_the_same_samples_after_saving() {
        let rate = 44_100;
        let range = 31_973..76_073;
        let mut frames = vec![[-0.5; 2]; 85_000];
        frames[range.clone()].fill([0.25; 2]);
        let sound = Arc::new(Sound { rate, frames });
        let mut clip = crate::storage::Clip::from_folder("selection.wav".into());
        clip.trim = Some(ClipTrim {
            start_ms: 725,
            end_ms: 1725,
        });
        let saved = serde_json::to_vec(&clip).unwrap();
        let restored: crate::storage::Clip = serde_json::from_slice(&saved).unwrap();
        assert_eq!(restored.trim, clip.trim);

        for (preview, selection) in [
            (true, clip.trim),
            (false, restored.trim),
            (true, restored.trim),
        ] {
            let play = Play::new(sound.clone(), 1, preview, selection).unwrap();
            assert_eq!(play.frames, range);
            assert_eq!(play.duration_ms, 1000);
            let mut playback = Playback::new(play.clone());
            assert!((0..rate).all(|_| playback.next(rate) == Some([0.25; 2])));
            assert_eq!(playback.next(rate), None);

            let mut playback = Playback::new(play);
            assert!((0..48_000).all(|_| {
                playback
                    .next(48_000)
                    .is_some_and(|[sample, _]| sample > 0.0)
            }));
            assert_eq!(playback.next(48_000), None);
        }
    }

    #[test]
    fn buffer_interruptions_keep_stream_open_and_invalidation_reconnects() {
        let failed = Arc::new(AtomicBool::new(false));
        let meter = Arc::new(Meter::default());
        let (send, receive) = mpsc::channel();
        let events: EventSink = Arc::new(move |event| send.send(event).unwrap());
        let mut handle_error = error_handler(&failed, &meter, &events);

        handle_error(ErrorKind::Xrun.into());
        handle_error(ErrorKind::Xrun.into());
        assert_eq!(meter.device_xruns.load(Relaxed), 2);
        assert!(!failed.load(Relaxed));
        assert!(matches!(receive.try_recv(), Err(mpsc::TryRecvError::Empty)));

        handle_error(ErrorKind::StreamInvalidated.into());
        handle_error(ErrorKind::DeviceNotAvailable.into());
        assert!(failed.load(Relaxed));
        assert!(
            matches!(receive.try_recv(), Ok(Event::Error(Failure::Reconnect, message)) if message.contains("Reconnecting"))
        );
        assert!(matches!(receive.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }

    #[test]
    fn stopping_one_request_preserves_the_other_lane_and_newer_requests() {
        let (decode, _) = mpsc::sync_channel(1);
        let player = Player {
            decode,
            params: Arc::default(),
        };
        player.params.clip_generation.store(5, Relaxed);
        player.params.preview_generation.store(8, Relaxed);
        player.stop_clip(false, 4);
        assert!(player.is_current(false, 5));
        player.stop_clip(false, 5);
        assert!(!player.is_current(false, 5));
        assert!(player.is_current(true, 8));
    }

    #[test]
    fn combines_voice_and_clip_without_clipping() {
        assert!((mix(0.30, 0.40, 0.75) - 0.60).abs() < 0.00001);
        assert!((mix(-0.20, 0.40, 0.0) + 0.20).abs() < 0.00001);
        assert!(mix(0.90, 0.90, 1.5) < 1.0);
        assert!(mix(-0.90, -0.90, 1.5) > -1.0);
    }

    #[test]
    fn accepts_pcm_wav_for_offline_clips() {
        let data = b"RIFF\x28\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\xbb\0\0\0\x77\x01\0\x02\0\x10\0data\x04\0\0\0\0\x20\0\xe0";
        let sound = decode_audio(Box::new(Cursor::new(data)), "wav").unwrap();
        assert_eq!(sound.rate, 48000);
        assert_eq!(sound.frames, vec![[0.25; 2], [-0.25; 2]]);
    }
}
