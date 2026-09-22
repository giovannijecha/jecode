//! ABI declarations for the documented kernel32 process/job/pipe boundary.
#![allow(unsafe_code)]
use std::{
    ffi::c_void,
    io,
    os::windows::io::{FromRawHandle, OwnedHandle},
};
pub type Handle = *mut c_void;
#[repr(C)]
pub struct Security {
    pub size: u32,
    pub descriptor: Handle,
    pub inherit: i32,
}
#[repr(C)]
#[derive(Default)]
pub struct Startup {
    pub size: u32,
    pub reserved: Handle,
    pub desktop: Handle,
    pub title: Handle,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub columns: u32,
    pub rows: u32,
    pub fill: u32,
    pub flags: u32,
    pub show: u16,
    pub reserved_size: u16,
    pub reserved_bytes: Handle,
    pub input: Handle,
    pub output: Handle,
    pub error: Handle,
    pub attributes: Handle,
}
#[repr(C)]
#[derive(Default)]
pub struct ProcessInfo {
    pub process: Handle,
    pub thread: Handle,
    pub pid: u32,
    pub tid: u32,
}
#[repr(C)]
#[derive(Default)]
pub struct Limits {
    pub process_time: i64,
    pub job_time: i64,
    pub flags: u32,
    pub min_working: usize,
    pub max_working: usize,
    pub active: u32,
    pub affinity: usize,
    pub priority: u32,
    pub scheduling: u32,
    pub io: [u64; 6],
    pub process_memory: usize,
    pub job_memory: usize,
    pub peak_process_memory: usize,
    pub peak_job_memory: usize,
}
#[repr(C)]
#[derive(Default)]
pub struct Accounting {
    pub times: [i64; 4],
    pub faults: u32,
    pub total: u32,
    pub active: u32,
    pub terminated: u32,
}
#[repr(C)]
pub struct ProcessIds {
    pub assigned: u32,
    pub count: u32,
    pub ids: [usize; 256],
}
#[link(name = "kernel32")]
unsafe extern "system" {
    pub fn GetSystemDirectoryW(buffer: *mut u16, length: u32) -> u32;
    pub fn CreateJobObjectW(security: Handle, name: *const u16) -> Handle;
    pub fn SetInformationJobObject(job: Handle, class: u32, info: Handle, bytes: u32) -> i32;
    pub fn QueryInformationJobObject(
        job: Handle,
        class: u32,
        info: Handle,
        bytes: u32,
        returned: Handle,
    ) -> i32;
    pub fn TerminateJobObject(job: Handle, code: u32) -> i32;
    pub fn CreatePipe(
        read: *mut Handle,
        write: *mut Handle,
        security: *mut Security,
        size: u32,
    ) -> i32;
    pub fn SetHandleInformation(handle: Handle, mask: u32, flags: u32) -> i32;
    pub fn CreateFileW(
        path: *const u16,
        access: u32,
        share: u32,
        security: *mut Security,
        creation: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    pub fn PeekNamedPipe(
        handle: Handle,
        buffer: Handle,
        length: u32,
        read: Handle,
        available: *mut u32,
        left: Handle,
    ) -> i32;
    pub fn InitializeProcThreadAttributeList(
        list: Handle,
        count: u32,
        flags: u32,
        bytes: *mut usize,
    ) -> i32;
    pub fn UpdateProcThreadAttribute(
        list: Handle,
        flags: u32,
        attribute: usize,
        value: Handle,
        bytes: usize,
        previous: Handle,
        returned: Handle,
    ) -> i32;
    pub fn DeleteProcThreadAttributeList(list: Handle);
    pub fn CreateProcessW(
        app: *const u16,
        command: *mut u16,
        process_security: Handle,
        thread_security: Handle,
        inherit: i32,
        flags: u32,
        environment: Handle,
        cwd: *const u16,
        startup: *mut Startup,
        result: *mut ProcessInfo,
    ) -> i32;
    pub fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    pub fn GetExitCodeProcess(handle: Handle, code: *mut u32) -> i32;
    pub fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    pub fn IsProcessInJob(process: Handle, job: Handle, result: *mut i32) -> i32;
}
pub fn owned(handle: Handle) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle as isize == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: caller passes a successful newly created owned kernel handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}
pub fn checked(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
