//! rutis on wasm32-unknown-unknown: compile is not enough, the fiber driver
//! needs a tokio runtime. This probe boots a current-thread runtime, builds a
//! root `Ctx`, and runs rutis's own headline example (a provider that is
//! replaced, with a dependency-gated consumer) inside wasm.
//!
//! Every observation goes to the host through `host_log`, because a wasm trap
//! would otherwise lose it.
use std::alloc::{alloc, dealloc, Layout};
use std::sync::Arc;

use rutis::{BoxFuture, CordisError, Ctx, Effect, Plugin, Typed, TypedPlugin};

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn host_log(ptr: *const u8, len: usize);
}

fn log(msg: &str) {
    unsafe { host_log(msg.as_ptr(), msg.len()) }
}

#[unsafe(no_mangle)]
pub extern "C" fn kernel_alloc(len: usize) -> *mut u8 {
    unsafe { alloc(Layout::from_size_align(len.max(1), 8).unwrap()) }
}

#[unsafe(no_mangle)]
pub extern "C" fn kernel_free(ptr: *mut u8, len: usize) {
    unsafe { dealloc(ptr, Layout::from_size_align(len.max(1), 8).unwrap()) }
}

/// 4-byte little-endian length, then UTF-8.
fn respond(body: &[u8]) -> *mut u8 {
    let ptr = kernel_alloc(4 + body.len());
    unsafe {
        std::ptr::copy_nonoverlapping((body.len() as u32).to_le_bytes().as_ptr(), ptr, 4);
        std::ptr::copy_nonoverlapping(body.as_ptr(), ptr.add(4), body.len());
    }
    ptr
}

/// A service is a type.
struct Greeting(String);

struct Greeter(&'static str);

impl Plugin for Greeter {
    fn name(&self) -> &str {
        "greeter"
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            ctx.provide(Greeting(format!("hello from {}", self.0)))?;
            log(&format!("  [wasm] supplier started: {}", self.0));
            Ok(Effect::Done)
        })
    }
}

/// Depends on `Greeting`: starts when it appears, restarts when it is replaced.
struct Listener;

impl TypedPlugin for Listener {
    type Deps = (Arc<Greeting>,);

    fn name(&self) -> &str {
        "listener"
    }

    fn apply<'a>(
        &'a self,
        _: &'a Ctx,
        (greeting,): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            log(&format!("  [wasm] consumer ran -> {}", greeting.0));
            Ok(Effect::Done)
        })
    }
}

/// The kernel keeps one runtime for its whole life; each host call drives it.
fn kernel_runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("kernel runtime")
    })
}

fn run() -> Result<String, String> {
    kernel_runtime().block_on(async {
        let ctx = Ctx::root().map_err(|e| format!("Ctx::root failed: {e}"))?;
        let listener = ctx.plugin(Typed::new(Listener));
        let english = ctx.plugin(Greeter("English"));
        (&english).await.map_err(|e| format!("english: {e}"))?;
        (&listener).await.map_err(|e| format!("listener: {e}"))?;

        // Provider replacement must evict and restart the consumer.
        english.dispose().await.map_err(|e| format!("dispose: {e}"))?;
        let esperanto = ctx.plugin(Greeter("Esperanto"));
        (&esperanto).await.map_err(|e| format!("esperanto: {e}"))?;
        (&listener).await.map_err(|e| format!("listener reload: {e}"))?;

        let state = format!("{:?}", listener.state());
        Ok(format!("lifecycle ok, consumer state={state}"))
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn boot() -> *mut u8 {
    std::panic::set_hook(Box::new(|info| {
        log(&format!("  [wasm] PANIC: {info}"));
    }));
    let report = match run() {
        Ok(s) => s,
        Err(e) => format!("ERROR: {e}"),
    };
    respond(report.as_bytes())
}

/// Can the guest reach the real filesystem through WASI preopens, rather than
/// round-tripping every read through a host call?
#[unsafe(no_mangle)]
pub extern "C" fn probe_fs(root_ptr: *const u8, root_len: usize) -> *mut u8 {
    let root = unsafe { std::slice::from_raw_parts(root_ptr, root_len) };
    let root = String::from_utf8_lossy(root);
    let report = match std::fs::read_dir(root.as_ref()) {
        Ok(entries) => {
            let names: Vec<String> = entries
                .take(6)
                .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
                .collect();
            format!("read_dir OK: {} entries, first: {}", names.len(), names.join(", "))
        }
        Err(e) => format!("read_dir FAILED: {e}"),
    };
    respond(report.as_bytes())
}
