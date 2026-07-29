use std::io;
use std::mem::size_of_val;
use std::path::Path;
use std::ptr::null;

use windows_sys::Win32::Storage::FileSystem::{
    FileBasicInfo, GetFileInformationByHandleEx, FILE_BASIC_INFO, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, OPEN_EXISTING,
};

use super::{
    file_identity, open_path, raw_handle, verify_handle_kind, HandleKind, WindowsFileId,
    SAFE_SHARE_MODE,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WindowsPathFingerprint {
    identity: WindowsFileId,
    change_time: i64,
}

pub(super) fn path_fingerprint(
    path: &Path,
    allow_reparse_point: bool,
) -> io::Result<WindowsPathFingerprint> {
    let file = open_path(
        path,
        FILE_READ_ATTRIBUTES,
        SAFE_SHARE_MODE | FILE_SHARE_DELETE,
        OPEN_EXISTING,
        FILE_FLAG_OPEN_REPARSE_POINT,
        null(),
    )?;
    if !allow_reparse_point {
        verify_handle_kind(&file, HandleKind::File)?;
    }
    let identity = file_identity(&file)?;
    let mut basic = FILE_BASIC_INFO::default();
    // SAFETY: the output buffer is correctly sized and writable, and the file
    // handle remains valid for the duration of the call.
    if unsafe {
        GetFileInformationByHandleEx(
            raw_handle(&file),
            FileBasicInfo,
            (&mut basic as *mut FILE_BASIC_INFO).cast(),
            size_of_val(&basic) as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(WindowsPathFingerprint {
        identity,
        change_time: basic.ChangeTime,
    })
}
