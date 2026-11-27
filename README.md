# Meme Soundboard

An English-language soundboard for Windows 11 with a local clip library. It mixes your physical microphone and clips into a virtual recording device for games and voice apps. Use headphones to preview clips privately.

## Install and set up

1. Run `MemeSoundboard-Setup-x64.exe`. Pick a folder your Windows account can write to. The default is `%LOCALAPPDATA%\Programs\Meme Soundboard`.
2. For voice chat, select **Install Soundboard Microphone** during interactive setup. Setup includes the original [Virtual Audio Cable Lite 4.71 package](https://vac.muzychenko.net/en/download.htm) by Eugene Muzychenko and opens its installer. Approve Windows administrator access and review the vendor's license. The Lite package is for personal home use.
3. Open **Audio devices** and select your physical microphone and headphone output. Turn off **Local playback only**, then set **Voice output** to **Soundboard Microphone** or leave it on **Detect virtual output**. Use the refresh icon if you just installed the driver.
4. In the game or voice app, select the recording device shown in **Audio devices**, usually **Line 1 (Virtual Audio Cable)**. If the game follows the Windows communications input, choose that recording device as the communications input and select your physical microphone explicitly in Soundboard. Check your usual speakers or headphones in Windows Sound settings after installing the driver.
5. Run **Preview test** to check headphones and **Send test to chat** to check the virtual input. Use the destination app's microphone test to check the clip and your voice together.

With **open mic** or **voice activity** enabled in the destination app, you can play a clip mid-sentence. With **Push-to-Talk** enabled there, hold the app's own talk key for the whole clip. If a destination's noise filter swallows sound effects, check its microphone sensitivity and suppression settings.

For headphone or speaker playback, turn on **Local playback only** in **Audio devices**. Existing VB-CABLE and other virtual audio devices can also be selected as **Voice output**. Select the corresponding recording endpoint in the game.

## Use

- **Library** supports MP3, WAV, OGG Vorbis, FLAC, and M4A/AAC clips up to 24 MB and two minutes. Use **Import audio**, or click the folder icon and copy clips directly into the folder. Soundboard picks up new files automatically.
- Use the play icon and the Stop square to play or stop a clip. The headphones icon previews it locally. Click or drag the progress bar to seek within the current clip.
- Click the scissors icon to set the start and end. Soundboard previews the selection on headphones or speakers when you finish editing a boundary. **Preview** repeats it from the selected start. **Save trim** applies the selection to Library playback and shortcuts and previews the saved clip locally. **Revert** restores the full clip. The library keeps the original audio.
- Click **Set shortcut** next to any clip, press a key or combination such as `F8` or `Alt+1`, then select **Save shortcut**. Shortcuts work on the desktop and in other apps while Soundboard is running. Saved digit shortcuts also accept the corresponding number-pad key with Num Lock on when that key is available. Type `Numpad1` using **Type manually** to assign it specifically.
- **Stop** stops playback. Its default shortcut is `Ctrl+Alt+0`, which you can change in **Shortcuts**. If Windows or another clip already uses a combination, choose a different one.
- **Mix** adjusts mic and clip volumes individually. The optional microphone gate uses a configurable signal threshold and release time. It leaves clip playback active whether or not you are speaking.
- The close button sends Soundboard to the Windows tray by default. Open it again from the taskbar or the tray. Use **Quit** from the tray menu to exit. Enable **Start Soundboard in the tray when I sign in to Windows** in Setup to have shortcuts ready after sign-in. You can manage this in **Windows startup settings** from the **Shortcuts** tab.
- In **Appearance**, you can turn off **Close to tray** or set the theme to **System**, **Light**, or **Dark**. **System** follows the Windows app theme.

## Upgrade

Run the new setup over the existing installation. Setup closes the copy running from that folder, including when it's in the tray, then updates it in place. The `data` folder beside it holds your clips, favorites, shortcuts, settings, and diagnostic log.

Deliver **dist/MemeSoundboard-Setup-x64.exe** with every release. Keep the existing Inno Setup app ID and previous-directory detection so Setup can upgrade the installed copy. Before handing off the installer, verify that it closes a running previous version, updates it in its existing folder, and preserves the data folder.

Open **Updates** from the sidebar to see your installed version and check for new stable releases. In-app checking and downloading use an installed GitHub CLI signed in with access to the private repository. You can also select **Open GitHub releases** and use your existing browser sign-in. Soundboard verifies the GitHub asset's size and SHA-256 digest before opening Setup in your existing installation folder.

Build from a checkout whose `origin` points to the GitHub repository. To configure a build from a source archive, set `MEME_SOUNDBOARD_GITHUB_REPOSITORY` to the `owner/repository` slug before building. Release stable versions with a `v`-prefixed version tag and attach `dist/MemeSoundboard-Setup-x64.exe` to the GitHub Release. The update checker ignores prereleases.

## Develop

Use the stable Rust MSVC toolchain, Visual Studio Build Tools with the Desktop development with C++ workload, and the Windows SDK.

```powershell
cargo run --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
.\build-installer.ps1 -Iscc 'C:\path\to\ISCC.exe'
```

The installer script uses Inno Setup. The x64 build links the Visual C++ runtime statically. CPAL opens Windows WASAPI endpoints, Symphonia decodes audio outside the audio callbacks, and egui renders the desktop window with [shadcn-egui](https://github.com/oetiker/shadcn-egui) components and [Lucide](https://lucide.dev/) icons. Imports are decoded on a worker. Device, playback, and import events wake the UI as needed.

`src/audio.rs` handles device selection, mixing, playback, and resampling. `src/storage.rs` and `src/folder.rs` manage local data and folder notifications. `src/shortcuts.rs` processes global key presses on a Win32 message thread. `src/tray.rs` handles the tray menu. `src/ui/` contains the library, settings, shortcut recorder, and design tokens.

To reproduce an audio issue, include the chosen input and output devices, the destination app and voice mode, whether its mic test receives the test tone, and relevant lines from `data/soundboard.log`. The app does not save microphone recordings.
