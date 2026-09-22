//! Retained-buffer clearing. This cannot erase compiler copies, registers or swap.

use std::{
    fmt,
    sync::atomic::{Ordering, compiler_fence},
};

// This is the retained-buffer unsafe boundary. Callers supply owned
// integer storage. Volatile stores keep its final clearing observable to LLVM.
#[allow(unsafe_code)]
pub(in crate::tls) fn erase<T: Copy + Default>(values: &mut [T]) {
    for value in values {
        // SAFETY: The exclusive reference names initialized, aligned live storage.
        // Copy has no destructor; Default supplies a valid replacement value.
        unsafe { std::ptr::write_volatile(value, T::default()) };
    }
    compiler_fence(Ordering::SeqCst);
}

pub(in crate::tls) fn equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0;
    for (&a, &b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    std::hint::black_box(difference) == 0
}

pub struct Secret(pub(in crate::tls) [u8; 32]);

impl Secret {
    pub(in crate::tls) fn zero() -> Self {
        Self([0; 32])
    }
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
    pub(in crate::tls) fn as_mut_bytes(&mut self) -> &mut [u8; 32] {
        &mut self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        erase(&mut self.0);
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}
