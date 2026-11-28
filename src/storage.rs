use crate::language::Language;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    ffi::OsStr,
    fs::{self, OpenOptions, TryLockError},
    io::{Read, Write},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

pub const MAX_CLIP_BYTES: u64 = 24 * 1024 * 1024;
pub const MAX_CLIP_SECONDS: usize = 120;
pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "wav", "ogg", "flac", "m4a", "aac"];
pub const LIBRARY_FILE: &str = "library.json";
const RENAME_JOURNAL: &str = "pending-rename.json";

#[derive(Clone, Copy, Default, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    pub microphone: Option<String>,
    pub cable: Option<String>,
    pub headphones: Option<String>,
    pub mic_volume: f32,
    pub clip_volume: f32,
    pub threshold_db: f32,
    pub voice_gate: bool,
    pub mic_muted: bool,
    pub monitor_clips: bool,
    pub local_playback_only: bool,
    pub stop_key: String,
    pub appearance: Appearance,
    pub language: Language,
    pub close_to_tray: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            microphone: None,
            cable: None,
            headphones: None,
            mic_volume: 1.0,
            clip_volume: 0.70,
            threshold_db: -43.0,
            voice_gate: false,
            mic_muted: false,
            monitor_clips: true,
            local_playback_only: false,
            stop_key: "Ctrl+Alt+0".into(),
            appearance: Appearance::System,
            language: Language::default(),
            close_to_tray: true,
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Clip {
    pub id: String,
    pub name: String,
    pub file: String,
    pub source: String,
    pub source_page: Option<String>,
    pub credit: Option<String>,
    pub hotkey: Option<String>,
    pub favorite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim: Option<ClipTrim>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ClipTrim {
    pub start_ms: u32,
    pub end_ms: u32,
}

impl ClipTrim {
    pub fn full(duration_ms: u32) -> Self {
        Self {
            start_ms: 0,
            end_ms: duration_ms,
        }
    }

    pub fn duration_ms(self) -> u32 {
        self.end_ms.saturating_sub(self.start_ms)
    }

    pub fn bounded(self, duration_ms: u32) -> Option<Self> {
        let end_ms = self.end_ms.min(duration_ms);
        (self.start_ms < end_ms).then_some(Self { end_ms, ..self })
    }
}

impl Clip {
    pub fn from_folder(file: String) -> Self {
        let name = Path::new(&file)
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or("Untitled clip")
            .trim()
            .chars()
            .take(160)
            .collect();
        Self::local(unique_id(), file, name)
    }

    fn local(id: String, file: String, name: String) -> Self {
        Self {
            id,
            file,
            name,
            source: "Local file".into(),
            source_page: None,
            credit: None,
            hotkey: None,
            favorite: false,
            trim: None,
        }
    }

    pub fn renamed(&self, name: &str) -> Result<Self> {
        let extension = Path::new(&self.file)
            .extension()
            .and_then(OsStr::to_str)
            .context("This audio file has no extension.")?;
        let mut name = name.trim();
        if let Some((stem, suffix)) = name.rsplit_once('.')
            && suffix.eq_ignore_ascii_case(extension)
        {
            name = stem;
        }
        ensure_filename(name)?;
        let mut clip = self.clone();
        clip.name = name.to_owned();
        clip.file = format!("{name}.{extension}");
        Ok(clip)
    }
}

#[derive(Deserialize, Serialize)]
struct PendingRename {
    id: String,
    from: String,
    to: String,
    name: String,
}

#[derive(Clone)]
pub struct Store {
    pub root: PathBuf,
}

impl Store {
    pub fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(root.join("clips")).with_context(|| {
            format!(
                "Can't write to {}. Choose a writable install folder.",
                root.display()
            )
        })?;
        Ok(Self { root })
    }

    pub fn read<T: DeserializeOwned + Default>(&self, filename: &str) -> Result<T> {
        match fs::read(self.root.join(filename)) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).with_context(|| format!("Can't read {filename}"))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
            Err(err) => Err(err).with_context(|| format!("Can't open {filename}")),
        }
    }

    pub fn save(&self, filename: &str, value: &impl Serialize) -> Result<()> {
        write_atomic(
            &self.root.join(filename),
            &serde_json::to_vec_pretty(value)?,
        )
    }

    pub fn try_lock(&self) -> Result<Option<fs::File>> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("instance.lock"))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(err)) => Err(err).context("Can't lock the Soundboard library"),
        }
    }

    pub fn clip_path(&self, file: &str) -> Result<PathBuf> {
        let path = Path::new(file);
        if path.file_name() != Some(path.as_os_str()) || file.contains(':') {
            bail!("Invalid clip filename");
        }
        Ok(self.clips_dir().join(path))
    }

    pub fn clips_dir(&self) -> PathBuf {
        self.root.join("clips")
    }

    pub fn read_library(&self) -> Result<Vec<Clip>> {
        let mut clips: Vec<Clip> = self.read(LIBRARY_FILE)?;
        let Some(rename): Option<PendingRename> = self.read(RENAME_JOURNAL)? else {
            return Ok(clips);
        };
        if let Some(clip) = clips
            .iter_mut()
            .find(|clip| clip.id == rename.id && clip.file == rename.from)
        {
            // Exact spelling matters when recovering a case-only rename on NTFS.
            let files: Vec<_> = fs::read_dir(self.clips_dir())?
                .collect::<std::io::Result<Vec<_>>>()?
                .into_iter()
                .map(|file| file.file_name())
                .collect();
            if !files.contains(&rename.from.as_str().into())
                && files.contains(&rename.to.as_str().into())
            {
                self.clip_path(&rename.to)?;
                clip.file = rename.to;
                clip.name = rename.name;
                self.save(LIBRARY_FILE, &clips)?;
            }
        }
        self.finish_rename();
        Ok(clips)
    }

    pub fn rename_clip_file(
        &self,
        old: &Clip,
        new: &Clip,
        save: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        if old.file == new.file {
            return save();
        }
        let from = self.clip_path(&old.file)?;
        let to = self.clip_path(&new.file)?;
        if self.root.join(RENAME_JOURNAL).exists() {
            bail!("Reopen Soundboard to finish the previous file rename.");
        }
        let rename = PendingRename {
            id: old.id.clone(),
            from: old.file.clone(),
            to: new.file.clone(),
            name: new.name.clone(),
        };
        self.save(RENAME_JOURNAL, &Some(rename))?;
        if let Err(err) = move_file(&from, &to, MOVEFILE_WRITE_THROUGH) {
            self.finish_rename();
            return match err.raw_os_error() {
                Some(80 | 183) => Err(anyhow::anyhow!("A file with that name already exists.")),
                Some(2 | 3) => Err(anyhow::anyhow!(
                    "The audio file is missing from the clips folder."
                )),
                _ => Err(err).context("Couldn't rename the audio file"),
            };
        }
        if let Err(err) = save() {
            if let Err(rollback) = move_file(&to, &from, MOVEFILE_WRITE_THROUGH) {
                self.log(&format!(
                    "Couldn't roll back file rename. {rollback}. {err:#}"
                ));
                return Err(err).context("Reopen Soundboard to recover the file rename.");
            }
            self.finish_rename();
            return Err(err);
        }
        self.finish_rename();
        Ok(())
    }

    fn finish_rename(&self) {
        if let Err(err) = fs::remove_file(self.root.join(RENAME_JOURNAL))
            && err.kind() != std::io::ErrorKind::NotFound
        {
            self.log(&format!("Couldn't clear the file-rename journal: {err}"));
        }
    }

    pub fn import(
        &self,
        path: &Path,
        validate: impl FnOnce(&[u8], &str) -> Result<()>,
    ) -> Result<Clip> {
        let extension = path
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        if !AUDIO_EXTENSIONS.contains(&extension.as_str()) {
            bail!("Choose an MP3, WAV, OGG, FLAC, M4A or AAC clip");
        }
        let name = path
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or("Untitled clip");
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(MAX_CLIP_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.is_empty() || bytes.len() as u64 > MAX_CLIP_BYTES {
            bail!("The clip must be between 1 byte and 24 MB");
        }
        validate(&bytes, &extension).context("Can't decode this audio clip")?;
        let id = unique_id();
        let file = format!("{id}.{extension}");
        write_atomic(&self.clip_path(&file)?, &bytes)?;
        Ok(Clip::local(
            id,
            file,
            name.trim().chars().take(160).collect(),
        ))
    }

    pub fn remove_clip_file(&self, clip: &Clip) -> Result<()> {
        match fs::remove_file(self.clip_path(&clip.file)?) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err).context("Can't remove the clip file"),
        }
    }

    pub fn delete_clip(&self, clip: &Clip, save: impl FnOnce() -> Result<()>) -> Result<()> {
        let from = self.clip_path(&clip.file)?;
        let staged = from.with_extension(format!("{}.tmp", unique_id()));
        match move_file(&from, &staged, MOVEFILE_WRITE_THROUGH) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return save(),
            Err(err) => return Err(err).context("Couldn't remove the audio file"),
        }
        if let Err(err) = save() {
            if let Err(rollback) = move_file(&staged, &from, MOVEFILE_WRITE_THROUGH) {
                self.log(&format!(
                    "Couldn't restore the staged clip {}. {rollback}",
                    staged.display()
                ));
            }
            return Err(err);
        }
        if let Err(err) = fs::remove_file(&staged) {
            self.log(&format!(
                "Couldn't clear the staged clip {}. {err}",
                staged.display()
            ));
        }
        Ok(())
    }

    pub fn log(&self, message: &str) {
        let path = self.root.join("soundboard.log");
        if fs::metadata(&path).is_ok_and(|info| info.len() >= 512 * 1024) {
            let previous = self.root.join("soundboard.previous.log");
            let _ = fs::remove_file(&previous);
            let _ = fs::rename(&path, previous);
        }
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let line = message.replace(['\r', '\n'], " ");
            let _ = writeln!(file, "{seconds} {line}");
        }
    }
}

pub fn file_key(name: &str) -> String {
    name.to_lowercase()
}

fn ensure_filename(name: &str) -> Result<()> {
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    let reserved = stem.to_ascii_uppercase();
    let port = reserved
        .strip_prefix("COM")
        .or_else(|| reserved.strip_prefix("LPT"));
    if name.is_empty()
        || name.ends_with(['.', ' '])
        || name.encode_utf16().count() > 160
        || name
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
        || matches!(
            reserved.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        )
        || port.is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    {
        bail!("Enter a valid Windows filename of up to 160 characters.");
    }
    Ok(())
}

fn unique_id() -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{now:x}-{:x}-{counter:x}", std::process::id())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = path.with_extension(format!("{}.tmp", unique_id()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        move_file(
            &temp,
            path,
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result.with_context(|| format!("Can't save {}", path.display()))
}

fn move_file(from: &Path, to: &Path, flags: u32) -> std::io::Result<()> {
    let from: Vec<_> = from.as_os_str().encode_wide().chain([0]).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain([0]).collect();
    // The UTF-16 paths stay alive during this call. Both paths are on the same volume.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), flags) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_settings_default_to_english_and_can_save_persian() {
        let mut settings: Settings =
            serde_json::from_str(r#"{"mic_muted":true,"stop_key":"F8"}"#).unwrap();
        assert_eq!(settings.language, Language::English);
        settings.language = Language::Persian;

        let saved = serde_json::to_value(&settings).unwrap();
        assert_eq!(saved["language"], "persian");
        let reloaded: Settings = serde_json::from_value(saved).unwrap();
        assert_eq!(reloaded.language, Language::Persian);
        assert!(reloaded.mic_muted);
        assert_eq!(reloaded.stop_key, "F8");
    }
}
