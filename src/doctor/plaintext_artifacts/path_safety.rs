use std::ffi::OsStr;
use std::fs::Metadata;
use std::path::Path;

#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT as WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT;

#[cfg(all(test, not(windows)))]
const WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

#[cfg(windows)]
pub(super) fn is_reparse_point(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    has_reparse_point_attribute(metadata.file_attributes())
}

#[cfg(not(windows))]
pub(super) fn is_reparse_point(_metadata: &Metadata) -> bool {
    false
}

pub(super) fn is_managed_backups_path(data_dir: &Path, path: &Path) -> bool {
    path.parent() == Some(data_dir) && path.file_name().is_some_and(is_managed_backups_component)
}

#[cfg(windows)]
fn is_managed_backups_component(component: &OsStr) -> bool {
    is_windows_managed_backups_component(component)
}

#[cfg(any(windows, test))]
fn is_windows_managed_backups_component(component: &OsStr) -> bool {
    component
        .to_str()
        .is_some_and(|name| name.eq_ignore_ascii_case("backups"))
}

#[cfg(not(windows))]
fn is_managed_backups_component(component: &OsStr) -> bool {
    component == OsStr::new("backups")
}

#[cfg(any(windows, test))]
fn has_reparse_point_attribute(attributes: u32) -> bool {
    attributes & WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_reparse_attribute_is_rejected_without_rejecting_ordinary_entries() {
        assert!(has_reparse_point_attribute(
            WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT
        ));
        assert!(!has_reparse_point_attribute(0));
        assert!(!has_reparse_point_attribute(0x0010));
    }

    #[test]
    fn windows_managed_backups_component_is_ascii_case_insensitive() {
        for name in ["backups", "BACKUPS", "Backups"] {
            assert!(is_windows_managed_backups_component(OsStr::new(name)));
        }
        assert!(!is_windows_managed_backups_component(OsStr::new("backup")));
    }
}
