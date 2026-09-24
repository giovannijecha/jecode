//! Windows 10+: attach to a kill-on-close job during creation, before any code
//! runs. Only the three stdio handles are inherited. No console window is opened.
#![allow(unsafe_code)]
#[path = "windows_ffi.rs"]
mod ffi;
use crate::{
    command::{Channel, Exit, shell},
    workspace::Directory,
};
use ffi::*;
use std::{
    fs::File,
    io::{self, Read},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, OwnedHandle},
    },
    ptr::{null, null_mut},
    thread,
    time::Duration,
};

struct Attributes(Vec<usize>);
impl Attributes {
    fn new(handles: &mut [Handle; 3], job: &mut Handle) -> io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: size query writes only the size, followed by aligned storage.
        unsafe {
            InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut bytes);
        }
        if bytes == 0 || bytes > 65536 {
            return Err(io::Error::other("invalid process attribute size"));
        }
        let mut list = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        // SAFETY: allocated capacity covers the exact returned size.
        unsafe {
            checked(InitializeProcThreadAttributeList(
                list.as_mut_ptr().cast(),
                2,
                0,
                &mut bytes,
            ))?;
        }
        let mut attributes = Self(list);
        // SAFETY: both arrays remain alive until CreateProcess returns. Documented
        // PROC_THREAD_ATTRIBUTE_HANDLE_LIST and PROC_THREAD_ATTRIBUTE_JOB_LIST.
        unsafe {
            checked(UpdateProcThreadAttribute(
                attributes.ptr(),
                0,
                0x20002,
                handles.as_mut_ptr().cast(),
                size_of_val(handles),
                null_mut(),
                null_mut(),
            ))?;
            checked(UpdateProcThreadAttribute(
                attributes.ptr(),
                0,
                0x2000d,
                (job as *mut Handle).cast(),
                size_of::<Handle>(),
                null_mut(),
                null_mut(),
            ))?;
        }
        Ok(attributes)
    }
    fn ptr(&mut self) -> Handle {
        self.0.as_mut_ptr().cast()
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        // SAFETY: successfully initialized attribute list, deleted once before storage.
        unsafe {
            DeleteProcThreadAttributeList(self.ptr());
        }
    }
}
fn pipe(security: &mut Security) -> io::Result<(File, OwnedHandle)> {
    let (mut read, mut write) = (null_mut(), null_mut());
    // SAFETY: writable handle outputs and initialized inheritable security.
    unsafe {
        checked(CreatePipe(&mut read, &mut write, security, 0))?;
    }
    let (read, write) = (owned(read)?, owned(write)?);
    // SAFETY: only the write end is inherited; the reader stays with this worker.
    unsafe {
        checked(SetHandleInformation(read.as_raw_handle(), 1, 0))?;
    }
    Ok((File::from(read), write))
}
pub(in crate::command) struct Process {
    process: OwnedHandle,
    job: OwnedHandle,
    stdout: File,
    stderr: File,
    finished: Option<Exit>,
}
impl Process {
    pub fn spawn(script: &str, directory: &Directory) -> io::Result<Self> {
        if !directory.file().metadata()?.is_dir() {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let process_path = directory.process_path()?;
        let mut system = vec![0u16; 32768];
        // SAFETY: writable UTF-16 buffer. Use the OS directory, not a PATH search.
        let len = unsafe { GetSystemDirectoryW(system.as_mut_ptr(), system.len() as u32) } as usize;
        if len == 0 || len >= system.len() {
            return Err(io::Error::last_os_error());
        }
        system.truncate(len);
        system.extend("\\WindowsPowerShell\\v1.0\\powershell.exe".encode_utf16());
        let app: Vec<u16> = system.iter().copied().chain([0]).collect();
        let mut command: Vec<u16> = [34]
            .into_iter()
            .chain(system)
            .chain([34])
            .chain(
                format!(
                    " -NoLogo -NoProfile -NonInteractive -EncodedCommand {}",
                    shell::encoded(script)
                )
                .encode_utf16(),
            )
            .chain([0])
            .collect();
        let cwd: Vec<u16> = process_path.as_os_str().encode_wide().chain([0]).collect();
        let mut security = Security {
            size: size_of::<Security>() as u32,
            descriptor: null_mut(),
            inherit: 1,
        };
        let (stdout, stdout_write) = pipe(&mut security)?;
        let (stderr, stderr_write) = pipe(&mut security)?;
        // SAFETY: fixed NUL device, inheritable read handle; all created handles get owners.
        let input = owned(unsafe {
            CreateFileW(
                [78u16, 85, 76, 0].as_ptr(),
                0x80000000,
                3,
                &mut security,
                3,
                0,
                null_mut(),
            )
        })?;
        let job = owned(unsafe { CreateJobObjectW(null_mut(), null()) })?;
        let mut limits = Limits {
            flags: 0x2000 | 8,
            active: 256,
            ..Limits::default()
        }; // KILL_ON_JOB_CLOSE | ACTIVE_PROCESS
        unsafe {
            checked(SetInformationJobObject(
                job.as_raw_handle(),
                9,
                (&mut limits as *mut Limits).cast(),
                size_of::<Limits>() as u32,
            ))?;
        }
        let mut handles = [
            input.as_raw_handle(),
            stdout_write.as_raw_handle(),
            stderr_write.as_raw_handle(),
        ];
        let mut job_handle = job.as_raw_handle();
        let mut attributes = Attributes::new(&mut handles, &mut job_handle)?;
        let mut startup = Startup {
            size: size_of::<Startup>() as u32,
            flags: 0x100,
            input: handles[0],
            output: handles[1],
            error: handles[2],
            attributes: attributes.ptr(),
            ..Startup::default()
        };
        let mut result = ProcessInfo::default();
        // SAFETY: UTF-16 NUL-terminated paths and writable command, live attribute
        // backing values, STARTUPINFOEXW. CREATE_NO_WINDOW|EXTENDED_STARTUPINFO_PRESENT.
        unsafe {
            checked(CreateProcessW(
                app.as_ptr(),
                command.as_mut_ptr(),
                null_mut(),
                null_mut(),
                1,
                0x08000000 | 0x00080000,
                null_mut(),
                cwd.as_ptr(),
                &mut startup,
                &mut result,
            ))?;
        }
        let process = owned(result.process)?;
        drop(owned(result.thread)?);
        Ok(Self {
            process,
            job,
            stdout,
            stderr,
            finished: None,
        })
    }
    pub fn read(&mut self, channel: Channel, buffer: &mut [u8]) -> io::Result<usize> {
        let pipe = match channel {
            Channel::Stdout => &mut self.stdout,
            Channel::Stderr => &mut self.stderr,
        };
        let mut available = 0;
        // SAFETY: synchronous pipe with a single reader; read only available bytes.
        if unsafe {
            PeekNamedPipe(
                pipe.as_raw_handle(),
                null_mut(),
                0,
                null_mut(),
                &mut available,
                null_mut(),
            )
        } == 0
        {
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(109) {
                Ok(0)
            } else {
                Err(error)
            };
        }
        let count = buffer.len().min(available as usize);
        if count == 0 {
            Ok(0)
        } else {
            pipe.read(&mut buffer[..count])
        }
    }
    pub fn exited(&mut self) -> io::Result<bool> {
        // SAFETY: owned process handle. A zero timeout never blocks the output loop.
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            0 => Ok(true),
            258 => Ok(false),
            _ => Err(io::Error::last_os_error()),
        }
    }
    pub fn finish(&mut self) -> io::Result<Exit> {
        if let Some(exit) = self.finished {
            return Ok(exit);
        }
        // A job's active count can reach zero before all terminating processes
        // have released their handles. Hold and wait on individual members too.
        let members = self.members();
        // SAFETY: owned non-inherited job; includes ordinary descendants from launch.
        unsafe {
            checked(TerminateJobObject(self.job.as_raw_handle(), 1))?;
        }
        if unsafe { WaitForSingleObject(self.process.as_raw_handle(), u32::MAX) } != 0 {
            return Err(io::Error::last_os_error());
        }
        loop {
            let mut accounting = Accounting::default();
            unsafe {
                checked(QueryInformationJobObject(
                    self.job.as_raw_handle(),
                    1,
                    (&mut accounting as *mut Accounting).cast(),
                    size_of::<Accounting>() as u32,
                    null_mut(),
                ))?;
            }
            if accounting.active == 0 {
                break;
            }
            thread::sleep(Duration::from_millis(8));
        }
        for member in members? {
            // SAFETY: query-only owned handle verified as a member of this job.
            if unsafe { WaitForSingleObject(member.as_raw_handle(), u32::MAX) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        let mut code = 0;
        unsafe {
            checked(GetExitCodeProcess(self.process.as_raw_handle(), &mut code))?;
        }
        let exit = Exit {
            code: Some(i64::from(code)),
            signal: None,
        };
        self.finished = Some(exit);
        Ok(exit)
    }
    fn members(&self) -> io::Result<Vec<OwnedHandle>> {
        let mut ids = ProcessIds {
            assigned: 0,
            count: 0,
            ids: [0; 256],
        };
        // SAFETY: ABI-sized buffer covers the job's active process limit.
        unsafe {
            checked(QueryInformationJobObject(
                self.job.as_raw_handle(),
                3,
                (&mut ids as *mut ProcessIds).cast(),
                size_of::<ProcessIds>() as u32,
                null_mut(),
            ))?;
        }
        if ids.count > 256 || ids.assigned > ids.count {
            return Err(io::Error::other("incomplete job membership"));
        }
        let mut members = Vec::new();
        for &pid in &ids.ids[..ids.count as usize] {
            // Query plus synchronize only. Check membership against PID recycling.
            let member = match owned(unsafe { OpenProcess(0x00101000, 0, pid as u32) }) {
                Ok(member) => member,
                Err(error) if error.raw_os_error() == Some(87) => continue,
                Err(error) => return Err(error),
            };
            let mut belongs = 0;
            unsafe {
                checked(IsProcessInJob(
                    member.as_raw_handle(),
                    self.job.as_raw_handle(),
                    &mut belongs,
                ))?;
            }
            if belongs != 0 {
                members.push(member);
            }
        }
        Ok(members)
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}
