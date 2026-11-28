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
- Choose **Language** in **Appearance** to switch between English (left to right) and Persian (right to left). The change applies immediately and is saved in your settings.

## Updates and saved data

Open **Updates** from the sidebar and select **Check for updates**. The app reads `meme-soundboard-update.json` from the latest public GitHub Release and downloads the installer from that version’s release. It verifies the file size and SHA-256 before opening Setup. [GitHub release downloads](https://docs.github.com/en/repositories/releasing-projects-on-github/linking-to-releases)

Setup closes the running copy from the selected installation folder and updates it in place. The `data` folder beside the executable holds clips, saved trims, favorites, shortcuts, settings, and diagnostics.

## Build on Windows

Install the stable Rust MSVC toolchain, Visual Studio Build Tools with **Desktop development with C++**, and the Windows SDK.

```powershell
cargo run --locked
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

To build the installer with Inno Setup, run the following command. It creates `MemeSoundboard-Setup-x64.exe` and `meme-soundboard-update.json` in `dist`.

```powershell
.\build-installer.ps1 -Iscc "C:\path\to\ISCC.exe"
```

Builds use the public Releases of their GitHub `origin` as their update source. Source archives default to this project’s repository. Set `MEME_SOUNDBOARD_GITHUB_REPOSITORY` to an `owner/repository` pair to override it.

<details>
<summary>Build the installer with Iconly</summary>

The official installer embeds rendered Iconly frames. With a licensed Iconly Local archive and Python with Pillow, generate those frames locally and build with the same artwork. The generated atlas and frame index live in the local `assets/iconly` directory.

```powershell
python .\tools\prepare-iconly-atlas.py "C:\path\to\iconly-local\library"
cargo run --locked --features local-iconly
.\build-installer.ps1 -WithIconly -Iscc "C:\path\to\ISCC.exe"
```

</details>

## Publish a release

GitHub Releases is the sole publication channel for new versions.

1. Set the version in `Cargo.toml`, `Cargo.lock`, `assets/app.rc`, and `installer.iss`. Build the official installer with `-WithIconly` and test an in-place upgrade from the previous release.
2. Push the release commit to `main`. Keep the licensed Iconly source local and the generated atlas in the ignored `assets/iconly/` directory.
3. Create a draft GitHub Release for that commit using a `v`-prefixed tag. Attach both `MemeSoundboard-Setup-x64.exe` and `meme-soundboard-update.json` from the same build. Publish the stable release as latest once both assets are ready.
4. Verify that the previous version finds the release in **Updates** and that the published download matches the installer.

## Support and license

For audio or shortcut issues, include the app version, Windows version, playback mode, reproduction steps, and relevant lines from `data\soundboard.log`. [Report an issue](https://github.com/a-nahvi/meme-soundboard/issues/new/choose)

The project source code is licensed under [MIT](LICENSE). See [third-party notices](THIRD_PARTY_NOTICES.md) for the bundled driver and icon licenses.
