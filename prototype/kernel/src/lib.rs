//! EvoBuddy's core.
//!
//! A Cordis-shaped plugin runtime written in Rust, compiled to `wasm32-wasip1`,
//! and hosted by Node. The core owns the plugin registry, the lifecycle, and the
//! capability that motivated a Rust core in the first place: in-process
//! multi-pattern content search.
//!
//! Layout:
//!
//! - [`protocol`] — the JSON messages in both directions.
//! - [`mem`] — the framed heap boundary.
//! - [`host`] — the core's outbound channel (logging and, later, callbacks).
//! - [`walk`] / [`search`] — the search capability and its traversal.
//! - [`fs`] — the confined filesystem capability over WASI preopens.
//! - [`policy`] — the waterfall that gates scans before the walker starts.
//! - [`plugins`] — the rutis plugins that register and consume them.
//! - [`kernel`] — the root context and request routing.
//! - [`abi`] — the C exports, present only on `wasm32`.
//!
//! Build for the wasm target:
//!
//! ```sh
//! cargo build --release --target wasm32-wasip1
//! ```
//!
//! `wasm32-wasip1` is required, not preferred: the core reads the clock and the
//! filesystem, and WASI preview1 is what provides both without a host call per
//! operation.
pub mod fs;
pub mod host;
pub mod kernel;
pub mod mem;
pub mod plugins;
pub mod policy;
pub mod protocol;
pub mod search;
pub mod walk;

#[cfg(target_arch = "wasm32")]
pub mod abi;
