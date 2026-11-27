use crate::{
    audio::{Player, Source},
    storage::ClipTrim,
};
use anyhow::{Context, Result, anyhow, bail};
use eframe::egui;
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
        mpsc::{self, Sender},
    },
    thread::{self, JoinHandle},
};
use windows_sys::Win32::{
    System::Threading::GetCurrentThreadId,
    UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_APP,
    },
};

#[derive(Clone)]
pub enum Action {
    Play {
        path: PathBuf,
        id: String,
        title: String,
        trim: Option<ClipTrim>,
    },
    Stop,
}

pub enum Event {
    Playing {
        id: String,
        title: String,
        generation: u64,
    },
    Stopped,
    Captured(String),
    Error(String),
}

#[derive(Clone)]
struct Binding {
    action: Action,
    text: String,
}

#[derive(Default)]
struct Actions {
    primary: HashMap<u32, Binding>,
    aliases: HashMap<u32, (u32, String)>,
}

type Chord = (Modifiers, Code);

pub struct Shortcuts {
    manager: HotKeyThread,
    registered: HashMap<String, HotKey>,
    aliases: HashMap<Chord, (HotKey, u32)>,
    actions: Arc<Mutex<Actions>>,
    capturing: Arc<AtomicBool>,
    activations: Arc<AtomicU64>,
}

impl Shortcuts {
    pub fn new(player: Player, updates: Sender<Event>, ctx: egui::Context) -> Result<Self> {
        let manager = HotKeyThread::start().context("Can't start global shortcuts")?;
        let actions = Arc::new(Mutex::new(Actions::default()));
        let bindings = actions.clone();
        let capturing = Arc::new(AtomicBool::new(false));
        let record = capturing.clone();
        let activations = Arc::new(AtomicU64::new(0));
        let triggered = activations.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state() != HotKeyState::Pressed {
                return;
            }
            let Some(binding) = bindings.lock().ok().and_then(|map| {
                let id = event.id();
                if let Some((primary, alias_text)) = map.aliases.get(&id) {
                    map.primary.get(primary).map(|binding| Binding {
                        action: binding.action.clone(),
                        text: alias_text.clone(),
                    })
                } else {
                    map.primary.get(&id).cloned()
                }
            }) else {
                return;
            };
            let update = if record.swap(false, Relaxed) && crate::update::owns_foreground() {
                Event::Captured(binding.text)
            } else {
                triggered.fetch_add(1, Relaxed);
                match binding.action {
                    Action::Play {
                        path,
                        id,
                        title,
                        trim,
                    } => match player.play(Source::File(path), false, trim) {
                        Ok(generation) => Event::Playing {
                            id,
                            title,
                            generation,
                        },
                        Err(err) => Event::Error(err.to_string()),
                    },
                    Action::Stop => {
                        player.stop();
                        Event::Stopped
                    }
                }
            };
            if updates.send(update).is_ok() {
                ctx.request_repaint();
            }
        }));
        Ok(Self {
            manager,
            actions,
            capturing,
            activations,
            registered: HashMap::new(),
            aliases: HashMap::new(),
        })
    }

    pub fn capture(&self, active: bool) {
        self.capturing.store(active, Relaxed);
    }

    pub fn is_registered(&self, id: &str) -> bool {
        self.registered.contains_key(id)
    }

    pub fn status(&self) -> (usize, u64) {
        (self.registered.len(), self.activations.load(Relaxed))
    }

    fn available_id(&self) -> Result<u32> {
        // RegisterHotKey reserves IDs above 0xBFFF for shared libraries.
        (1..=0xbfff)
            .find(|id| {
                self.registered.values().all(|key| key.id() != *id)
                    && self.aliases.values().all(|(key, _)| key.id() != *id)
            })
            .context("Too many registered shortcuts")
    }

    fn remove_alias(&mut self, chord: Chord) {
        if let Some((alias, _)) = self.aliases.remove(&chord) {
            let _ = self.manager.request(WorkerCommand::Unregister(alias));
            self.actions
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .aliases
                .remove(&alias.id());
        }
    }

    fn sync_number_pad(&mut self) {
        let wanted: HashMap<Chord, u32> = self
            .registered
            .values()
            .filter_map(|key| {
                let alias = (key.mods, number_pad_alias(key.key)?);
                if self.registered.values().any(|key| chord(*key) == alias) {
                    return None;
                }
                Some((alias, key.id()))
            })
            .collect();
        let stale: Vec<_> = self
            .aliases
            .iter()
            .filter_map(|(chord, (_, primary))| {
                (wanted.get(chord) != Some(primary)).then_some(*chord)
            })
            .collect();
        for chord in stale {
            self.remove_alias(chord);
        }
        for (chord, primary) in wanted {
            if self.aliases.contains_key(&chord) {
                continue;
            }
            let Ok(id) = self.available_id() else { break };
            let mut alias = HotKey::new(Some(chord.0), chord.1);
            alias.id = id;
            if self.manager.request(WorkerCommand::Register(alias)).is_ok() {
                self.aliases.insert(chord, (alias, primary));
                self.actions
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .aliases
                    .insert(alias.id(), (primary, alias.to_string()));
            }
        }
    }

    pub fn set(
        &mut self,
        id: &str,
        value: &str,
        action: Action,
        save: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let mut requested = if value.trim().is_empty() {
            None
        } else {
            Some(
                value
                    .trim()
                    .parse::<HotKey>()
                    .map_err(|_| anyhow!("That key combination is not supported."))?,
            )
        };
        let previous = self.registered.get(id).copied();
        let changed = previous.map(chord) != requested.map(chord);
        if !changed {
            requested = previous;
        } else if let Some(new) = &mut requested {
            if let Some(existing) = self
                .registered
                .values()
                .find(|key| chord(**key) == chord(*new))
            {
                let binding = self
                    .actions
                    .lock()
                    .ok()
                    .and_then(|map| map.primary.get(&existing.id()).cloned());
                match binding.map(|binding| binding.action) {
                    Some(Action::Play { title, .. }) => {
                        bail!("That shortcut already plays {title}.")
                    }
                    Some(Action::Stop) => bail!("That shortcut is assigned to Stop playback."),
                    None => bail!("That shortcut is already assigned in Soundboard."),
                }
            }
            new.id = self.available_id()?;
            self.remove_alias(chord(*new));
            if let Err(err) = self
                .manager
                .request(WorkerCommand::Register(*new))
                .context("Windows couldn't register that shortcut. Try another combination.")
            {
                self.sync_number_pad();
                return Err(err);
            }
        }
        if let Err(err) = save() {
            if changed && let Some(new) = requested {
                let _ = self.manager.request(WorkerCommand::Unregister(new));
                self.sync_number_pad();
            }
            return Err(err);
        }
        if changed && let Some(old) = previous {
            let _ = self.manager.request(WorkerCommand::Unregister(old));
        }
        let mut actions = self.actions.lock().unwrap_or_else(|err| err.into_inner());
        if changed && let Some(old) = previous {
            actions.primary.remove(&old.id());
        }
        if let Some(new) = requested {
            self.registered.insert(id.to_owned(), new);
            actions.primary.insert(
                new.id(),
                Binding {
                    action,
                    text: value.trim().to_owned(),
                },
            );
        } else {
            self.registered.remove(id);
        }
        drop(actions);
        if changed {
            self.sync_number_pad();
        }
        Ok(())
    }
}

fn chord(key: HotKey) -> Chord {
    (key.mods, key.key)
}

fn number_pad_alias(code: Code) -> Option<Code> {
    Some(match code {
        Code::Digit0 => Code::Numpad0,
        Code::Digit1 => Code::Numpad1,
        Code::Digit2 => Code::Numpad2,
        Code::Digit3 => Code::Numpad3,
        Code::Digit4 => Code::Numpad4,
        Code::Digit5 => Code::Numpad5,
        Code::Digit6 => Code::Numpad6,
        Code::Digit7 => Code::Numpad7,
        Code::Digit8 => Code::Numpad8,
        Code::Digit9 => Code::Numpad9,
        _ => return None,
    })
}

const WORKER_MESSAGE: u32 = WM_APP + 1;

enum WorkerCommand {
    Register(HotKey),
    Unregister(HotKey),
    Exit,
}

type CommandQueue = Arc<Mutex<VecDeque<(WorkerCommand, Sender<Result<()>>)>>>;

struct HotKeyThread {
    id: u32,
    queue: CommandQueue,
    handle: Option<JoinHandle<()>>,
}

impl HotKeyThread {
    fn start() -> Result<Self> {
        let queue: CommandQueue = Arc::new(Mutex::new(VecDeque::new()));
        let worker_queue = queue.clone();
        let (ready, started) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("soundboard-hotkeys".into())
            .spawn(move || {
                let manager = match GlobalHotKeyManager::new() {
                    Ok(manager) => manager,
                    Err(err) => {
                        let _ = ready.send(Err(err));
                        return;
                    }
                };
                let mut message = MSG::default();
                // Create the thread's Win32 message queue before reporting readiness.
                unsafe {
                    PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
                }
                if ready.send(Ok(unsafe { GetCurrentThreadId() })).is_err() {
                    return;
                }

                while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
                    if message.hwnd.is_null() && message.message == WORKER_MESSAGE {
                        let pending = std::mem::take(
                            &mut *worker_queue.lock().unwrap_or_else(|err| err.into_inner()),
                        );
                        for (command, reply) in pending {
                            let result = match command {
                                WorkerCommand::Register(key) => manager.register(key),
                                WorkerCommand::Unregister(key) => manager.unregister(key),
                                WorkerCommand::Exit => {
                                    let _ = reply.send(Ok(()));
                                    return;
                                }
                            };
                            let _ = reply.send(result.map_err(anyhow::Error::from));
                        }
                    } else {
                        unsafe {
                            DispatchMessageW(&message);
                        }
                    }
                }
                worker_queue
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .clear();
            })?;
        let id = started.recv().context("Shortcut thread didn't start")??;
        Ok(Self {
            id,
            queue,
            handle: Some(handle),
        })
    }

    fn request(&self, command: WorkerCommand) -> Result<()> {
        let (reply, completed) = mpsc::channel();
        let mut queue = self.queue.lock().unwrap_or_else(|err| err.into_inner());
        queue.push_back((command, reply));
        if unsafe { PostThreadMessageW(self.id, WORKER_MESSAGE, 0, 0) } == 0 {
            queue.pop_back();
            return Err(std::io::Error::last_os_error())
                .context("Couldn't contact the Windows shortcut thread");
        }
        drop(queue);
        completed
            .recv()
            .context("The Windows shortcut thread stopped")?
    }
}

impl Drop for HotKeyThread {
    fn drop(&mut self) {
        if self.request(WorkerCommand::Exit).is_ok()
            && let Some(handle) = self.handle.take()
        {
            let _ = handle.join();
        }
    }
}
