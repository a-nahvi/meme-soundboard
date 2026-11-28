use anyhow::{Context, Result, bail, ensure};
use semver::Version;
use serde::Deserialize;
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::Read,
    os::windows::{ffi::OsStringExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    ptr::{null, null_mut},
    time::{SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Security::Cryptography::{
        BCRYPT_HASH_HANDLE, BCRYPT_SHA256_ALG_HANDLE, BCryptCreateHash, BCryptDestroyHash,
        BCryptFinishHash, BCryptHashData,
    },
    System::{SystemInformation::GetSystemDirectoryW, Threading::CREATE_NO_WINDOW},
};

const INSTALLER: &str = "MemeSoundboard-Setup-x64.exe";
const MANIFEST: &str = "meme-soundboard-update.json";
const MANIFEST_LIMIT: u64 = 16 * 1024;
pub const REPOSITORY: &str = env!("MEME_SOUNDBOARD_GITHUB_REPOSITORY");
pub const INSTALLED_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Deserialize)]
struct UpdateManifest {
    version: String,
    size: u64,
    sha256: String,
}

#[derive(Clone)]
pub struct AvailableUpdate {
    pub version: Version,
    size: u64,
    sha256: String,
}

pub fn check(repository: &str) -> Result<Option<AvailableUpdate>> {
    check_for_version(repository, &Version::parse(INSTALLED_VERSION)?)
}

fn check_for_version(repository: &str, installed: &Version) -> Result<Option<AvailableUpdate>> {
    let url = format!("https://github.com/{repository}/releases/latest/download/{MANIFEST}");
    let response = download(&url, Stdio::piped(), MANIFEST_LIMIT)?;
    update_from_manifest(&response, installed)
}

fn update_from_manifest(bytes: &[u8], installed: &Version) -> Result<Option<AvailableUpdate>> {
    let manifest: UpdateManifest =
        serde_json::from_slice(bytes).context("Can't read the release update information.")?;
    let version = Version::parse(&manifest.version).context("The update version is invalid.")?;
    if !version.pre.is_empty() || !version.cmp_precedence(installed).is_gt() {
        return Ok(None);
    }
    ensure!(
        manifest.size != 0,
        "The installer size in the update information is invalid."
    );
    ensure!(
        manifest.sha256.len() == 64 && manifest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "The installer SHA-256 in the update information is invalid."
    );
    Ok(Some(AvailableUpdate {
        version,
        size: manifest.size,
        sha256: manifest.sha256,
    }))
}

pub fn download_and_install(repository: &str, update: &AvailableUpdate, data: &Path) -> Result<()> {
    let install_dir = data
        .parent()
        .context("Can't locate the installation folder.")?;
    let installer = download_installer(repository, update, data)?;

    // Inno Setup accepts a quoted /DIR value. This also covers custom install locations.
    let mut directory_arg = OsString::from("/DIR=\"");
    directory_arg.push(install_dir);
    directory_arg.push("\"");
    Command::new(installer)
        .raw_arg(directory_arg)
        .current_dir(install_dir)
        .spawn()
        .context("Can't open the update installer.")?;
    Ok(())
}

fn installer_url(repository: &str, version: &Version) -> String {
    format!("https://github.com/{repository}/releases/download/v{version}/{INSTALLER}")
}

fn download_installer(repository: &str, update: &AvailableUpdate, data: &Path) -> Result<PathBuf> {
    let directory = data.join("updates").join(update.version.to_string());
    fs::create_dir_all(&directory).context("Can't create the updates folder.")?;
    let installer = directory.join(INSTALLER);
    if !installer_matches(&installer, update) {
        let url = installer_url(repository, &update.version);
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let staging = directory.join(format!("download-{nonce}.part"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
            .context("Can't create the installer download file.")?;
        let downloaded = (|| {
            download(&url, Stdio::from(file), update.size)?;
            ensure!(
                installer_matches(&staging, update),
                "The installer did not match the size and SHA-256 in the update information."
            );
            fs::rename(&staging, &installer).context("Can't save the verified installer.")
        })();
        if downloaded.is_err() {
            let _ = fs::remove_file(&staging);
        }
        downloaded?;
    }

    Ok(installer)
}

fn installer_matches(path: &Path, update: &AvailableUpdate) -> bool {
    let verify = || -> Result<bool> {
        let file = File::open(path)?;
        if file.metadata()?.len() != update.size {
            return Ok(false);
        }
        let (length, digest) = sha256(file)?;
        Ok(length == update.size && digest.eq_ignore_ascii_case(&update.sha256))
    };
    verify().unwrap_or(false)
}

struct HashHandle(BCRYPT_HASH_HANDLE);

impl Drop for HashHandle {
    fn drop(&mut self) {
        // This owning handle was returned by BCryptCreateHash.
        unsafe { BCryptDestroyHash(self.0) };
    }
}

fn sha256(mut source: impl Read) -> Result<(u64, String)> {
    let mut handle = null_mut();
    // CNG owns the hash object when its buffer is null, and no secret is used for SHA-256.
    let status = unsafe {
        BCryptCreateHash(
            BCRYPT_SHA256_ALG_HANDLE,
            &mut handle,
            null_mut(),
            0,
            null(),
            0,
            0,
        )
    };
    ensure!(
        status >= 0 && !handle.is_null(),
        "Can't create a SHA-256 hash."
    );
    let handle = HashHandle(handle);
    let mut bytes = [0; 32 * 1024];
    let mut length = 0;
    loop {
        let read = source.read(&mut bytes)?;
        if read == 0 {
            break;
        }
        length += read as u64;
        let status = unsafe { BCryptHashData(handle.0, bytes.as_ptr(), read as u32, 0) };
        ensure!(status >= 0, "Can't hash the installer.");
    }
    let mut digest = [0; 32];
    let status = unsafe { BCryptFinishHash(handle.0, digest.as_mut_ptr(), digest.len() as u32, 0) };
    ensure!(status >= 0, "Can't finish hashing the installer.");
    let digest = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok((length, digest))
}

fn download(url: &str, output: Stdio, limit: u64) -> Result<Vec<u8>> {
    let curl = system_curl().context("Can't locate Windows' GitHub download helper.")?;
    let response = Command::new(curl)
        .args([
            "--disable",
            "--silent",
            "--show-error",
            "--fail",
            "--location",
            "--max-redirs",
            "5",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "10",
            "--max-time",
            "120",
            "--user-agent",
            concat!("Meme-Soundboard/", env!("CARGO_PKG_VERSION")),
            "--write-out",
            "%{stderr}%{http_code}\n",
        ])
        .arg("--max-filesize")
        .arg(limit.to_string())
        .arg(url)
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .stdin(Stdio::null())
        .stdout(output)
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .context("Can't download the GitHub release file.")?;
    let errors = String::from_utf8_lossy(&response.stderr);
    let code = errors
        .lines()
        .find_map(|line| line.trim().parse::<u16>().ok());
    match code {
        Some(200) if response.status.success() => Ok(response.stdout),
        Some(429) => {
            bail!(
                "GitHub is temporarily limiting file downloads. Try again later or open Releases in your browser."
            )
        }
        Some(404) => {
            bail!("The public release file is missing. Open Releases in your browser.")
        }
        _ => bail!(
            "Can't reach the GitHub release. Check your connection or open Releases in your browser."
        ),
    }
}

fn system_curl() -> Option<PathBuf> {
    let mut buffer = [0; 260];
    // The Windows 11 inbox curl executable lives in the system directory.
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if length == 0 || length >= buffer.len() {
        return None;
    }
    let path = PathBuf::from(OsString::from_wide(&buffer[..length])).join("curl.exe");
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(version: &str, size: u64, digest: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "version": version,
            "size": size,
            "sha256": digest,
        }))
        .unwrap()
    }

    #[test]
    fn windows_sha256_matches_the_release_digest_format() {
        let (length, digest) = sha256(&b"abc"[..]).unwrap();
        assert_eq!(length, 3);
        assert_eq!(
            digest,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn the_manifest_pins_the_installer_to_a_newer_stable_release() {
        let digest = "ab".repeat(32);
        let installed = Version::new(0, 4, 9);
        let check = |version| update_from_manifest(&manifest(version, 42, &digest), &installed);
        assert!(check("0.4.8").unwrap().is_none());
        assert!(check("0.4.9").unwrap().is_none());
        assert!(check("0.4.10-beta.1").unwrap().is_none());
        let update = check("0.4.10").unwrap().unwrap();
        assert_eq!(update.size, 42);
        assert_eq!(update.sha256, digest);
        assert_eq!(
            installer_url("a-nahvi/meme-soundboard", &update.version),
            "https://github.com/a-nahvi/meme-soundboard/releases/download/v0.4.10/MemeSoundboard-Setup-x64.exe"
        );
    }

    #[test]
    fn rejects_invalid_update_information() {
        let installed = Version::new(0, 4, 9);
        let digest = "ab".repeat(32);
        for bytes in [
            b"<html>Not found</html>".to_vec(),
            br#"{"version":"0.4.10"}"#.to_vec(),
            manifest("latest/other-file", 42, &digest),
            manifest("0.4.10", 0, &digest),
            manifest("0.4.10", 42, &"gg".repeat(32)),
        ] {
            assert!(update_from_manifest(&bytes, &installed).is_err());
        }
    }
}
