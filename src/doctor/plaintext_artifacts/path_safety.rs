use std::fs::Metadata;

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
}
