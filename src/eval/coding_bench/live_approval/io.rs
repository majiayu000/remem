use std::collections::BTreeSet;
use std::env;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};

use super::super::process::{command_output, ensure_success};

#[derive(Debug)]
pub(super) struct FileSnapshot {
    pub(super) bytes: Vec<u8>,
    pub(super) sha256: String,
    pub(super) uid: u32,
    pub(super) mode: u32,
}

#[derive(Debug)]
pub(super) struct GitBinding {
    pub(super) root: PathBuf,
    pub(super) head: String,
    pub(super) branch: String,
    pub(super) origin_main: String,
}

pub(super) fn load_git_binding(cwd: &Path) -> Result<GitBinding> {
    let root = PathBuf::from(git_stdout(cwd, &["rev-parse", "--show-toplevel"])?);
    let root = fs::canonicalize(&root).context("canonicalize live approval repository root")?;
    let head = git_stdout(&root, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let branch = git_stdout(&root, &["branch", "--show-current"])?;
    let origin_main = git_stdout(
        &root,
        &["rev-parse", "--verify", "refs/remotes/origin/main^{commit}"],
    )?;
    Ok(GitBinding {
        root,
        head: head.to_ascii_lowercase(),
        branch,
        origin_main: origin_main.to_ascii_lowercase(),
    })
}

pub(super) fn ensure_ancestor(git: &GitBinding, ancestor: &str) -> Result<()> {
    let output = command_output(
        "git",
        ["merge-base", "--is-ancestor", ancestor, &git.head],
        &git.root,
        &[],
        30_000,
    )?;
    ensure_success("approved commit ancestry check", &output)
}

pub(super) fn read_tracked_head_json<T: DeserializeOwned>(
    git: &GitBinding,
    path: &Path,
    label: &str,
) -> Result<(T, FileSnapshot)> {
    let (absolute, relative) = confined_repo_path(&git.root, path)?;
    let relative_text = relative
        .to_str()
        .context("live approval policy path is not UTF-8")?;
    if relative_text.contains(':') {
        bail!("{label} path must not contain ':'");
    }
    git_stdout(
        &git.root,
        &["ls-files", "--error-unmatch", "--", relative_text],
    )
    .with_context(|| format!("{label} is not tracked at HEAD"))?;
    let snapshot = read_nofollow(&absolute, label)?;
    let head_blob = git_stdout_raw(&git.root, &["show", &format!("HEAD:{relative_text}")])?;
    if head_blob.as_bytes() != snapshot.bytes {
        bail!("{label} differs from its tracked HEAD blob");
    }
    let parsed = parse_unique_json(&snapshot.bytes).with_context(|| format!("parse {label}"))?;
    Ok((parsed, snapshot))
}

pub(super) fn read_json_nofollow<T: DeserializeOwned>(
    path: &Path,
    label: &str,
) -> Result<(T, FileSnapshot)> {
    let snapshot = read_nofollow(path, label)?;
    let parsed = parse_unique_json(&snapshot.bytes).with_context(|| format!("parse {label}"))?;
    Ok((parsed, snapshot))
}

pub(super) fn read_nofollow(path: &Path, label: &str) -> Result<FileSnapshot> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut file = options.open(path).with_context(|| {
        format!(
            "open {label} without following final symlink {}",
            path.display()
        )
    })?;
    let metadata = file
        .metadata()
        .with_context(|| format!("inspect opened {label} {}", path.display()))?;
    if !metadata.is_file() {
        bail!("{label} must be a regular file");
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("read opened {label} {}", path.display()))?;
    #[cfg(unix)]
    let (uid, mode) = (metadata.uid(), metadata.mode());
    #[cfg(not(unix))]
    let (uid, mode) = (u32::MAX, 0_u32);
    Ok(FileSnapshot {
        sha256: sha256_hex(&bytes),
        bytes,
        uid,
        mode,
    })
}

pub(super) fn resolve_executable(value: &str, cwd: &Path) -> Result<PathBuf> {
    let requested = Path::new(value);
    if requested.is_absolute() || requested.components().count() > 1 {
        return Ok(if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            cwd.join(requested)
        });
    }
    let path = env::var_os("PATH").context("PATH is unavailable while resolving executable")?;
    env::split_paths(&path)
        .map(|directory| directory.join(requested))
        .find(|candidate| candidate.is_file())
        .with_context(|| format!("resolve executable {value:?} from PATH"))
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn confined_repo_path(root: &Path, path: &Path) -> Result<(PathBuf, PathBuf)> {
    let relative = if path.is_absolute() {
        path.strip_prefix(root)
            .context("live approval policy path is outside repository root")?
            .to_path_buf()
    } else {
        path.to_path_buf()
    };
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("live approval policy path must be a confined repository-relative file");
    }
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let canonical_parent =
        fs::canonicalize(root.join(parent)).context("canonicalize live approval policy parent")?;
    if !canonical_parent.starts_with(root) {
        bail!("live approval policy parent escapes repository root");
    }
    let file_name = relative
        .file_name()
        .context("live approval policy path has no file name")?;
    Ok((canonical_parent.join(file_name), relative))
}

fn git_stdout(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = command_output("git", args, cwd, &[], 30_000)?;
    ensure_success("live approval git command", &output)?;
    let value = output.stdout.trim().to_string();
    if value.is_empty() {
        bail!("live approval git command returned empty output");
    }
    Ok(value)
}

fn git_stdout_raw(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = command_output("git", args, cwd, &[], 30_000)?;
    ensure_success("live approval git command", &output)?;
    Ok(output.stdout)
}

fn parse_unique_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let unique: UniqueJson = serde_json::from_slice(bytes)?;
    serde_json::from_value(unique.0).map_err(Into::into)
}

struct UniqueJson(Value);

impl<'de> Deserialize<'de> for UniqueJson {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueJsonVisitor)
    }
}

struct UniqueJsonVisitor;

impl<'de> Visitor<'de> for UniqueJsonVisitor {
    type Value = UniqueJson;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E> {
        Ok(UniqueJson(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
        Ok(UniqueJson(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
        Ok(UniqueJson(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E>
    where
        E: de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .map(UniqueJson)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
        Ok(UniqueJson(Value::String(value.to_string())))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
        Ok(UniqueJson(Value::String(value)))
    }

    fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(UniqueJson(Value::Null))
    }

    fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(UniqueJson(Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<UniqueJson>()? {
            values.push(value.0);
        }
        Ok(UniqueJson(Value::Array(values)))
    }

    fn visit_map<A>(self, mut object: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = BTreeSet::new();
        let mut values = Map::new();
        while let Some((key, value)) = object.next_entry::<String, UniqueJson>()? {
            if !keys.insert(key.clone()) {
                return Err(de::Error::custom(format!("duplicate JSON key {key:?}")));
            }
            values.insert(key, value.0);
        }
        Ok(UniqueJson(Value::Object(values)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_json_rejects_duplicate_keys_recursively() {
        let error = parse_unique_json::<Value>(br#"{"outer":{"value":1,"value":2}}"#)
            .expect_err("duplicate nested key must fail");
        assert!(error.to_string().contains("duplicate JSON key"));
    }

    #[cfg(unix)]
    #[test]
    fn nofollow_reader_rejects_final_symlink() {
        use std::os::unix::fs::symlink;

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("test clock after epoch")
            .as_nanos();
        let target = std::env::temp_dir().join(format!(
            "remem-gh931-nofollow-{}-{nonce}-target.json",
            std::process::id()
        ));
        let link = std::env::temp_dir().join(format!(
            "remem-gh931-nofollow-{}-{nonce}-approval.json",
            std::process::id()
        ));
        fs::write(&target, b"{}").expect("write symlink target");
        symlink(&target, &link).expect("create final symlink");

        let error = read_nofollow(&link, "approval").expect_err("final symlink must fail");
        fs::remove_file(&link).expect("remove test symlink");
        fs::remove_file(&target).expect("remove symlink target");
        assert!(error
            .to_string()
            .contains("without following final symlink"));
    }
}
