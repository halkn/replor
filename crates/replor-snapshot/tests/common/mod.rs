//! Building the repositories the integration tests read.

use std::path::Path;
use std::process::Command;

pub fn git(dir: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    // Keep the caller's Git environment out of the fixture: `GIT_DIR` and
    // friends would point the commands at another repository, and
    // `GIT_CONFIG_*` would bring in its configuration and hooks.
    for (key, _) in std::env::vars_os() {
        if key.to_str().is_some_and(|key| key.starts_with("GIT_")) {
            command.env_remove(key);
        }
    }
    let out = command
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "replor")
        .env("GIT_AUTHOR_EMAIL", "replor@example.invalid")
        .env("GIT_COMMITTER_NAME", "replor")
        .env("GIT_COMMITTER_EMAIL", "replor@example.invalid")
        .output()
        .expect("git should run");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("git prints UTF-8")
}

pub fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}
