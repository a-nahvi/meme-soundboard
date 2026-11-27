use std::{env, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=.git/config");
    println!("cargo:rerun-if-env-changed=MEME_SOUNDBOARD_GITHUB_REPOSITORY");
    if let Some(repository) = repository() {
        println!("cargo:rustc-env=MEME_SOUNDBOARD_GITHUB_REPOSITORY={repository}");
    }
    embed_resource::compile("assets/app.rc", embed_resource::NONE)
        .manifest_required()
        .expect("compile the Windows application resources");
}

fn repository() -> Option<String> {
    let repository = if let Ok(repository) = env::var("MEME_SOUNDBOARD_GITHUB_REPOSITORY") {
        repository.trim().to_owned()
    } else {
        let output = Command::new("git")
            .args(["remote", "get-url", "origin"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let remote = String::from_utf8(output.stdout).ok()?;
        let remote = remote.trim();
        remote
            .strip_prefix("https://github.com/")
            .or_else(|| remote.strip_prefix("git@github.com:"))
            .or_else(|| remote.strip_prefix("ssh://git@github.com/"))?
            .trim_end_matches(".git")
            .to_owned()
    };
    let (owner, name) = repository.split_once('/')?;
    let valid = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    };
    (valid(owner) && valid(name)).then_some(repository)
}
