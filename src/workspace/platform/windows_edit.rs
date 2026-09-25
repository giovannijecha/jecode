//! Synchronous NT handle-relative operations; no absolute model paths or helpers.
use super::*;

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSetInformationFile(
        file: Handle,
        status: *mut Status,
        info: Handle,
        size: u32,
        class: u32,
    ) -> i32;
}
#[link(name = "advapi32")]
unsafe extern "system" {
    fn GetKernelObjectSecurity(
        file: Handle,
        flags: u32,
        buffer: Handle,
        size: u32,
        needed: *mut u32,
    ) -> i32;
    fn SetKernelObjectSecurity(file: Handle, flags: u32, buffer: Handle) -> i32;
    fn GetSecurityDescriptorControl(
        descriptor: Handle,
        control: *mut u16,
        revision: *mut u32,
    ) -> i32;
}
pub fn identity(file: &File) -> io::Result<(u64, u64)> {
    // FILE_ID_INFO includes a volume serial and a 128-bit ID. Our supported
    // ordinary local filesystems also expose the full stable 64-bit file index.
    let mut data = [0u64; 3];
    // SAFETY: aligned 24-byte FILE_ID_INFO output and valid handle.
    if unsafe {
        GetFileInformationByHandleEx(file.as_raw_handle(), 18, data.as_mut_ptr().cast(), 24)
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if data[2] != 0 {
        return Err(io::ErrorKind::Unsupported.into());
    }
    Ok((data[0], data[1]))
}
pub fn editable(file: &File) -> io::Result<()> {
    let mut standard = [0u64; 3];
    // SAFETY: FILE_STANDARD_INFO needs 24 aligned writable bytes.
    if unsafe {
        GetFileInformationByHandleEx(file.as_raw_handle(), 1, standard.as_mut_ptr().cast(), 24)
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let links = standard[2] as u32;
    let flags = file.metadata()?.file_attributes();
    // Readonly, hidden, system, sparse, compressed, encrypted, reparse, offline.
    if links != 1 || flags & (1 | 2 | 4 | 0x200 | 0x400 | 0x800 | 0x1000 | 0x4000) != 0 {
        return Err(io::ErrorKind::Unsupported.into());
    }
    // Reject alternate streams; replacing them with an ordinary file would lose data.
    let mut streams = [0u64; 512];
    // SAFETY: aligned FILE_STREAM_INFO buffer. Overflow is rejected, not ignored.
    if unsafe {
        GetFileInformationByHandleEx(file.as_raw_handle(), 7, streams.as_mut_ptr().cast(), 4096)
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let bytes = unsafe { std::slice::from_raw_parts(streams.as_ptr().cast::<u8>(), 4096) };
    let next = u32::from_le_bytes(bytes[..4].try_into().unwrap());
    let len = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let expected: Vec<u8> = "::$DATA"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    if next != 0 || len != expected.len() || bytes[24..24 + len] != expected {
        return Err(io::ErrorKind::Unsupported.into());
    }
    Ok(())
}
pub fn edit_open(parent: &File, name: &str) -> io::Result<File> {
    // READ_DATA | READ_ATTRIBUTES | READ_CONTROL | DELETE | SYNCHRONIZE.
    // No write/delete sharing while a validated file is being published.
    child_options(parent, name, false, 0x00130081, 1, 1)
}
pub fn create(parent: &File, name: &str) -> io::Result<File> {
    // READ/WRITE_DATA, attributes, READ_CONTROL, WRITE_DAC, DELETE, SYNCHRONIZE.
    child_options(parent, name, false, 0x00170183, 1, 2)
}
pub fn metadata_to(source: &File, target: &File) -> io::Result<()> {
    apply_policy(target, &capture_policy(source)?)
}
pub fn capture_policy(source: &File) -> io::Result<Vec<u8>> {
    let mut size = 0;
    // SAFETY: size query only; credentials/security descriptor never enter logs.
    unsafe { GetKernelObjectSecurity(source.as_raw_handle(), 4, null_mut(), 0, &mut size) };
    if size == 0 || size > 65536 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut data = vec![0u64; (size as usize).div_ceil(8)];
    // SAFETY: initialized aligned storage of the OS-reported size, live handles.
    if unsafe {
        GetKernelObjectSecurity(
            source.as_raw_handle(),
            4,
            data.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), size as usize) };
    Ok(bytes.to_vec())
}
pub fn apply_policy(target: &File, policy: &[u8]) -> io::Result<()> {
    if policy.is_empty() || policy.len() > 65536 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut data = vec![0u64; policy.len().div_ceil(8)];
    unsafe { std::slice::from_raw_parts_mut(data.as_mut_ptr().cast::<u8>(), policy.len()) }
        .copy_from_slice(policy);
    let (mut control, mut revision) = (0, 0);
    // SAFETY: returned initialized security descriptor and scalar outputs.
    if unsafe {
        GetSecurityDescriptorControl(data.as_mut_ptr().cast(), &mut control, &mut revision)
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let flags = 4 | if control & 0x1000 != 0 {
        0x80000000
    } else {
        0x20000000
    };
    // SAFETY: preserve DACL and its inheritance protection before content is written.
    if unsafe { SetKernelObjectSecurity(target.as_raw_handle(), flags, data.as_mut_ptr().cast()) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
pub fn move_new(parent: &File, file: &File, _: &str, to: &str) -> io::Result<()> {
    #[repr(C)]
    struct Rename {
        replace: u8,
        root: Handle,
        length: u32,
        name: [u16; 1],
    }
    let wide: Vec<u16> = to.encode_utf16().collect();
    let offset = std::mem::offset_of!(Rename, name);
    let size = size_of::<Rename>() + wide.len() * 2;
    let mut storage = vec![0usize; size.div_ceil(size_of::<usize>())];
    // SAFETY: pointer-aligned storage has room for the fixed header and UTF-16 name.
    unsafe {
        let info = storage.as_mut_ptr().cast::<Rename>();
        (*info).root = parent.as_raw_handle();
        (*info).length = (wide.len() * 2) as u32;
        std::ptr::copy_nonoverlapping(
            wide.as_ptr(),
            storage.as_mut_ptr().cast::<u8>().add(offset).cast(),
            wide.len(),
        );
    }
    set(file, storage.as_mut_ptr().cast(), size as u32, 10)
}
pub fn remove_owned(_: &File, file: &File, _: &str) -> io::Result<()> {
    let mut delete = 1u8;
    set(file, (&mut delete as *mut u8).cast(), 1, 13)
}
pub fn sync_parent(_: &File) -> io::Result<()> {
    // Windows does not expose a reliable directory flush for this held handle.
    Ok(())
}
fn set(file: &File, info: Handle, size: u32, class: u32) -> io::Result<()> {
    let mut status = Status {
        status: 0,
        information: 0,
    };
    // SAFETY: callers provide a live owned handle and initialized class-specific buffer.
    let result =
        unsafe { NtSetInformationFile(file.as_raw_handle(), &mut status, info, size, class) };
    if result < 0 {
        return Err(io::Error::from_raw_os_error(
            unsafe { RtlNtStatusToDosError(result) } as i32,
        ));
    }
    Ok(())
}
