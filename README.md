<img src="assets/app.png" width="60" height="60" alt="">

# Meme Soundboard

Play clips through your microphone in games and voice chat. Meme Soundboard is a Windows 11 desktop app written in Rust. It keeps your library on your PC and lets you preview clips through headphones.

**[Download for Windows 11 (x64)](https://github.com/a-nahvi/meme-soundboard/releases/latest/download/MemeSoundboard-Setup-x64.exe)** · [Release notes](https://github.com/a-nahvi/meme-soundboard/releases) · [Report a bug](https://github.com/a-nahvi/meme-soundboard/issues/new/choose)

[![Windows checks](https://github.com/a-nahvi/meme-soundboard/actions/workflows/windows.yml/badge.svg?branch=main)](https://github.com/a-nahvi/meme-soundboard/actions/workflows/windows.yml)

## Quick start

1. Run the Windows installer. The default location is `%LOCALAPPDATA%\Programs\Meme Soundboard`.
2. Open **Library** and select **Import audio**, or use the folder button and drop your clips there.
3. Select **Play** to use the current audio route, or select the headphones icon to preview a clip locally.

For speakers or headphones, enable **Local playback only** in **Audio devices**.

### Send clips to voice chat

1. Select **Install Soundboard Microphone** during setup, or use an existing virtual audio device such as VB-CABLE. The optional setup installs the original Virtual Audio Cable Lite package for personal, noncommercial use. [Virtual Audio Cable](https://vac.muzychenko.net/en/download.htm)
2. In **Audio devices**, select your physical microphone and headphones, turn off **Local playback only**, and choose the virtual cable as **Voice output**. Refresh the device list if you just installed it.
3. In the game or chat app, choose the corresponding recording device as its microphone. With VAC Lite, it is typically **Line 1 (Virtual Audio Cable)**.
4. Use **Send test to chat** and the destination app’s microphone test to confirm the route.

For destinations using voice activity, clips can play while you talk. For destinations using Push-to-Talk, hold their talk key while the clip plays.

## Library and controls

- Import MP3, WAV, Ogg Vorbis, FLAC, and M4A/AAC files up to 24 MiB and two minutes each. The library detects files added to the clips folder.
- Assign global shortcuts to individual clips. Set a separate Stop shortcut in **Shortcuts**. The default is `Ctrl+Alt+0`.
- Seek from the progress bar. Use **Trim** to preview and save a range, then use **Revert** to restore full playback. The original audio stays in the library.
- Use **Mix** to set separate clip and microphone levels and configure the optional microphone gate.
- Keep Soundboard in the tray for shortcuts while you play. **Appearance** controls Close to tray and the System, Light, and Dark themes.

## Updates and saved data

Open **Updates** from the sidebar and select **Check for updates**. The app reads the latest stable public Release directly from GitHub and checks the installer’s SHA-256 before opening Setup.

Setup closes the running copy from the selected installation folder and updates it in place. The `data` folder beside the executable holds clips, saved trims, favorites, shortcuts, settings, and diagnostics.

## Build on Windows

Install the stable Rust MSVC toolchain, Visual Studio Build Tools with **Desktop development with C++**, and the Windows SDK.

```powershell
cargo run --locked
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

To build the installer with Inno Setup, run the following command. The output is `dist\MemeSoundboard-Setup-x64.exe`.

```powershell
.\build-installer.ps1 -Iscc "C:\path\to\ISCC.exe"
```

Builds use the GitHub `origin` as their update source. Source archives default to this project’s repository. Set `MEME_SOUNDBOARD_GITHUB_REPOSITORY` to an `owner/repository` pair to override it. For a private update source, set `MEME_SOUNDBOARD_PRIVATE_UPDATES=1` before building and sign in with Git for Windows or GitHub CLI.

Publish a stable `v`-prefixed tag and attach `MemeSoundboard-Setup-x64.exe` to its GitHub Release. Test an in-place upgrade from the previous version before release.

<details>
<summary>Build the installer with Iconly</summary>

The official installer embeds rendered Iconly frames. With a licensed Iconly Local archive and Python with Pillow, generate those frames locally and build with the same artwork. The generated atlas and frame index live in the local `assets/iconly` directory.

```powershell
python .\tools\prepare-iconly-atlas.py "C:\path\to\iconly-local\library"
cargo run --locked --features local-iconly
.\build-installer.ps1 -WithIconly -Iscc "C:\path\to\ISCC.exe"
```

</details>

## Support and license

For audio or shortcut issues, include the app version, Windows version, playback mode, reproduction steps, and relevant lines from `data\soundboard.log`. [Report an issue](https://github.com/a-nahvi/meme-soundboard/issues/new/choose)

The project source code is licensed under [MIT](LICENSE). See [third-party notices](THIRD_PARTY_NOTICES.md) for the bundled driver and icon licenses.
