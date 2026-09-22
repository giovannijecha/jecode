#![cfg(windows)]
#![allow(unsafe_code)]
#[path = "support/workspace.rs"]
mod support;
use jecode::workspace::{Budget, Error, Workspace};
use std::{
    ffi::c_void,
    fs::{self, OpenOptions},
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[link(name = "kernel32")]
unsafe extern "system" {
    fn DeviceIoControl(
        file: *mut c_void,
        code: u32,
        input: *const c_void,
        input_size: u32,
        output: *mut c_void,
        output_size: u32,
        returned: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
}

// Owned test setup through the OS reparse API. Both endpoints are isolated fixtures.
fn junction(link: &Path, destination: &Path) {
    fs::create_dir(link).unwrap();
    let destination = destination.canonicalize().unwrap();
    let spelling = destination.to_string_lossy();
    let spelling = format!(r"\??\{}", spelling.strip_prefix(r"\\?\").unwrap());
    let mut target: Vec<u16> = std::ffi::OsStr::new(&spelling).encode_wide().collect();
    let length = (target.len() * 2) as u16;
    target.extend([0, 0]);
    let mut bytes = Vec::new();
    bytes.extend(0xa0000003u32.to_le_bytes()); // IO_REPARSE_TAG_MOUNT_POINT.
    bytes.extend((8 + target.len() as u16 * 2).to_le_bytes());
    for value in [0, 0, length, length + 2, 0] {
        bytes.extend(value.to_le_bytes());
    }
    for value in target {
        bytes.extend(value.to_le_bytes());
    }
    let file = OpenOptions::new()
        .write(true)
        .custom_flags(0x02200000)
        .open(link)
        .unwrap();
    let mut returned = 0;
    // SAFETY: synchronous call with a live directory handle and initialized buffer.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            0x000900a4,
            bytes.as_ptr().cast(),
            bytes.len() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    assert_ne!(ok, 0, "{}", std::io::Error::last_os_error());
}

#[test]
fn junctions_cannot_escape_and_selected_root_cannot_be_replaced() {
    let fixture = support::Fixture::new();
    fixture.write("root/owned.txt", "owned");
    fixture.write("outside/private.txt", "outside");
    junction(&fixture.0.join("root/link"), &fixture.0.join("outside"));
    let workspace = Workspace::open(&fixture.0.join("root")).unwrap();
    let cancelled = AtomicBool::new(false);
    let budget = Budget {
        cancelled: &cancelled,
        deadline: Instant::now() + Duration::from_secs(5),
    };
    assert_eq!(workspace.read("owned.txt", &budget).unwrap(), "owned");
    assert!(
        workspace
            .prepare_create("link/new.txt", "new", &budget)
            .is_err()
    );
    assert!(
        workspace
            .prepare_edit("link/private.txt", "outside", "changed", &budget)
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("outside/private.txt")).unwrap(),
        "outside"
    );
    assert!(!fixture.0.join("outside/new.txt").exists());
    assert_eq!(
        workspace.read("link/private.txt", &budget),
        Err(Error::Unavailable)
    );
    let list = workspace.list(".", &budget).unwrap();
    assert_eq!(list.entries.len(), 1);
    assert_eq!(list.omitted, 1);
    assert!(fs::rename(fixture.0.join("root"), fixture.0.join("replaced")).is_err());
    assert_eq!(workspace.read("owned.txt", &budget).unwrap(), "owned");
}
