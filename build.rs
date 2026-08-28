use std::process::Command;

use sha2::{Digest, Sha256};

const AUTHORITY_INPUT_PATHS: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "build.rs",
    ".cargo",
    "rust-toolchain.toml",
    "src",
    "prompts",
    "assets",
    "eval/public/memory/suites/adversarial-policy/suite.json",
];

fn main() {
    for path in AUTHORITY_INPUT_PATHS {
        println!("cargo:rerun-if-changed={path}");
    }
    emit_git_rerun_paths();

    if let Some(commit) = build_git_stdout(&["rev-parse", "--verify", "HEAD^{commit}"]) {
        println!("cargo:rustc-env=REMEM_BUILD_GIT_SHA={commit}");
    }
    if let Some(dirty) = git_dirty() {
        println!("cargo:rustc-env=REMEM_BUILD_SOURCE_DIRTY={dirty}");
    }
    if let Some(tree) = production_input_tree_sha256() {
        println!("cargo:rustc-env=REMEM_BUILD_PRODUCTION_INPUT_TREE_SHA256={tree}");
    }
}

fn emit_git_rerun_paths() {
    for args in [
        &["rev-parse", "--git-path", "HEAD"][..],
        &["rev-parse", "--git-path", "index"][..],
    ] {
        if let Some(path) = build_git_stdout(args) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    if let Some(reference) = build_git_stdout(&["symbolic-ref", "-q", "HEAD"]) {
        if let Some(path) = build_git_stdout(&["rev-parse", "--git-path", &reference]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
}

fn build_git_stdout(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!value.is_empty()).then_some(value)
}

fn git_dirty() -> Option<bool> {
    let output = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .output()
        .ok()?;
    output.status.success().then_some(!output.stdout.is_empty())
}

fn production_input_tree_sha256() -> Option<String> {
    let mut args = vec!["ls-files", "-s", "--"];
    args.extend(AUTHORITY_INPUT_PATHS);
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    Some(format!("{:x}", Sha256::digest(&output.stdout)))
}
