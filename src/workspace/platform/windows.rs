//! Native directory-relative opens and handle-based enumeration; edits are isolated.
#![allow(unsafe_code)]
use super::super::Opened;
use std::{
    ffi::{OsString, c_void},
    fs::{File, OpenOptions},
    io,
    os::windows::{
        ffi::OsStringExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle},
    },
    path::{Component, Path, Prefix},
    ptr::null_mut,
};
type Handle = *mut c_void;
#[path = "windows_edit.rs"]
mod edit;
pub use edit::*;
#[repr(C)]
struct Unicode {
    length: u16,
    maximum: u16,
    buffer: *mut u16,
}
#[repr(C)]
struct Attributes {
    length: u32,
    root: Handle,
    name: *mut Unicode,
    flags: u32,
    security: Handle,
    quality: Handle,
}
#[repr(C)]
struct Status {
    status: usize,
    information: usize,
}
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtCreateFile(
        file: *mut Handle,
        access: u32,
        attributes: *mut Attributes,
        status: *mut Status,
        allocation: *mut i64,
        flags: u32,
        share: u32,
        disposition: u32,
        options: u32,
        ea: Handle,
        ea_length: u32,
    ) -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetFileInformationByHandleEx(file: Handle, class: u32, buffer: Handle, size: u32) -> i32;
}
pub fn root(path: &Path) -> io::Result<File> {
    if !matches!(path.components().next(), Some(Component::Prefix(p))
        if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
    {
        return Err(io::ErrorKind::Unsupported.into());
    }
    let file = OpenOptions::new()
        .read(true)
        .share_mode(3)
        .custom_flags(0x02000000 | 0x00200000)
        .open(path)?; // BACKUP_SEMANTICS | OPEN_REPARSE_POINT.
    if !ordinary(&file, true)? {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(file)
}
fn ordinary(file: &File, directory: bool) -> io::Result<bool> {
    let metadata = file.metadata()?;
    Ok(metadata.file_attributes() & 0x400 == 0
        && if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        })
}
pub fn open(root: &File, path: &str, directory: bool) -> io::Result<Opened> {
    if path == "." {
        if !directory {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        return Ok(Opened {
            file: child(root, "", true)?,
            _parents: Vec::new(),
        });
    }
    let parts: Vec<_> = path.split('/').collect();
    let mut parents = Vec::new();
    for (index, name) in parts.iter().enumerate() {
        let parent = parents.last().unwrap_or(root);
        let last = index + 1 == parts.len();
        let file = child(parent, name, !last || directory)?;
        if last {
            return Ok(Opened {
                file,
                _parents: parents,
            });
        }
        parents.push(file);
    }
    Err(io::ErrorKind::InvalidInput.into())
}
fn child(parent: &File, name: &str, directory: bool) -> io::Result<File> {
    child_options(
        parent,
        name,
        directory,
        0x00100081,
        if directory { 3 } else { 7 },
        1,
    )
}
fn child_options(
    parent: &File,
    name: &str,
    directory: bool,
    access: u32,
    share: u32,
    disposition: u32,
) -> io::Result<File> {
    // Each open has a single validated component. Final reparses are opened as
    // objects and rejected; they are never followed into another namespace.
    let mut wide: Vec<u16> = name.encode_utf16().collect();
    let length = u16::try_from(wide.len() * 2).map_err(|_| io::ErrorKind::InvalidInput)?;
    let mut unicode = Unicode {
        length,
        maximum: length,
        buffer: wide.as_mut_ptr(),
    };
    let mut attributes = Attributes {
        length: size_of::<Attributes>() as u32,
        root: parent.as_raw_handle(),
        name: &mut unicode,
        flags: 0x40,
        security: null_mut(),
        quality: null_mut(),
    };
    let mut status = Status {
        status: 0,
        information: 0,
    };
    let mut handle = null_mut();
    // SAFETY: all ABI buffers live through this synchronous call. Rights and
    // disposition are selected by owned callers, never supplied by model input.
    let result = unsafe {
        NtCreateFile(
            &mut handle,
            access,
            &mut attributes,
            &mut status,
            null_mut(),
            0,
            share,
            disposition,
            0x20 | 0x00200000 | if directory { 1 } else { 0x40 },
            null_mut(),
            0,
        )
    };
    if result < 0 {
        // SAFETY: translates an NTSTATUS value without dereferencing memory.
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(result) } as i32,
        ));
    }
    // SAFETY: successful NtCreateFile transfers exactly one owned handle.
    let file = unsafe { File::from_raw_handle(handle) };
    if !ordinary(&file, directory)? {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(file)
}
pub fn names(file: &File, visit: &mut dyn FnMut(OsString) -> bool) -> io::Result<()> {
    // u64 storage supplies the alignment required by FILE_FULL_DIR_INFO.
    let mut storage = [0u64; 8192];
    let mut class = 15; // FileFullDirectoryRestartInfo, then FileFullDirectoryInfo.
    loop {
        storage.fill(0);
        // SAFETY: valid directory and writable aligned 64 KiB buffer.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                class,
                storage.as_mut_ptr().cast(),
                size_of_val(&storage) as u32,
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(18) {
                Ok(())
            } else {
                Err(error)
            };
        }
        class = 14;
        // SAFETY: view initialized storage as bytes, never as unaligned structs.
        let bytes = unsafe {
            std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(), size_of_val(&storage))
        };
        let mut at = 0usize;
        loop {
            let record = bytes
                .get(at..)
                .filter(|v| v.len() >= 68)
                .ok_or(io::ErrorKind::InvalidData)?;
            let next = u32::from_le_bytes(record[0..4].try_into().unwrap()) as usize;
            let length = u32::from_le_bytes(record[60..64].try_into().unwrap()) as usize;
            if !length.is_multiple_of(2)
                || next != 0 && (next < 68 + length || !next.is_multiple_of(8))
            {
                return Err(io::ErrorKind::InvalidData.into());
            }
            let name = record
                .get(68..68 + length)
                .ok_or(io::ErrorKind::InvalidData)?;
            let name: Vec<u16> = name
                .chunks_exact(2)
                .map(|v| u16::from_le_bytes([v[0], v[1]]))
                .collect();
            if !visit(OsString::from_wide(&name)) {
                return Ok(());
            }
            if next == 0 {
                break;
            }
            at = at.checked_add(next).ok_or(io::ErrorKind::InvalidData)?;
        }
    }
}
