use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn main() {
    // Mirrors the vite `define` of YTMD_GIT_BRANCH / YTMD_GIT_COMMIT_HASH
    if let Some(branch) = git(&["rev-parse", "--abbrev-ref", "HEAD"]) {
        println!("cargo:rustc-env=YTMD_GIT_BRANCH={branch}");
    }
    if let Some(hash) = git(&["rev-parse", "HEAD"]) {
        println!("cargo:rustc-env=YTMD_GIT_COMMIT_HASH={hash}");
    }
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-env-changed=LASTFM_API_KEY");
    println!("cargo:rerun-if-env-changed=LASTFM_API_SECRET");
}
