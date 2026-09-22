//! Owned Windows test fixture. Only kernel32 supplies console/process primitives.
#![allow(unsafe_code)]
use std::{
    ffi::c_void,
    fs::File,
    io::{Read, Write},
    os::windows::io::{AsRawHandle, FromRawHandle},
    ptr::null_mut,
    sync::{Arc, Mutex},
    thread::JoinHandle,
};
type Handle = *mut c_void;
#[repr(C)]
struct Security {
    size: u32,
    descriptor: Handle,
    inherit: i32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Coord {
    x: i16,
    y: i16,
}
#[repr(C)]
#[derive(Default)]
struct Rect {
    left: i16,
    top: i16,
    right: i16,
    bottom: i16,
}
#[repr(C)]
#[derive(Default)]
struct Info {
    size: Coord,
    cursor: Coord,
    attributes: u16,
    window: Rect,
    max: Coord,
}
#[repr(C)]
struct Startup {
    cb: u32,
    reserved: Handle,
    desktop: Handle,
    title: Handle,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    columns: u32,
    rows: u32,
    fill: u32,
    flags: u32,
    show: u16,
    reserved_size: u16,
    reserved_bytes: Handle,
    input: Handle,
    output: Handle,
    error: Handle,
    attributes: Handle,
}
#[repr(C)]
#[derive(Default)]
struct Process {
    process: Handle,
    thread: Handle,
    pid: u32,
    tid: u32,
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreatePipe(read: *mut Handle, write: *mut Handle, security: *mut Security, size: u32)
    -> i32;
    fn CreatePseudoConsole(
        size: Coord,
        input: Handle,
        output: Handle,
        flags: u32,
        console: *mut Handle,
    ) -> i32;
    fn ResizePseudoConsole(console: Handle, size: Coord) -> i32;
    fn ClosePseudoConsole(console: Handle);
    fn CloseHandle(handle: Handle) -> i32;
    fn InitializeProcThreadAttributeList(
        list: Handle,
        count: u32,
        flags: u32,
        bytes: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        list: Handle,
        flags: u32,
        attribute: usize,
        value: Handle,
        size: usize,
        previous: Handle,
        returned: Handle,
    ) -> i32;
    fn DeleteProcThreadAttributeList(list: Handle);
    fn CreateProcessW(
        app: *const u16,
        command: *mut u16,
        process_security: Handle,
        thread_security: Handle,
        inherit: i32,
        flags: u32,
        environment: Handle,
        cwd: *const u16,
        startup: *mut Startup,
        process: *mut Process,
    ) -> i32;
    fn WaitForSingleObject(handle: Handle, timeout: u32) -> u32;
    fn TerminateProcess(handle: Handle, code: u32) -> i32;
    fn GetConsoleScreenBufferInfo(handle: Handle, info: *mut Info) -> i32;
    fn ReadConsoleOutputCharacterW(
        handle: Handle,
        chars: *mut u16,
        count: u32,
        coord: Coord,
        read: *mut u32,
    ) -> i32;
    fn SetStdHandle(id: u32, handle: Handle) -> i32;
}
fn pipe() -> (File, File) {
    let (mut read, mut write) = (null_mut(), null_mut());
    // SAFETY: valid handle outputs, default non-inherited pipe security.
    assert_ne!(
        unsafe { CreatePipe(&mut read, &mut write, null_mut(), 0) },
        0
    );
    // SAFETY: each newly created handle transfers to exactly one File owner.
    unsafe { (File::from_raw_handle(read), File::from_raw_handle(write)) }
}
pub struct Console {
    console: Handle,
    process: Handle,
    pub input: File,
    reader: Option<JoinHandle<()>>,
    output: Arc<Mutex<Vec<u8>>>,
}
impl Console {
    pub fn start(directory: &std::path::Path) -> Self {
        Self::start_named(directory, "native_console_child")
    }
    pub fn start_named(directory: &std::path::Path, test: &str) -> Self {
        let (input_read, input) = pipe();
        let (mut output_read, output_write) = pipe();
        let output = Arc::new(Mutex::new(Vec::new()));
        let received = output.clone();
        let reader = std::thread::spawn(move || {
            let mut chunk = [0; 8192];
            while let Ok(count) = output_read.read(&mut chunk) {
                if count == 0 {
                    break;
                }
                let mut all = received.lock().unwrap();
                if all.len() < 4_194_304 {
                    all.extend_from_slice(&chunk[..count]);
                }
            }
        });
        let mut result = Self {
            console: null_mut(),
            process: null_mut(),
            input,
            reader: Some(reader),
            output,
        };
        // SAFETY: live pipe handles and writable HPCON output. Reader drains concurrently.
        let created = unsafe {
            CreatePseudoConsole(
                Coord { x: 120, y: 36 },
                input_read.as_raw_handle(),
                output_write.as_raw_handle(),
                0,
                &mut result.console,
            )
        };
        // Release local writers before any assertion can unwind into the reader join.
        drop(input_read);
        drop(output_write);
        assert_eq!(created, 0);
        let mut bytes = 0;
        // SAFETY: documented sizing query, then usize-aligned allocation for opaque list.
        unsafe {
            InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes);
        }
        let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let list = storage.as_mut_ptr().cast();
        // SAFETY: initialized list lives until after CreateProcessW.
        unsafe {
            assert_ne!(InitializeProcThreadAttributeList(list, 1, 0, &mut bytes), 0);
            assert_ne!(
                UpdateProcThreadAttribute(
                    list,
                    0,
                    0x20016,
                    result.console,
                    size_of::<Handle>(),
                    null_mut(),
                    null_mut()
                ),
                0
            );
        }
        let executable = std::env::current_exe().unwrap();
        let mut command: Vec<u16> =
            format!("\"{}\" --exact {test} --nocapture", executable.display())
                .encode_utf16()
                .chain([0])
                .collect();
        let mut vars: Vec<(String, String)> = std::env::vars()
            .filter(|(k, _)| {
                !k.eq_ignore_ascii_case("JECODE_TUI_TEST_DIR")
                    && !k.eq_ignore_ascii_case("JECODE_TUI_TRACE")
                    && !k.eq_ignore_ascii_case("NO_COLOR")
            })
            .collect();
        vars.push((
            "JECODE_TUI_TEST_DIR".into(),
            directory.to_str().unwrap().into(),
        ));
        vars.push(("NO_COLOR".into(), "".into()));
        vars.push((
            "JECODE_TUI_TRACE".into(),
            directory.join("geometry.log").to_str().unwrap().into(),
        ));
        vars.sort_by_key(|(k, _)| k.to_uppercase());
        let mut environment: Vec<u16> = vars
            .into_iter()
            .flat_map(|(k, v)| format!("{k}={v}\0").encode_utf16().collect::<Vec<_>>())
            .collect();
        environment.push(0);
        // SAFETY: zero is valid for all optional STARTUPINFOEX fields; explicit size/list below.
        let mut startup: Startup = unsafe { std::mem::zeroed() };
        startup.cb = size_of::<Startup>() as u32;
        startup.attributes = list;
        let mut process = Process::default();
        // SAFETY: NUL-terminated mutable command/environment; structure and list remain live.
        let ok = unsafe {
            CreateProcessW(
                std::ptr::null(),
                command.as_mut_ptr(),
                null_mut(),
                null_mut(),
                0,
                0x80400,
                environment.as_mut_ptr().cast(),
                std::ptr::null(),
                &mut startup,
                &mut process,
            )
        };
        // SAFETY: initialized list is no longer needed by CreateProcessW.
        unsafe {
            DeleteProcThreadAttributeList(list);
        }
        assert_ne!(ok, 0, "{}", std::io::Error::last_os_error());
        result.process = process.process;
        // SAFETY: no thread operations are needed; process handle remains owned.
        unsafe {
            CloseHandle(process.thread);
        }
        result
    }
    pub fn resize(&self, columns: i16, rows: i16) {
        // SAFETY: owned live console and positive test dimensions.
        assert_eq!(
            unsafe {
                ResizePseudoConsole(
                    self.console,
                    Coord {
                        x: columns,
                        y: rows,
                    },
                )
            },
            0
        );
    }
    pub fn output(&self) -> String {
        let bytes = self.output.lock().unwrap();
        match std::str::from_utf8(&bytes) {
            Ok(text) => text.to_owned(),
            Err(error) if error.error_len().is_none() => {
                // A pipe read can end inside a scalar; expose only the stable prefix.
                std::str::from_utf8(&bytes[..error.valid_up_to()])
                    .unwrap()
                    .to_owned()
            }
            Err(error) => panic!("invalid console UTF-8: {error}"),
        }
    }
}
impl Drop for Console {
    fn drop(&mut self) {
        let _ = self.input.write_all(&[17]);
        // SAFETY: owned handles only, process reaped before console closure; reader keeps draining.
        unsafe {
            if !self.process.is_null() {
                if WaitForSingleObject(self.process, 3000) != 0 {
                    TerminateProcess(self.process, 1);
                    WaitForSingleObject(self.process, 3000);
                }
                CloseHandle(self.process);
            }
            if !self.console.is_null() {
                ClosePseudoConsole(self.console);
            }
        }
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
    }
}
pub fn snapshot() -> String {
    let console = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONOUT$")
        .unwrap();
    let mut info = Info::default();
    // SAFETY: correctly sized native output; handle opened in the attached child console.
    assert_ne!(
        unsafe { GetConsoleScreenBufferInfo(console.as_raw_handle(), &mut info) },
        0
    );
    let mut result = format!(
        "{} {} {} {}\n",
        info.size.x, info.size.y, info.window.top, info.window.bottom
    );
    for row in 0..info.size.y {
        let mut chars = vec![0u16; info.size.x as usize];
        let mut count = 0;
        // SAFETY: initialized UTF-16 buffer, valid coordinates, count bounded by allocation.
        assert_ne!(
            unsafe {
                ReadConsoleOutputCharacterW(
                    console.as_raw_handle(),
                    chars.as_mut_ptr(),
                    chars.len() as u32,
                    Coord { x: 0, y: row },
                    &mut count,
                )
            },
            0
        );
        result.push_str(String::from_utf16_lossy(&chars[..count as usize]).trim_end());
        result.push('\n');
    }
    result
}
pub fn bind_test_io() -> [File; 3] {
    let files = ["CONIN$", "CONOUT$", "CONOUT$"].map(|name| {
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(name)
            .unwrap()
    });
    for (id, file) in [-10i32, -11, -12].into_iter().zip(&files) {
        // SAFETY: dedicated child process only; Files keep redirected console handles live.
        assert_ne!(unsafe { SetStdHandle(id as u32, file.as_raw_handle()) }, 0);
    }
    files
}
