#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod folder;
mod releases;
mod shortcuts;
mod storage;
mod tray;
mod ui;
mod update;

use anyhow::{Context, Result};
use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::{
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{ICON_BIG, LoadIconW, SendMessageW, WM_SETICON},
};

fn main() {
    let mut args = std::env::args_os().skip(1);
    let mode = args.next();
    if mode.as_deref() == Some(std::ffi::OsStr::new("--check-soundboard-microphone")) {
        let available = args.next().is_none() && audio::soundboard_microphone_available();
        std::process::exit(i32::from(!available));
    }
    if mode.as_deref() == Some(std::ffi::OsStr::new("--prepare-update")) {
        let completed = args
            .next()
            .filter(|_| args.next().is_none())
            .is_some_and(|executable| {
                update::close_running_install(std::path::Path::new(&executable)).is_ok()
            });
        std::process::exit(i32::from(!completed));
    }
    let background = mode.as_deref() == Some(std::ffi::OsStr::new("--background"));
    if let Err(error) = run(background) {
        eprintln!("{error:#}");
        rfd::MessageDialog::new()
            .set_title("Meme Soundboard")
            .set_description(format!("{error:#}"))
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
}

fn run(background: bool) -> Result<()> {
    let exe = std::env::current_exe().context("Can't locate the installation folder")?;
    let directory = exe
        .parent()
        .context("Can't locate the installation folder")?;
    let store = storage::Store::open(directory.join("data"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    let _library_lock = loop {
        if let Some(lock) = store.try_lock()? {
            break lock;
        }
        if background || update::show_running_install(&exe)? {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            anyhow::bail!("Soundboard is running. Open it from the Windows tray.");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Meme Soundboard")
            .with_decorations(false)
            .with_inner_size([980.0, 680.0])
            .with_min_inner_size([690.0, 480.0])
            .with_visible(!background)
            .with_active(!background)
            .with_icon(window_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "Meme Soundboard",
        options,
        Box::new(move |context| {
            update::listen_for_requests(context);
            let app = ui::App::new(&context.egui_ctx, store, background)?;
            set_taskbar_icon(context);
            Ok(Box::new(app))
        }),
    )
    .map_err(|err| anyhow::anyhow!("{err}"))
}

fn set_taskbar_icon(context: &eframe::CreationContext<'_>) {
    const APP_ICON_RESOURCE: u16 = 1;
    let Ok(handle) = context.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(window) = handle.as_raw() else {
        return;
    };
    // Load the executable's shared icon so Windows can use the same art as the installer shortcut.
    unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        if !module.is_null() {
            let icon = LoadIconW(
                module,
                std::ptr::without_provenance(APP_ICON_RESOURCE.into()),
            );
            if !icon.is_null() {
                SendMessageW(
                    window.hwnd.get() as _,
                    WM_SETICON,
                    ICON_BIG as usize,
                    icon as isize,
                );
            }
        }
    }
}

fn window_icon() -> egui::IconData {
    static LOGO: std::sync::OnceLock<egui::IconData> = std::sync::OnceLock::new();
    LOGO.get_or_init(|| {
        eframe::icon_data::from_png_bytes(include_bytes!("../assets/app.png"))
            .expect("The embedded soundboard logo is a PNG")
    })
    .clone()
}
