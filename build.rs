use std::{env, path::Path, process::Command};

const UPSTREAM: &str = "a-nahvi/meme-soundboard";

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=.git/config");
    println!("cargo:rerun-if-env-changed=MEME_SOUNDBOARD_GITHUB_REPOSITORY");
    println!("cargo:rerun-if-env-changed=MEME_SOUNDBOARD_PRIVATE_UPDATES");
    let repository = repository();
    println!("cargo:rustc-env=MEME_SOUNDBOARD_GITHUB_REPOSITORY={repository}");
    embed_resource::compile("assets/app.rc", embed_resource::NONE)
        .manifest_required()
        .expect("compile the Windows application resources");
}

fn repository() -> String {
    if let Ok(repository) = env::var("MEME_SOUNDBOARD_GITHUB_REPOSITORY") {
        let repository = repository.trim();
        assert!(
            valid_repository(repository),
            "MEME_SOUNDBOARD_GITHUB_REPOSITORY must be in owner/repository form"
        );
        return repository.to_owned();
    }
    origin_repository()
        .filter(|repository| valid_repository(repository))
        .unwrap_or_else(|| UPSTREAM.to_owned())
}

fn origin_repository() -> Option<String> {
    if !Path::new(".git").exists() {
        return None;
    }
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let remote = String::from_utf8(output.stdout).ok()?;
    let remote = remote.trim();
    Some(
        remote
            .strip_prefix("https://github.com/")
            .or_else(|| remote.strip_prefix("git@github.com:"))
            .or_else(|| remote.strip_prefix("ssh://git@github.com/"))?
            .trim_end_matches(".git")
            .to_owned(),
    )
}

fn valid_repository(repository: &str) -> bool {
    let Some((owner, name)) = repository.split_once('/') else {
        return false;
    };
    let valid = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    };
    valid(owner) && valid(name)
}
