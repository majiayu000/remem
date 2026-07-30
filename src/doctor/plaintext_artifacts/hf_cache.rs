use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub(super) fn is_snapshot_pointer(data_dir: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(data_dir) else {
        return false;
    };
    let Some(parts) = relative
        .components()
        .map(|component| match component {
            Component::Normal(part) => Some(part),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    let Some(snapshot) = parts
        .iter()
        .rposition(|part| *part == OsStr::new("snapshots"))
    else {
        return false;
    };
    let lower_hex_len = |part: &OsStr| {
        part.to_str()
            .filter(|value| {
                value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            })
            .map(str::len)
    };
    if snapshot != 3
        || parts.len() < snapshot + 3
        || parts[0] != OsStr::new("models")
        || !parts[1]
            .to_str()
            .is_some_and(|name| name.starts_with("fastembed-"))
        || !parts[2]
            .to_str()
            .is_some_and(|name| name.starts_with("models--"))
        || lower_hex_len(parts[snapshot + 1]) != Some(40)
    {
        return false;
    }
    let Ok(target) = fs::read_link(path) else {
        return false;
    };
    let parent_depth = parts.len() - snapshot - 1;
    let Some(Component::Normal(digest)) = target.components().next_back() else {
        return false;
    };
    let mut expected = PathBuf::new();
    for _ in 0..parent_depth {
        expected.push("..");
    }
    expected.push("blobs");
    expected.push(digest);
    if target.as_os_str() != expected.as_os_str()
        || !matches!(lower_hex_len(digest), Some(40) | Some(64))
    {
        return false;
    }
    let repo = parts[..snapshot]
        .iter()
        .fold(data_dir.to_path_buf(), |base, part| base.join(part));
    let blobs = repo.join("blobs");
    fs::symlink_metadata(&blobs).is_ok_and(|metadata| metadata.is_dir())
        && fs::symlink_metadata(blobs.join(digest)).is_ok_and(|metadata| metadata.is_file())
}
