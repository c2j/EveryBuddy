//! The C ABI the host loads.
//!
//! Five exports and one import for the whole core. A language that can produce a
//! wasm module with these exports and that can call `env.host_call` can be the
//! core; nothing above this file knows the host exists.
//!
//! - `kernel_alloc(len) -> ptr`, `kernel_free(ptr, len)` — heap in the core's
//!   memory, so the host can place a request where the core can read it.
//! - `kernel_boot(ptr, len) -> ptr` — build the runtime, the rutis root context,
//!   and the first provider/consumer pair.
//! - `kernel_request(ptr, len) -> ptr` — route one request to the core.
//! - `kernel_version() -> u32` — packed `major.minor.patch`, so a host can refuse
//!   a module it does not understand before calling anything else.
//!
//! Every reply is a framed JSON [`Response`]. A malformed request is a reply
//! with an error, not a trap: the host has to be able to distinguish "the core
//! rejected this" from "the core is gone".
//!
//! Panics are **not** recovered here. The release profile aborts on panic, so an
//! unrecovered panic traps the module and ends it. The host treats a trap as a
//! dead core and rebuilds the instance; pretending a half-initialised kernel is
//! still usable would be worse than saying so.
use std::cell::RefCell;

use serde::Deserialize;

use crate::kernel::Kernel;
use crate::mem;
use crate::protocol::{Request, Response};
use crate::search::SearchLimits;

/// Packed `major.minor.patch`, derived from [`crate::kernel::VERSION`] by the
/// same split a host would use, so the two can never drift.
#[unsafe(no_mangle)]
pub extern "C" fn kernel_version() -> u32 {
    let mut parts = crate::kernel::VERSION.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
    let major = parts.next().unwrap_or(0);
    let minor = parts.next().unwrap_or(0);
    let patch = parts.next().unwrap_or(0);
    (major << 16) | (minor << 8) | patch
}

/// Allocate `len` bytes in the core's memory for the host to write into.
///
/// The host must release the buffer with `kernel_free` and the same `len`.
#[unsafe(no_mangle)]
pub extern "C" fn kernel_alloc(len: usize) -> *mut u8 {
    mem::alloc(len)
}

/// Release a buffer previously returned by [`kernel_alloc`].
///
/// # Safety
///
/// `ptr` must come from [`kernel_alloc`] with the same `len`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kernel_free(ptr: *mut u8, len: usize) {
    if !ptr.is_null() {
        unsafe { mem::free(ptr, len) };
    }
}

/// Arguments for [`kernel_boot`].
#[derive(Debug, Deserialize)]
struct BootConfig {
    /// Directory the sweep plugin scans and the default root for searches.
    root: String,
    /// Patterns for the first search provider.
    patterns: Vec<String>,
    /// Optional bounds; absent bounds are [`SearchLimits::default`].
    #[serde(default)]
    limits: Option<SearchLimits>,
}

thread_local! {
    /// The one kernel this module instance owns. wasm is single-threaded, so a
    /// thread-local is the whole of the synchronisation needed.
    static KERNEL: RefCell<Option<Kernel>> = const { RefCell::new(None) };
}

/// Build the core. `ptr`/`len` carry a JSON [`BootConfig`].
///
/// Booting a second time replaces the first kernel, which is what makes the ABI
/// usable from a test harness that wants a fresh core without a fresh instance.
#[unsafe(no_mangle)]
pub extern "C" fn kernel_boot(ptr: *const u8, len: usize) -> *mut u8 {
    std::panic::set_hook(Box::new(|info| {
        crate::host::log("error", &format!("kernel panicked: {info}"));
    }));

    let config = unsafe { std::slice::from_raw_parts(ptr, len) };
    let config: BootConfig = match serde_json::from_slice(config) {
        Ok(config) => config,
        Err(error) => {
            return mem::write_frame(
                serde_json::to_vec(&Response::fail(0, "bad_config", format!("boot config: {error}")))
                    .unwrap_or_default()
                    .as_slice(),
            );
        }
    };

    let limits = config.limits.unwrap_or_default();
    match Kernel::boot(config.root.into(), config.patterns, limits) {
        Ok(kernel) => {
            let replaced = KERNEL.with(|cell| cell.borrow_mut().replace(kernel).is_some());
            let states = KERNEL.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .map(Kernel::states)
                    .unwrap_or_default()
            });
            crate::host::log("info", "kernel booted");
            mem::write_frame(
                serde_json::to_vec(&Response::ok(
                    0,
                    serde_json::json!({ "replaced": replaced, "plugins": states }),
                ))
                .unwrap_or_default()
                .as_slice(),
            )
        }
        Err(message) => mem::write_frame(
            serde_json::to_vec(&Response::fail(0, "boot_failed", message))
                .unwrap_or_default()
                .as_slice(),
        ),
    }
}

/// Route one request. `ptr`/`len` carry a JSON [`Request`].
///
/// A request that arrives before [`kernel_boot`], or a second request re-entering
/// while one is being served, is answered with an error rather than a trap: the
/// host may call back into the core from `host_call`, and that must be a
/// diagnosable mistake rather than a crash.
#[unsafe(no_mangle)]
pub extern "C" fn kernel_request(ptr: *const u8, len: usize) -> *mut u8 {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let request: Request = match serde_json::from_slice(bytes) {
        Ok(request) => request,
        Err(error) => return reply(Response::fail(0, "bad_json", format!("request: {error}"))),
    };
    let id = request.id;

    let response = KERNEL.with(|cell| {
        let mut slot = match cell.try_borrow_mut() {
            Ok(slot) => slot,
            Err(_) => {
                return Response::fail(
                    id,
                    "reentrant",
                    "a request arrived while another was being served",
                );
            }
        };
        match slot.as_mut() {
            Some(kernel) => kernel.handle(request),
            None => Response::fail(id, "not_booted", "kernel_boot has not run"),
        }
    });

    reply(response)
}

/// Frame one response for the host.
fn reply(response: Response) -> *mut u8 {
    let body = serde_json::to_vec(&response).unwrap_or_else(|error| {
        // Serialisation of our own reply types cannot fail for a reason the host
        // could act on, but the host still needs a parseable answer.
        format!("{{\"id\":0,\"ok\":false,\"error\":{{\"code\":\"encode_failed\",\"message\":\"{error}\"}}}}")
            .into_bytes()
    });
    mem::write_frame(&body)
}

/// Shut the core down and drop it. A later request reports `not_booted`.
#[unsafe(no_mangle)]
pub extern "C" fn kernel_shutdown() {
    KERNEL.with(|cell| {
        let mut slot = cell.borrow_mut();
        if let Some(mut kernel) = slot.take() {
            kernel.shutdown();
        }
    });
}
