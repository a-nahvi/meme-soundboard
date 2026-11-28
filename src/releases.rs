use anyhow::{Context, Result, bail, ensure};
use semver::Version;
use serde::Deserialize;
use std::{
    env,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
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
pub const REPOSITORY: &str = env!("MEME_SOUNDBOARD_GITHUB_REPOSITORY");
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
    let response = request(
        &endpoint,
        "application/vnd.github+json",
        Stdio::piped(),
        2_097_152,
    )?;
    let release = serde_json::from_slice(&response).context("Can't read the GitHub release.")?;
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
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let staging = directory.join(format!("download-{nonce}.part"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
            .context("Can't create the installer download file.")?;
        let downloaded = (|| {
            request(
                &endpoint,
                "application/octet-stream",
                Stdio::from(file),
                update.size,
            )?;
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

fn request(endpoint: &str, accept: &str, output: Stdio, limit: u64) -> Result<Vec<u8>> {
    let token = if option_env!("MEME_SOUNDBOARD_PRIVATE_UPDATES") == Some("1") {
        let token = github_token().context(
            "Sign in to Git for Windows or GitHub CLI with access to this repository. You can also open Releases in your browser.",
        )?;
        ensure!(
            token.trim().bytes().all(|byte| byte.is_ascii_graphic()),
            "The stored GitHub credential is invalid."
        );
        Some(token)
    } else {
        None
    };
    let curl = system_curl().context("Can't locate Windows' GitHub download helper.")?;
    let mut command = Command::new(curl);
    command
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
            "--header",
            "X-GitHub-Api-Version: 2026-03-10",
            "--write-out",
            "%{stderr}%{http_code}\n",
        ])
        .arg("--header")
        .arg(format!("Accept: {accept}"))
        .arg("--max-filesize")
        .arg(limit.to_string())
        .arg(format!("https://api.github.com/{endpoint}"))
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .stdout(output)
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);
    if token.is_some() {
        // curl drops Authorization on cross-origin redirects. Keep the token off the command line.
        command.args(["--header", "@-"]).stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    let mut child = command
        .spawn()
        .context("Can't start the GitHub connection.")?;
    let header_sent = if let Some(token) = &token {
        child
            .stdin
            .take()
            .context("Can't send GitHub authentication.")?
            .write_all(format!("Authorization: Bearer {}\n", token.trim()).as_bytes())
    } else {
        Ok(())
    };
    let response = child
        .wait_with_output()
        .context("Can't read the GitHub response.")?;
    let errors = String::from_utf8_lossy(&response.stderr);
    let code = errors
        .lines()
        .find_map(|line| line.trim().parse::<u16>().ok());
    match code {
        Some(200..=299) if response.status.success() => {
            header_sent.context("Can't send GitHub authentication.")?;
            Ok(response.stdout)
        }
        Some(401 | 403) if token.is_some() => {
            bail!("Sign in to GitHub with an account that can read the private repository.")
        }
        Some(403 | 429) => {
            bail!(
                "GitHub is limiting update checks. Try again later or open Releases in your browser."
            )
        }
        Some(404) => {
            bail!("GitHub could not find a release you can access. Open Releases in your browser.")
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

fn github_token() -> Option<String> {
    ["GH_TOKEN", "GITHUB_TOKEN"]
        .into_iter()
        .filter_map(|name| env::var(name).ok())
        .find(|token| !token.trim().is_empty())
        .or_else(cli_token)
        .or_else(git_token)
}

fn executable(name: &str, program_path: &str) -> Option<PathBuf> {
    env::split_paths(&env::var_os("PATH").unwrap_or_default())
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join(name))
        .chain(
            env::var_os("ProgramFiles").map(|programs| PathBuf::from(programs).join(program_path)),
        )
        .find(|exe| exe.is_file())
}

fn cli_token() -> Option<String> {
    let cli = executable("gh.exe", "GitHub CLI/gh.exe")?;
    let output = Command::new(cli)
        .args(["auth", "token", "--hostname", "github.com"])
        .env("GH_PROMPT_DISABLED", "1")
        .env_remove("GH_DEBUG")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8(output.stdout).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

fn git_token() -> Option<String> {
    let git = executable("git.exe", "Git/cmd/git.exe")?;
    let mut child = Command::new(git)
        .args(["-c", "credential.interactive=false", "credential", "fill"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .env_remove("GIT_TRACE")
        .env_remove("GIT_TRACE_CURL")
        .env_remove("GCM_TRACE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .ok()?;
    let input_ok = child
        .stdin
        .take()?
        .write_all(b"protocol=https\nhost=github.com\n\n")
        .is_ok();
    let output = child.wait_with_output().ok()?;
    if !input_ok || !output.status.success() {
        return None;
    }
    let record = String::from_utf8(output.stdout).ok()?;
    record.lines().find_map(|line| {
        line.strip_prefix("password=")
            .filter(|token| !token.is_empty())
            .map(str::to_owned)
    })
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
