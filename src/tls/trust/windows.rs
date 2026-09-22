//! Read-only Crypt32 store access. Cryptography and path validation stay owned.
#![allow(unsafe_code)]
use crate::tls::crypto::sha256::Sha256;
use std::{collections::HashSet, ffi::c_void, io, mem, ptr, slice};
const NOT_FOUND: u32 = 0x80092004;
const READ_ONLY: u32 = 0x8000;
const OPEN_EXISTING: u32 = 0x4000;
const CURRENT_USER: u32 = 0x10000;
const LOCAL_MACHINE: u32 = 0x20000;
#[repr(C)]
struct Context {
    encoding: u32,
    encoded: *const u8,
    length: u32,
    info: *const c_void,
    store: *mut c_void,
}
#[repr(C)]
struct Usage {
    count: u32,
    identifiers: *const *const u8,
}
#[link(name = "crypt32")]
unsafe extern "system" {
    fn CertOpenStore(
        provider: *const u8,
        encoding: u32,
        crypt: usize,
        flags: u32,
        parameter: *const c_void,
    ) -> *mut c_void;
    fn CertCloseStore(store: *mut c_void, flags: u32) -> i32;
    fn CertEnumCertificatesInStore(store: *mut c_void, previous: *const Context) -> *const Context;
    fn CertFreeCertificateContext(context: *const Context) -> i32;
    fn CertGetEnhancedKeyUsage(
        context: *const Context,
        flags: u32,
        usage: *mut Usage,
        length: *mut u32,
    ) -> i32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(error: u32);
}
struct Store {
    handle: *mut c_void,
    current: *const Context,
}
impl Drop for Store {
    fn drop(&mut self) {
        // SAFETY: Store owns its current enumeration context and store handle.
        unsafe {
            if !self.current.is_null() {
                CertFreeCertificateContext(self.current);
            }
            CertCloseStore(self.handle, 0);
        }
    }
}
pub(super) fn load() -> io::Result<super::TrustStore> {
    let mut roots = Vec::new();
    let mut denied = HashSet::new();
    for location in [CURRENT_USER, LOCAL_MACHINE] {
        for certificate in read("Disallowed", location, false)? {
            denied.insert(Sha256::digest(&certificate));
        }
        for certificate in read("ROOT", location, true)? {
            if !roots.contains(&certificate) {
                roots.push(certificate);
            }
        }
    }
    Ok(super::TrustStore { roots, denied })
}
fn read(name: &str, location: u32, filter_usage: bool) -> io::Result<Vec<Vec<u8>>> {
    let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
    // SAFETY: Wincrypt CERT_STORE_PROV_SYSTEM_W is the integer resource 10;
    // the terminated UTF-16 name remains alive through the synchronous call.
    let handle = unsafe {
        CertOpenStore(
            10_usize as *const u8,
            0,
            0,
            location | READ_ONLY | OPEN_EXISTING,
            name.as_ptr().cast(),
        )
    };
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut store = Store {
        handle,
        current: ptr::null(),
    };
    let mut output = Vec::new();
    let mut count = 0;
    let mut total = 0;
    loop {
        // SAFETY: enumeration consumes the old context even on failure. Store
        // records the returned owner immediately, so Drop never double-frees.
        store.current = unsafe { CertEnumCertificatesInStore(handle, store.current) };
        if store.current.is_null() {
            let error = unsafe { GetLastError() };
            if error == NOT_FOUND {
                break;
            }
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        count += 1;
        // SAFETY: Crypt32 owns a live immutable context until the next call.
        let context = unsafe { &*store.current };
        total += context.length as usize;
        if count > 4096
            || total > 16 * 1024 * 1024
            || context.length == 0
            || context.length > 65_536
            || context.encoded.is_null()
        {
            return Err(io::Error::other("native certificate store exceeds bounds"));
        }
        if filter_usage && !server_usage(store.current)? {
            continue;
        }
        // SAFETY: checked bounds; Crypt32 guarantees the encoded buffer's size.
        output.push(
            unsafe { slice::from_raw_parts(context.encoded, context.length as usize) }.to_vec(),
        );
    }
    Ok(output)
}
fn server_usage(context: *const Context) -> io::Result<bool> {
    let mut bytes = 0;
    // SAFETY: live certificate context, null output requests the required size.
    if unsafe { CertGetEnhancedKeyUsage(context, 0, ptr::null_mut(), &mut bytes) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if bytes < mem::size_of::<Usage>() as u32 || bytes > 65_536 {
        return Err(io::Error::other("native EKU bounds"));
    }
    // usize allocation provides the pointer alignment required by CTL_USAGE.
    let mut buffer = vec![0_usize; (bytes as usize).div_ceil(mem::size_of::<usize>())];
    let capacity = bytes as usize;
    // SAFETY: aligned writable storage, with capacity at least the requested bytes.
    let (success, error) = unsafe {
        SetLastError(0);
        let ok = CertGetEnhancedKeyUsage(context, 0, buffer.as_mut_ptr().cast(), &mut bytes);
        (ok, GetLastError())
    };
    if success == 0 {
        return Err(io::Error::from_raw_os_error(error as i32));
    }
    if bytes as usize > capacity {
        return Err(io::Error::other("native EKU changed size"));
    }
    // SAFETY: successful call initialized the fixed header in aligned storage.
    let usage = unsafe { &*buffer.as_ptr().cast::<Usage>() };
    if usage.count == 0 {
        return Ok(error == NOT_FOUND);
    }
    let start = buffer.as_ptr() as usize;
    let end = start + bytes as usize;
    let pointers = usage.identifiers as usize;
    if usage.count > 256
        || pointers < start
        || !pointers.is_multiple_of(mem::align_of::<usize>())
        || pointers
            .checked_add(usage.count as usize * mem::size_of::<usize>())
            .is_none_or(|n| n > end)
    {
        return Err(io::Error::other("native EKU pointer bounds"));
    }
    // SAFETY: complete aligned pointer table lies inside initialized output.
    for oid in unsafe { slice::from_raw_parts(usage.identifiers, usage.count as usize) } {
        let address = *oid as usize;
        if !(start..end).contains(&address) {
            return Err(io::Error::other("native EKU string bounds"));
        }
        // SAFETY: bounded inside initialized output; no unbounded C-string scan.
        let text = unsafe { slice::from_raw_parts(*oid, (end - address).min(128)) };
        let Some(length) = text.iter().position(|b| *b == 0) else {
            return Err(io::Error::other("native EKU string length"));
        };
        if &text[..length] == b"1.3.6.1.5.5.7.3.1" || &text[..length] == b"2.5.29.37.0" {
            return Ok(true);
        }
    }
    Ok(false)
}
