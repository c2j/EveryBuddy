use cordis_core::{Context, Plugin, PreparedPlugin};
use std::alloc::{alloc, dealloc, Layout};

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

fn respond(body: &[u8]) -> *mut u8 {
    let ptr = kernel_alloc(4 + body.len());
    unsafe {
        std::ptr::copy_nonoverlapping((body.len() as u32).to_le_bytes().as_ptr(), ptr, 4);
        std::ptr::copy_nonoverlapping(body.as_ptr(), ptr.add(4), body.len());
    }
    ptr
}

/// A Cordis plugin, written against cordis-core, running inside wasm.
struct Greeter;

impl Plugin for Greeter {
    type Config = String;
    type Input = String;
    type PrepareError = std::convert::Infallible;
    type ApplyError = std::convert::Infallible;

    fn name(&self) -> std::borrow::Cow<'_, str> { std::borrow::Cow::Borrowed("Greeter") }

    fn prepare(&self, config: String) -> Result<String, Self::PrepareError> { Ok(config) }

    async fn apply(&self, _ctx: Context, input: &String) -> Result<(), Self::ApplyError> {
        log(&format!("  [wasm] Greeter::apply ran, input={input}"));
        Ok(())
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn boot() -> *mut u8 {
    let report = futures::executor::block_on(async {
        let ctx = Context::new();
        let plugin = Greeter;
        let input = plugin.prepare("dsh".to_string()).unwrap();
        let handle = ctx.spawn(PreparedPlugin::from_input(plugin, input)).await.unwrap();
        let state = format!("{:?}", handle.state());
        let disposed = handle.dispose().await;
        format!("spawned state={state} disposed_ok={}", disposed.is_ok())
    });
    respond(report.as_bytes())
}

#[unsafe(no_mangle)]
pub extern "C" fn kernel_free(ptr: *mut u8, len: usize) {
    unsafe { dealloc(ptr, Layout::from_size_align(len.max(1), 8).unwrap()) }
}
