//! Ordinary JSON files with a protected DACL for the current user and SYSTEM.
#![allow(unsafe_code)]
use std::{
    ffi::c_void,
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr::null_mut,
};
type Handle = *mut c_void;
#[repr(C)]
struct Attributes {
    length: u32,
    security: Handle,
    inherit: i32,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn LocalFree(value: Handle) -> Handle;
    fn CreateDirectoryW(path: *const u16, attributes: *const Attributes) -> i32;
    fn CreateFileW(
        path: *const u16,
        access: u32,
        share: u32,
        attributes: *mut Attributes,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn GetFileInformationByHandleEx(file: Handle, class: u32, buffer: Handle, size: u32) -> i32;
}
#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn GetTokenInformation(
        token: Handle,
        class: u32,
        data: Handle,
        size: u32,
        needed: *mut u32,
    ) -> i32;
    fn ConvertSidToStringSidW(sid: Handle, text: *mut *mut u16) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        text: *const u16,
        revision: u32,
        descriptor: *mut Handle,
        size: *mut u32,
    ) -> i32;
    fn SetFileSecurityW(path: *const u16, information: u32, descriptor: Handle) -> i32;
}
struct Local(Handle);
impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: these non-null pointers are returned by LocalAlloc-backed Windows APIs.
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn checked(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn descriptor() -> io::Result<Local> {
    let mut raw = null_mut();
    // SAFETY: valid out pointer; process pseudo-handle need not be closed.
    checked(unsafe { OpenProcessToken(GetCurrentProcess(), 8, &mut raw) })?;
    // SAFETY: successful OpenProcessToken transfers a unique handle.
    let token = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut needed = 0;
    // SAFETY: documented sizing call, no data buffer.
    unsafe {
        GetTokenInformation(token.as_raw_handle(), 1, null_mut(), 0, &mut needed);
    }
    if needed == 0 || needed > 65_536 {
        return Err(io::Error::other("invalid user token size"));
    }
    let mut data = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
    // SAFETY: aligned buffer is at least needed bytes; class 1 returns TOKEN_USER.
    checked(unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            1,
            data.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    })?;
    let sid = data[0] as Handle;
    let mut text = null_mut();
    // SAFETY: first TOKEN_USER field is a SID pointer into the still-live buffer.
    checked(unsafe { ConvertSidToStringSidW(sid, &mut text) })?;
    let text_owner = Local(text.cast());
    let mut length = 0;
    // SAFETY: API returns a null-terminated SID string; Windows SID text is bounded.
    unsafe {
        while length < 256 && *text.add(length) != 0 {
            length += 1;
        }
    }
    if length == 256 {
        return Err(io::Error::other("invalid user SID"));
    }
    // SAFETY: length was checked against the returned string terminator.
    let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) })
        .map_err(|_| io::Error::other("invalid user SID"))?;
    drop(text_owner);
    let sddl: Vec<u16> = format!("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;{sid})")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut raw = null_mut();
    // SAFETY: null-terminated SDDL and valid out pointer; returned allocation is owned.
    checked(unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut raw, null_mut())
    })?;
    Ok(Local(raw))
}
fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
    if value.contains(&0) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    value.push(0);
    Ok(value)
}
pub fn directory(path: &Path) -> io::Result<()> {
    let security = descriptor()?;
    let attrs = Attributes {
        length: size_of::<Attributes>() as u32,
        security: security.0,
        inherit: 0,
    };
    // SAFETY: path and descriptor remain live throughout this synchronous call.
    checked(unsafe { CreateDirectoryW(wide(path)?.as_ptr(), &attrs) })
}
pub fn protect_directory(path: &Path) -> io::Result<()> {
    let security = descriptor()?;
    // SAFETY: valid directory path and security descriptor; DACL is protected from inheritance.
    checked(unsafe { SetFileSecurityW(wide(path)?.as_ptr(), 4 | 0x80000000, security.0) })
}
pub fn read(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(1 | 4)
        .custom_flags(0x00200000)
        .open(path)
}
pub fn create(path: &Path, existing: bool) -> io::Result<File> {
    let security = descriptor()?;
    let mut attrs = Attributes {
        length: size_of::<Attributes>() as u32,
        security: security.0,
        inherit: 0,
    };
    // SAFETY: owned descriptor and UTF-16 path; handle is checked before transferring it.
    let raw = unsafe {
        CreateFileW(
            wide(path)?.as_ptr(),
            0x80000000 | 0x40000000,
            1 | 2,
            &mut attrs,
            if existing { 4 } else { 1 },
            0x00200000,
            null_mut(),
        )
    };
    if raw as isize == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful CreateFileW returns a unique owned handle.
    Ok(unsafe { File::from_raw_handle(raw) })
}
pub fn check_file(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    let mut standard = [0u64; 3];
    // SAFETY: FileStandardInfo is 24 bytes and the buffer has its required alignment.
    checked(unsafe {
        GetFileInformationByHandleEx(file.as_raw_handle(), 1, standard.as_mut_ptr().cast(), 24)
    })?;
    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 || standard[2] as u32 != 1 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}
pub fn sync_directory(_: &Path) -> io::Result<()> {
    Ok(())
}
