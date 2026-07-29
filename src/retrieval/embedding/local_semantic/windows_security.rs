use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::mem::{size_of, size_of_val};
use std::ops::Deref;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    LocalFree, ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS,
    GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    EqualSid, GetAce, GetLengthSid, GetSecurityDescriptorControl, GetSecurityDescriptorDacl,
    GetTokenInformation, IsValidAcl, IsValidSecurityDescriptor, IsValidSid, TokenUser, ACE_HEADER,
    ACL, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, OBJECT_INHERIT_ACE,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, SE_DACL_PROTECTED,
    TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateDirectoryW, CreateFileW, FileAttributeTagInfo, FileIdInfo, GetFileInformationByHandleEx,
    CREATE_NEW, DELETE, FILE_ALL_ACCESS, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL,
};
use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

mod path_fingerprint;
pub(super) use path_fingerprint::{path_fingerprint, WindowsPathFingerprint};

const DIRECTORY_ACE_FLAGS: u8 = (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE) as u8;
// Omitting FILE_SHARE_DELETE is what turns an open handle into a namespace
// anchor: another process cannot rename, delete, or replace that object.
const SAFE_SHARE_MODE: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WindowsFileId {
    volume_serial_number: u64,
    identifier: [u8; 16],
}

#[derive(Debug)]
pub(super) struct DirectoryAnchor {
    file: File,
    identity: WindowsFileId,
    path: PathBuf,
}

#[derive(Debug)]
pub(super) struct AnchoredFile {
    file: File,
    _parent_anchor: DirectoryAnchor,
}

impl AnchoredFile {
    pub(super) fn new(file: File, parent_anchor: DirectoryAnchor) -> Self {
        Self {
            file,
            _parent_anchor: parent_anchor,
        }
    }
}

impl Deref for AnchoredFile {
    type Target = File;

    fn deref(&self) -> &Self::Target {
        &self.file
    }
}

impl fs2::FileExt for AnchoredFile {
    fn duplicate(&self) -> io::Result<File> {
        fs2::FileExt::duplicate(&self.file)
    }

    fn allocated_size(&self) -> io::Result<u64> {
        fs2::FileExt::allocated_size(&self.file)
    }

    fn allocate(&self, len: u64) -> io::Result<()> {
        fs2::FileExt::allocate(&self.file, len)
    }

    fn lock_shared(&self) -> io::Result<()> {
        fs2::FileExt::lock_shared(&self.file)
    }

    fn lock_exclusive(&self) -> io::Result<()> {
        fs2::FileExt::lock_exclusive(&self.file)
    }

    fn try_lock_shared(&self) -> io::Result<()> {
        fs2::FileExt::try_lock_shared(&self.file)
    }

    fn try_lock_exclusive(&self) -> io::Result<()> {
        fs2::FileExt::try_lock_exclusive(&self.file)
    }

    fn unlock(&self) -> io::Result<()> {
        fs2::FileExt::unlock(&self.file)
    }
}

impl DirectoryAnchor {
    pub(super) fn open_owner_only(path: &Path, allow_rename: bool) -> io::Result<Self> {
        Self::open_inner(path, true, allow_rename, false)
    }

    pub(super) fn open_owner_only_for_cleanup(path: &Path, allow_rename: bool) -> io::Result<Self> {
        Self::open_inner(path, true, allow_rename, true)
    }

    fn open_inner(
        path: &Path,
        owner_only: bool,
        allow_rename: bool,
        delete_access: bool,
    ) -> io::Result<Self> {
        let desired_access =
            FILE_READ_ATTRIBUTES | READ_CONTROL | if delete_access { DELETE } else { 0 };
        let mut share_mode = SAFE_SHARE_MODE;
        if allow_rename {
            share_mode |= FILE_SHARE_DELETE;
        }
        let file = open_path(
            path,
            desired_access,
            share_mode,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null(),
        )?;
        verify_handle_kind(&file, HandleKind::Directory)?;
        if owner_only {
            verify_owner_only_dacl(&file, HandleKind::Directory)?;
        }
        let identity = file_identity(&file)?;
        verify_path_identity(path, HandleKind::Directory, identity)?;
        Ok(Self {
            file,
            identity,
            path: path.to_path_buf(),
        })
    }

    pub(super) fn identity(&self) -> WindowsFileId {
        self.identity
    }

    pub(super) fn verify_path(&self) -> io::Result<()> {
        verify_path_identity(&self.path, HandleKind::Directory, self.identity)
    }

    pub(super) fn file(&self) -> &File {
        &self.file
    }
}

pub(super) fn create_owner_only_directory(
    path: &Path,
    allow_rename: bool,
) -> io::Result<DirectoryAnchor> {
    create_owner_only_directory_inner(path, allow_rename, false)
}

pub(super) fn create_owner_only_cleanup_directory(
    path: &Path,
    allow_rename: bool,
) -> io::Result<DirectoryAnchor> {
    create_owner_only_directory_inner(path, allow_rename, true)
}

fn create_owner_only_directory_inner(
    path: &Path,
    allow_rename: bool,
    delete_access: bool,
) -> io::Result<DirectoryAnchor> {
    let security = OwnerOnlySecurity::new(HandleKind::Directory)?;
    let attributes = security.attributes();
    let path_wide = wide_path(path)?;
    // SAFETY: `path_wide` is NUL-terminated and `attributes` points to a live,
    // self-relative security descriptor for the duration of the call.
    if unsafe { CreateDirectoryW(path_wide.as_ptr(), &attributes) } == 0 {
        return Err(io::Error::last_os_error());
    }
    DirectoryAnchor::open_inner(path, true, allow_rename, delete_access)
}

pub(super) fn open_or_create_owner_only_directory(path: &Path) -> io::Result<DirectoryAnchor> {
    match create_owner_only_directory(path, false) {
        Ok(anchor) => Ok(anchor),
        Err(error)
            if matches!(
                error.raw_os_error().map(|code| code as u32),
                Some(ERROR_ALREADY_EXISTS)
            ) =>
        {
            DirectoryAnchor::open_owner_only(path, false)
        }
        Err(error) => Err(error),
    }
}

pub(super) fn create_owner_only_lock_file(path: &Path) -> io::Result<File> {
    let security = OwnerOnlySecurity::new(HandleKind::File)?;
    let attributes = security.attributes();
    let file = open_lock_path(path, CREATE_NEW, &attributes)?;
    validate_lock_file(path, &file)?;
    Ok(file)
}

pub(super) fn open_owner_only_lock_file(path: &Path) -> io::Result<File> {
    let file = open_lock_path(path, OPEN_EXISTING, null())?;
    validate_lock_file(path, &file)?;
    Ok(file)
}

pub(super) fn open_or_create_owner_only_lock_file(path: &Path) -> io::Result<File> {
    match create_owner_only_lock_file(path) {
        Ok(file) => Ok(file),
        Err(error)
            if matches!(
                error.raw_os_error().map(|code| code as u32),
                Some(ERROR_FILE_EXISTS) | Some(ERROR_ALREADY_EXISTS)
            ) =>
        {
            open_owner_only_lock_file(path)
        }
        Err(error) => Err(error),
    }
}

pub(super) fn open_owner_only_lock_file_for_cleanup(path: &Path) -> io::Result<File> {
    let file = open_path(
        path,
        DELETE | READ_CONTROL | FILE_READ_ATTRIBUTES,
        SAFE_SHARE_MODE,
        OPEN_EXISTING,
        FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
        null(),
    )?;
    validate_lock_file(path, &file)?;
    Ok(file)
}

pub(super) fn validate_lock_file(path: &Path, file: &File) -> io::Result<()> {
    verify_handle_kind(file, HandleKind::File)?;
    verify_owner_only_dacl(file, HandleKind::File)?;
    let identity = file_identity(file)?;
    verify_path_identity(path, HandleKind::File, identity)
}

pub(super) fn lock_file_identity(file: &File) -> io::Result<WindowsFileId> {
    file_identity(file)
}

fn open_lock_path(
    path: &Path,
    creation_disposition: u32,
    security_attributes: *const SECURITY_ATTRIBUTES,
) -> io::Result<File> {
    open_path(
        path,
        GENERIC_READ | GENERIC_WRITE | READ_CONTROL | FILE_READ_ATTRIBUTES,
        SAFE_SHARE_MODE,
        creation_disposition,
        FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
        security_attributes,
    )
}

fn verify_path_identity(path: &Path, kind: HandleKind, expected: WindowsFileId) -> io::Result<()> {
    // This short-lived verifier must share delete so it remains compatible
    // with an already-open DELETE-capable cleanup anchor. The primary anchor,
    // not this verifier, controls whether the object can be renamed/deleted.
    let share_mode = SAFE_SHARE_MODE | FILE_SHARE_DELETE;
    let flags = FILE_FLAG_OPEN_REPARSE_POINT
        | if kind == HandleKind::Directory {
            FILE_FLAG_BACKUP_SEMANTICS
        } else {
            0
        };
    let file = open_path(
        path,
        FILE_READ_ATTRIBUTES,
        share_mode,
        OPEN_EXISTING,
        flags,
        null(),
    )?;
    verify_handle_kind(&file, kind)?;
    let actual = file_identity(&file)?;
    if actual != expected {
        return Err(io::Error::other(format!(
            "{} changed while its Windows handle was being verified",
            path.display()
        )));
    }
    Ok(())
}

fn open_path(
    path: &Path,
    desired_access: u32,
    share_mode: u32,
    creation_disposition: u32,
    flags: u32,
    security_attributes: *const SECURITY_ATTRIBUTES,
) -> io::Result<File> {
    let path_wide = wide_path(path)?;
    // SAFETY: `path_wide` is NUL-terminated. If non-null, security_attributes
    // points to a descriptor that the caller keeps alive through this call.
    let handle = unsafe {
        CreateFileW(
            path_wide.as_ptr(),
            desired_access,
            share_mode,
            security_attributes,
            creation_disposition,
            flags,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateFileW returned a unique owned handle and this is its only
    // conversion into a Rust owner.
    Ok(unsafe { File::from_raw_handle(handle as RawHandle) })
}

fn verify_handle_kind(file: &File, expected: HandleKind) -> io::Result<()> {
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    // SAFETY: the output buffer is correctly sized and writable, and the file
    // handle remains valid for the duration of the call.
    if unsafe {
        GetFileInformationByHandleEx(
            raw_handle(file),
            FileAttributeTagInfo,
            (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            size_of_val(&info) as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::other(
            "Windows filesystem object is a reparse point",
        ));
    }
    let is_directory = info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    if is_directory != (expected == HandleKind::Directory) {
        return Err(io::Error::other(match expected {
            HandleKind::Directory => "Windows filesystem object is not a directory",
            HandleKind::File => "Windows filesystem object is not a regular file",
        }));
    }
    Ok(())
}

fn file_identity(file: &File) -> io::Result<WindowsFileId> {
    let mut info = FILE_ID_INFO::default();
    // SAFETY: the output buffer is correctly sized and writable, and the file
    // handle remains valid for the duration of the call.
    if unsafe {
        GetFileInformationByHandleEx(
            raw_handle(file),
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            size_of_val(&info) as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(WindowsFileId {
        volume_serial_number: info.VolumeSerialNumber,
        identifier: info.FileId.Identifier,
    })
}

fn verify_owner_only_dacl(file: &File, kind: HandleKind) -> io::Result<()> {
    let expected = OwnerOnlySecurity::new(kind)?;
    let mut owner: PSID = null_mut();
    let mut dacl: *mut ACL = null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: every output pointer is valid and the returned descriptor is
    // released by `LocalAllocation`.
    let status = unsafe {
        GetSecurityInfo(
            raw_handle(file),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = LocalAllocation::new(descriptor.cast())?;
    // SAFETY: GetSecurityInfo returned `descriptor`, and it remains live until
    // the end of this function.
    if unsafe { IsValidSecurityDescriptor(descriptor.as_ptr()) } == 0 {
        return Err(io::Error::other(
            "Windows object has an invalid security descriptor",
        ));
    }
    // SAFETY: both SIDs come from validated, live security buffers.
    if owner.is_null()
        || unsafe { IsValidSid(owner) } == 0
        || unsafe { EqualSid(owner, expected.user_sid()) } == 0
    {
        return Err(io::Error::other(
            "Windows object is not owned by the current user",
        ));
    }

    let mut control = 0u16;
    let mut revision = 0u32;
    // SAFETY: descriptor is valid and both output pointers are writable.
    if unsafe { GetSecurityDescriptorControl(descriptor.as_ptr(), &mut control, &mut revision) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    if control & SE_DACL_PROTECTED == 0 {
        return Err(io::Error::other(
            "Windows object DACL inherits permissions instead of being protected",
        ));
    }

    let mut dacl_present = 0;
    let mut dacl_defaulted = 0;
    let mut descriptor_dacl: *mut ACL = null_mut();
    // SAFETY: descriptor is valid and all output pointers are writable.
    if unsafe {
        GetSecurityDescriptorDacl(
            descriptor.as_ptr(),
            &mut dacl_present,
            &mut descriptor_dacl,
            &mut dacl_defaulted,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if dacl_present == 0 || dacl.is_null() || descriptor_dacl.is_null() || dacl != descriptor_dacl {
        return Err(io::Error::other(
            "Windows object has no verifiable owner-only DACL",
        ));
    }
    // SAFETY: descriptor_dacl belongs to the live, valid descriptor.
    if unsafe { IsValidAcl(descriptor_dacl) } == 0 {
        return Err(io::Error::other("Windows object has an invalid DACL"));
    }
    // SAFETY: IsValidAcl validated the ACL header.
    if unsafe { (*descriptor_dacl).AceCount } != 1 {
        return Err(io::Error::other("Windows object DACL is not owner-only"));
    }

    let mut raw_ace: *mut c_void = null_mut();
    // SAFETY: the ACL is valid, has exactly one ACE, and raw_ace is writable.
    if unsafe { GetAce(descriptor_dacl, 0, &mut raw_ace) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if raw_ace.is_null() {
        return Err(io::Error::other("Windows GetAce returned a null ACE"));
    }
    let acl_start = descriptor_dacl as usize;
    let acl_header_end = acl_start
        .checked_add(size_of::<ACL>())
        .ok_or_else(|| io::Error::other("Windows ACL header size overflow"))?;
    // SAFETY: IsValidAcl validated the ACL header.
    let acl_end = acl_start
        .checked_add(usize::from(unsafe { (*descriptor_dacl).AclSize }))
        .ok_or_else(|| io::Error::other("Windows ACL size overflow"))?;
    let ace_start = raw_ace as usize;
    let header_end = ace_start
        .checked_add(size_of::<ACE_HEADER>())
        .ok_or_else(|| io::Error::other("Windows ACE header size overflow"))?;
    if ace_start < acl_header_end || header_end > acl_end {
        return Err(io::Error::other("Windows ACE header is outside its ACL"));
    }
    // SAFETY: the complete header is inside the validated ACL; unaligned reads
    // avoid imposing alignment requirements on the foreign buffer.
    let header = unsafe { raw_ace.cast::<ACE_HEADER>().read_unaligned() };
    let ace_size = usize::from(header.AceSize);
    let ace_end = ace_start
        .checked_add(ace_size)
        .ok_or_else(|| io::Error::other("Windows ACE size overflow"))?;
    if ace_end > acl_end {
        return Err(io::Error::other("Windows access ACE is outside its ACL"));
    }
    // SAFETY: the ACE starts inside the live ACL and its complete declared size
    // was checked against that allocation above.
    let ace_bytes = unsafe { std::slice::from_raw_parts(raw_ace.cast::<u8>(), ace_size) };
    let layout = parse_access_ace_layout(ace_bytes)?;
    let expected_flags = if kind == HandleKind::Directory {
        DIRECTORY_ACE_FLAGS
    } else {
        0
    };
    if layout.ace_type != ACCESS_ALLOWED_ACE_TYPE as u8
        || layout.ace_flags != expected_flags
        || layout.mask != FILE_ALL_ACCESS
    {
        return Err(io::Error::other(
            "Windows object DACL is not the required owner-only ACL",
        ));
    }
    // SAFETY: the parser proved the complete SID is within `ace_bytes`.
    let ace_sid = unsafe { raw_ace.cast::<u8>().add(layout.sid_offset) }.cast::<c_void>();
    // SAFETY: every byte claimed by the SID header was bounds-checked first.
    if unsafe { IsValidSid(ace_sid) } == 0
        || unsafe { GetLengthSid(ace_sid) } as usize != layout.sid_size
        || unsafe { EqualSid(ace_sid, expected.user_sid()) } == 0
    {
        return Err(io::Error::other(
            "Windows object DACL grants access to a non-owner SID",
        ));
    }
    Ok(())
}

struct AccessAceLayout {
    ace_type: u8,
    ace_flags: u8,
    mask: u32,
    sid_offset: usize,
    sid_size: usize,
}

fn parse_access_ace_layout(bytes: &[u8]) -> io::Result<AccessAceLayout> {
    if bytes.len() < size_of::<ACE_HEADER>() {
        return Err(io::Error::other("Windows access ACE header is truncated"));
    }
    // SAFETY: the slice contains a complete header and unaligned reads impose
    // no additional alignment requirement.
    let header = unsafe { bytes.as_ptr().cast::<ACE_HEADER>().read_unaligned() };
    let ace_size = usize::from(header.AceSize);
    if ace_size != bytes.len() {
        return Err(io::Error::other(
            "Windows access ACE exceeds its containing ACL",
        ));
    }
    const SID_HEADER_BYTES: usize = 8;
    let sid_offset = size_of::<ACE_HEADER>() + size_of::<u32>();
    if ace_size < sid_offset + SID_HEADER_BYTES {
        return Err(io::Error::other("Windows access ACE SID is truncated"));
    }
    // SAFETY: the fixed mask and SID header are inside the checked slice.
    let mask = unsafe {
        bytes
            .as_ptr()
            .add(size_of::<ACE_HEADER>())
            .cast::<u32>()
            .read_unaligned()
    };
    let sub_authorities = usize::from(bytes[sid_offset + 1]);
    let sid_size = sub_authorities
        .checked_mul(size_of::<u32>())
        .and_then(|length| SID_HEADER_BYTES.checked_add(length))
        .ok_or_else(|| io::Error::other("Windows SID size overflow"))?;
    if sid_offset.checked_add(sid_size) != Some(ace_size) {
        return Err(io::Error::other(
            "Windows access ACE has an invalid SID boundary",
        ));
    }
    Ok(AccessAceLayout {
        ace_type: header.AceType,
        ace_flags: header.AceFlags,
        mask,
        sid_offset,
        sid_size,
    })
}

#[cfg(test)]
pub(super) fn validate_access_ace_layout_for_test(bytes: &[u8]) -> io::Result<()> {
    parse_access_ace_layout(bytes).map(|_| ())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HandleKind {
    Directory,
    File,
}

struct OwnerOnlySecurity {
    descriptor: LocalAllocation,
    token_information: Vec<usize>,
}

impl OwnerOnlySecurity {
    fn new(kind: HandleKind) -> io::Result<Self> {
        let token = current_process_token()?;
        let token_information = token_user_information(&token)?;
        let user_sid = token_user_sid(&token_information)?;
        let sid_string = sid_string(user_sid)?;
        let ace_flags = if kind == HandleKind::Directory {
            "OICI"
        } else {
            ""
        };
        let sddl = format!("O:{sid_string}D:P(A;{ace_flags};FA;;;{sid_string})");
        let sddl_wide = wide_string(&sddl)?;
        let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: sddl_wide is NUL-terminated and descriptor is writable.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl_wide.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            descriptor: LocalAllocation::new(descriptor.cast())?,
            token_information,
        })
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor.as_ptr(),
            bInheritHandle: 0,
        }
    }

    fn user_sid(&self) -> PSID {
        // Construction validated this token information and its SID.
        token_user_sid(&self.token_information).expect("validated TOKEN_USER")
    }
}

fn current_process_token() -> io::Result<OwnedHandle> {
    let mut token: HANDLE = null_mut();
    // SAFETY: GetCurrentProcess returns a process pseudo-handle and token is a
    // valid output pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenProcessToken returned a unique owned handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(token as RawHandle) })
}

fn token_user_information(token: &OwnedHandle) -> io::Result<Vec<usize>> {
    let mut required = 0u32;
    // SAFETY: a zero-sized first call is the documented way to query size.
    let first = unsafe {
        GetTokenInformation(
            token.as_raw_handle() as HANDLE,
            TokenUser,
            null_mut(),
            0,
            &mut required,
        )
    };
    if first != 0 || required == 0 {
        return Err(io::Error::other(
            "Windows token user size query returned an invalid result",
        ));
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error().map(|code| code as u32) != Some(ERROR_INSUFFICIENT_BUFFER) {
        return Err(error);
    }
    let word_count = (required as usize).div_ceil(size_of::<usize>());
    let mut buffer = vec![0usize; word_count];
    // SAFETY: buffer is aligned for TOKEN_USER and contains at least `required`
    // writable bytes.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle() as HANDLE,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    token_user_sid(&buffer)?;
    Ok(buffer)
}

fn token_user_sid(token_information: &[usize]) -> io::Result<PSID> {
    if token_information.len() * size_of::<usize>() < size_of::<TOKEN_USER>() {
        return Err(io::Error::other("Windows TOKEN_USER buffer is too short"));
    }
    // SAFETY: the buffer is sufficiently large and aligned for TOKEN_USER.
    let token_user = unsafe { &*(token_information.as_ptr().cast::<TOKEN_USER>()) };
    let sid = token_user.User.Sid;
    // SAFETY: the SID pointer is supplied by GetTokenInformation and remains
    // backed by token_information.
    if sid.is_null() || unsafe { IsValidSid(sid) } == 0 {
        return Err(io::Error::other(
            "Windows token contains an invalid user SID",
        ));
    }
    Ok(sid)
}

fn sid_string(sid: PSID) -> io::Result<String> {
    let mut raw = null_mut();
    // SAFETY: sid was validated and raw is a writable output pointer.
    if unsafe { ConvertSidToStringSidW(sid, &mut raw) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let raw = LocalAllocation::new(raw.cast())?;
    let wide = raw.as_ptr().cast::<u16>();
    let length = (0..256usize)
        // SAFETY: ConvertSidToStringSidW returns a NUL-terminated SID string;
        // the documented SID syntax is far shorter than this defensive bound.
        .find(|offset| unsafe { *wide.add(*offset) } == 0)
        .ok_or_else(|| io::Error::other("Windows SID string is not NUL-terminated"))?;
    // SAFETY: the allocation contains at least length initialized u16 values.
    String::from_utf16(unsafe { std::slice::from_raw_parts(wide, length) })
        .map_err(|_| io::Error::other("Windows SID string is not valid UTF-16"))
}

struct LocalAllocation(*mut c_void);

impl LocalAllocation {
    fn new(pointer: *mut c_void) -> io::Result<Self> {
        if pointer.is_null() {
            Err(io::Error::other(
                "Windows API returned a null local allocation",
            ))
        } else {
            Ok(Self(pointer))
        }
    }

    fn as_ptr(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: this pointer came from a Windows API documented to allocate
        // with LocalAlloc, and this RAII owner frees it exactly once.
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
    wide_units(path.as_os_str().encode_wide())
}

fn wide_string(value: &str) -> io::Result<Vec<u16>> {
    wide_units(value.encode_utf16())
}

fn wide_units(units: impl Iterator<Item = u16>) -> io::Result<Vec<u16>> {
    let mut wide = units.collect::<Vec<_>>();
    if wide.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows path or security descriptor contains NUL",
        ));
    }
    wide.push(0);
    Ok(wide)
}

fn raw_handle(file: &File) -> HANDLE {
    file.as_raw_handle() as HANDLE
}
