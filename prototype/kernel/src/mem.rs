//! The core's heap boundary.
//!
//! Everything that crosses between the host and the core as bytes is framed the
//! same way: a 4-byte little-endian length followed by the UTF-8 payload. One
//! framing rule means the host has exactly one way to read a buffer and one way
//! to write one, and a length mismatch is a single place to look.
//!
//! Buffers are allocated with an explicit layout, not through `Vec`, so the host
//! can allocate in the core's memory by calling the exported `kernel_alloc` and
//! the core can release it with the same size. Ownership never moves across the
//! boundary implicitly.
use std::alloc::{alloc as raw_alloc, dealloc, Layout};

/// Alignment for every buffer crossing the boundary.
const ALIGN: usize = 8;

/// Bytes of framing ahead of every payload.
pub const PREFIX: usize = 4;

/// Allocate `len` bytes in the core's linear memory.
///
/// A zero-length request still gets one byte, so the returned pointer is never
/// null and never aliases the heap's empty allocation.
pub fn alloc(len: usize) -> *mut u8 {
    let layout = Layout::from_size_align(len.max(1), ALIGN).expect("buffer layout");
    unsafe { raw_alloc(layout) }
}

/// Release a buffer from [`alloc`].
///
/// # Safety
///
/// `ptr` must come from [`alloc`] and `len` must be the value it was allocated
/// with; freeing with a different length is undefined.
pub unsafe fn free(ptr: *mut u8, len: usize) {
    let layout = Layout::from_size_align(len.max(1), ALIGN).expect("buffer layout");
    unsafe { dealloc(ptr, layout) };
}

/// Frame `body` into a freshly allocated buffer and hand it out.
///
/// The receiver owns the buffer and must release it with [`take_frame`] or
/// [`free`] using `PREFIX + body.len()`.
pub fn write_frame(body: &[u8]) -> *mut u8 {
    let ptr = alloc(PREFIX + body.len());
    unsafe {
        std::ptr::copy_nonoverlapping((body.len() as u32).to_le_bytes().as_ptr(), ptr, PREFIX);
        if !body.is_empty() {
            std::ptr::copy_nonoverlapping(body.as_ptr(), ptr.add(PREFIX), body.len());
        }
    }
    ptr
}

/// Read a framed buffer and release it in one step.
///
/// The length comes from the buffer's own prefix, so the caller does not have to
/// keep a second copy of it in sync.
///
/// # Safety
///
/// `ptr` must come from [`write_frame`] or [`alloc`] with at least the length its
/// prefix declares plus [`PREFIX`].
pub unsafe fn take_frame(ptr: *mut u8) -> Vec<u8> {
    unsafe {
        let length =
            u32::from_le_bytes([*ptr, *ptr.add(1), *ptr.add(2), *ptr.add(3)]) as usize;
        let body = std::slice::from_raw_parts(ptr.add(PREFIX), length).to_vec();
        free(ptr, PREFIX + length);
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_round_trips_and_is_released() {
        let frame = write_frame(b"{\"ok\":true}");
        let body = unsafe { take_frame(frame) };
        assert_eq!(body, b"{\"ok\":true}");
    }

    #[test]
    fn an_empty_body_still_has_a_pointer_and_a_prefix() {
        let frame = write_frame(b"");
        assert!(!frame.is_null());
        let body = unsafe { take_frame(frame) };
        assert!(body.is_empty());
    }
}
