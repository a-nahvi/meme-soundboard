use anyhow::{Context, Result, bail, ensure};
use semver::Version;
use serde::Deserialize;
use std::{
    env,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::Read,
    os::windows::process::CommandExt,
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
    System::Threading::CREATE_NO_WINDOW,
};

const INSTALLER: &str = "MemeSoundboard-Setup-x64.exe";
pub const REPOSITORY: Option<&str> = option_env!("MEME_SOUNDBOARD_GITHUB_REPOSITORY");
pub const INSTALLED_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    id: u64,
    name: String,
    size: u64,
    digest: Option<String>,
}

#[derive(Clone)]
pub struct AvailableUpdate {
    pub version: Version,
    asset_id: u64,
    size: u64,
    sha256: String,
}

pub fn check(repository: &str) -> Result<Option<AvailableUpdate>> {
    let endpoint = format!("repos/{repository}/releases/latest");
    let output = api()?
        .arg(endpoint)
        .output()
        .context("Can't start GitHub CLI.")?;
    if !output.status.success() {
        api_error(&output.stderr)?;
    }
    let release =
        serde_json::from_slice(&output.stdout).context("Can't read the GitHub release.")?;
    update_from_release(release, &Version::parse(INSTALLED_VERSION)?)
}

fn update_from_release(release: Release, installed: &Version) -> Result<Option<AvailableUpdate>> {
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let tag = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    let version = Version::parse(tag).context("The latest release has an invalid version tag.")?;
    if !version.pre.is_empty() || !version.cmp_precedence(installed).is_gt() {
        return Ok(None);
    }
    let asset = release
        .assets
        .into_iter()
        .find(|asset| asset.name == INSTALLER)
        .context("The latest GitHub release is missing the Windows installer.")?;
    let sha256 = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .context("GitHub has not supplied a SHA-256 digest for this installer.")?;
    ensure!(
        asset.id != 0 && asset.size != 0,
        "The installer asset is incomplete."
    );
    Ok(Some(AvailableUpdate {
        version,
        asset_id: asset.id,
        size: asset.size,
        sha256: sha256.to_owned(),
    }))
}

pub fn download_and_install(repository: &str, update: &AvailableUpdate, data: &Path) -> Result<()> {
    let install_dir = data
        .parent()
        .context("Can't locate the installation folder.")?;
    let directory = data.join("updates").join(update.version.to_string());
    fs::create_dir_all(&directory).context("Can't create the updates folder.")?;
    let installer = directory.join(INSTALLER);
    if !installer_matches(&installer, update) {
        let endpoint = format!("repos/{repository}/releases/assets/{}", update.asset_id);
        let mut command = api()?;
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let staging = directory.join(format!("download-{nonce}.part"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
            .context("Can't create the installer download file.")?;
        let downloaded = (|| {
            let output = command
                .args(["-H", "Accept: application/octet-stream"])
                .arg(endpoint)
                .stdout(Stdio::from(file))
                .output()
                .context("Can't download the installer from GitHub.")?;
            if !output.status.success() {
                api_error(&output.stderr)?;
            }
            ensure!(
                installer_matches(&staging, update),
                "The installer did not match the GitHub release digest."
            );
            fs::rename(&staging, &installer).context("Can't save the verified installer.")
        })();
        if downloaded.is_err() {
            let _ = fs::remove_file(&staging);
        }
        downloaded?;
    }

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

fn api() -> Result<Command> {
    let path = env::var_os("PATH").unwrap_or_default();
    let cli = env::split_paths(&path)
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join("gh.exe"))
        .chain(env::var_os("ProgramFiles").map(|programs| PathBuf::from(programs).join("GitHub CLI/gh.exe")))
        .find(|exe| exe.is_file())
        .context("To check in Soundboard, install GitHub CLI and sign in. You can also open Releases in your browser.")?;
    let mut command = Command::new(cli);
    command
        .args(["api", "--hostname", "github.com"])
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env_remove("GH_DEBUG")
        .env_remove("GH_REPO")
        .creation_flags(CREATE_NO_WINDOW);
    Ok(command)
}

fn api_error(stderr: &[u8]) -> Result<()> {
    let error = String::from_utf8_lossy(stderr);
    if error.contains("gh auth login") || error.contains("HTTP 401") || error.contains("HTTP 403") {
        bail!("Sign in with GitHub CLI using an account that can read the private repository.");
    }
    if error.contains("HTTP 404") {
        bail!("GitHub could not find a release you can access. Open Releases in your browser.");
    }
    bail!("Can't reach the GitHub release. Check your connection or open Releases in your browser.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool, digest: Option<&str>) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease,
            assets: vec![Asset {
                id: 1,
                name: INSTALLER.into(),
                size: 1,
                digest: digest.map(str::to_owned),
            }],
        }
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
    fn checks_newer_stable_versions_and_requires_a_verified_installer() {
        let digest = format!("sha256:{}", "ab".repeat(32));
        let installed = Version::parse("0.4.3").unwrap();
        let check = |tag, prerelease, digest| {
            update_from_release(release(tag, prerelease, digest), &installed)
        };
        assert!(check("v0.4.3", false, None).unwrap().is_none());
        assert!(check("v0.4.2", false, None).unwrap().is_none());
        assert!(check("v0.4.4-beta.1", true, None).unwrap().is_none());
        assert!(check("v0.4.4", false, None).is_err());
        assert_eq!(
            check("v0.4.10", false, Some(&digest))
                .unwrap()
                .unwrap()
                .version
                .to_string(),
            "0.4.10"
        );
    }
}
