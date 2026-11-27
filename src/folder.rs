use crate::{
    audio,
    storage::{AUDIO_EXTENSIONS, Clip, MAX_CLIP_BYTES, Store, file_key},
};
use anyhow::{Context, Result, ensure};
use notify::{EventKind, RecursiveMode, Watcher};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, Metadata, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, SystemTime},
};
use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

pub struct Snapshot {
    pub present: HashSet<String>,
    pub new: Vec<Clip>,
    pub failures: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp {
    size: u64,
    modified: Option<SystemTime>,
}

impl From<&Metadata> for Stamp {
    fn from(metadata: &Metadata) -> Self {
        Self {
            size: metadata.len(),
            modified: metadata.modified().ok(),
        }
    }
}

pub struct FolderSync {
    changes: SyncSender<()>,
    known: Arc<Mutex<HashSet<String>>>,
    done: Arc<AtomicBool>,
    pub events: Receiver<Snapshot>,
}

impl FolderSync {
    pub fn start(store: Store, clips: &[Clip], wake: impl Fn() + Send + 'static) -> Self {
        let (changes, dirty) = mpsc::sync_channel(1);
        let (send, events) = mpsc::channel();
        let known = Arc::new(Mutex::new(keys(clips)));
        let done = Arc::new(AtomicBool::new(false));
        let scanner_known = known.clone();
        let scanner_done = done.clone();
        let fs_changes = changes.clone();

        thread::spawn(move || {
            let folder = store.clips_dir();
            let changed_folder = folder.clone();
            let watch_failed = Arc::new(AtomicBool::new(false));
            let report_failed = watch_failed.clone();
            let watcher =
                notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                    let dirty = match result {
                        Ok(event) => {
                            event.need_rescan()
                                || (!matches!(event.kind, EventKind::Access(_))
                                    && event
                                        .paths
                                        .iter()
                                        .any(|path| relevant(path, &changed_folder)))
                        }
                        Err(_) => {
                            report_failed.store(true, Relaxed);
                            true
                        }
                    };
                    if dirty {
                        let _ = fs_changes.try_send(());
                    }
                })
                .and_then(|mut watcher| {
                    // Watching the parent also detects removal or recreation of the clips folder.
                    watcher.watch(&store.root, RecursiveMode::Recursive)?;
                    Ok(watcher)
                });
            if let Err(err) = &watcher {
                store.log(&format!(
                    "Folder notifications unavailable. Checking periodically. {err}"
                ));
            }

            let mut checked = HashMap::new();
            let mut directory_error = None;
            while !scanner_done.load(Relaxed) {
                let known = scanner_known
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .clone();
                let retry = match scan(&folder, &known, &mut checked) {
                    Ok((snapshot, busy)) => {
                        directory_error = None;
                        if send.send(snapshot).is_err() {
                            break;
                        }
                        wake();
                        busy
                    }
                    Err(err) => {
                        if directory_error.as_ref() != Some(&err.to_string()) {
                            store.log(&format!("Couldn't scan clips folder. {err}"));
                            directory_error = Some(err.to_string());
                        }
                        true
                    }
                };
                if scanner_done.load(Relaxed) {
                    break;
                }
                let periodic = if retry {
                    Some(Duration::from_millis(450))
                } else if watcher.is_err() || watch_failed.load(Relaxed) {
                    Some(Duration::from_secs(3))
                } else {
                    None
                };
                let changed = match periodic {
                    Some(delay) => match dirty.recv_timeout(delay) {
                        Ok(()) => true,
                        Err(mpsc::RecvTimeoutError::Timeout) => false,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    },
                    None => {
                        if dirty.recv().is_err() {
                            break;
                        }
                        true
                    }
                };
                if changed && !scanner_done.load(Relaxed) {
                    thread::sleep(Duration::from_millis(220));
                    while dirty.try_recv().is_ok() {}
                }
            }
        });
        Self {
            changes,
            known,
            done,
            events,
        }
    }

    pub fn set_known(&self, clips: &[Clip]) {
        *self.known.lock().unwrap_or_else(|err| err.into_inner()) = keys(clips);
    }

    pub fn refresh(&self) {
        let _ = self.changes.try_send(());
    }
}

impl Drop for FolderSync {
    fn drop(&mut self) {
        self.done.store(true, Relaxed);
        self.refresh();
    }
}

fn keys(clips: &[Clip]) -> HashSet<String> {
    clips.iter().map(|clip| file_key(&clip.file)).collect()
}

fn relevant(path: &Path, folder: &Path) -> bool {
    path == folder || (path.parent() == Some(folder) && audio_extension(path).is_some())
}

fn audio_extension(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    AUDIO_EXTENSIONS
        .contains(&extension.as_str())
        .then_some(extension)
}

fn scan(
    folder: &Path,
    known: &HashSet<String>,
    checked: &mut HashMap<String, (Stamp, Option<Clip>)>,
) -> std::io::Result<(Snapshot, bool)> {
    let mut snapshot = Snapshot {
        present: HashSet::new(),
        new: Vec::new(),
        failures: Vec::new(),
    };
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            checked.clear();
            return Ok((snapshot, false));
        }
        Err(err) => return Err(err),
    };
    let mut busy = false;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let Some(extension) = audio_extension(&path) else {
            continue;
        };
        if !entry.file_type()?.is_file() {
            continue;
        }
        let Some(file) = entry.file_name().into_string().ok() else {
            continue;
        };
        let key = file_key(&file);
        snapshot.present.insert(key.clone());
        if known.contains(&key) {
            continue;
        }

        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        let stamp = Stamp::from(&meta);
        if let Some((_, previous)) = checked.get(&key).filter(|(last, _)| last == &stamp) {
            snapshot.new.extend(previous.iter().cloned());
            continue;
        }
        // A file still held open for writing by Explorer will fail the read-only sharing check.
        let input = match OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
        {
            Ok(file) => file,
            Err(err) if matches!(err.raw_os_error(), Some(32 | 33)) => {
                busy = true;
                continue;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => {
                snapshot.failures.push(format!("{file}: {err}"));
                checked.insert(key, (stamp, None));
                continue;
            }
        };
        let clip = match validate(&path, &meta, input, &extension) {
            Ok(()) => Some(Clip::from_folder(file)),
            Err(err) => {
                snapshot.failures.push(format!("{file}: {err:#}"));
                None
            }
        };
        snapshot.new.extend(clip.iter().cloned());
        checked.insert(key, (stamp, clip));
    }
    checked.retain(|name, _| snapshot.present.contains(name) && !known.contains(name));
    snapshot.new.sort_by_cached_key(|clip| file_key(&clip.file));
    Ok((snapshot, busy))
}

fn validate(path: &Path, meta: &Metadata, file: fs::File, extension: &str) -> Result<()> {
    ensure!(
        meta.len() > 0 && meta.len() <= MAX_CLIP_BYTES,
        "Audio files must be between 1 byte and 24 MB."
    );
    audio::validate_file(file, extension)
        .with_context(|| format!("Couldn't decode {}", path.display()))
}
