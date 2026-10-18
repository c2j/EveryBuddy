//! The core: one rutis root context kept alive for the lifetime of the module,
//! driven by the host's calls.
//!
//! The runtime is created once and reused. Each host request drives it through
//! `block_on`; there is no ambient event loop inside wasm, so work only advances
//! while a request is being served. That is a deliberate constraint, not an
//! oversight: it means a request's outcome never depends on a task that is
//! still running when the reply is written.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rutis::{Ctx, EventKey, FiberView, Typed};
use serde::Serialize;
use tokio::runtime::{Builder, Runtime};

use crate::fs::{FsFailureCode, FsProvider, FsService};
use crate::host;
use crate::plugins::{SearchProvider, SweepReport, SweepReporter};
use crate::policy::{allow_terminal, SearchGate, SearchDecision, SearchRequested};
use crate::protocol::Response;
use crate::search::{SearchEngine, SearchLimits};

/// Core version reported by `kernel.hello`. Bumped when the wire contract
/// changes in a way an existing host would misread.
pub const VERSION: &str = "0.1.0";

/// A named service the host can address, with the route that reaches it.
const METHODS: &[&str] = &[
    "kernel.hello",
    "plugins.state",
    "plugins.reload",
    "search.find",
    "search.reports",
    "fs.resolve",
    "fs.stat",
    "fs.readText",
    "fs.processPath",
];

/// The live state of one plugin, flattened for the wire.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct PluginState {
    pub name: String,
    /// Fiber identity. A reload produces a new id, which is how the host sees
    /// that it is talking to a different plugin instance and not a mutated one.
    pub id: u64,
    /// Which load of the parent context this fiber belongs to.
    pub generation: u64,
    pub state: String,
    pub error: Option<String>,
}

/// Flatten one fiber's state for the wire.
fn state_of(fiber: &FiberView) -> PluginState {
    let snapshot = fiber.state();
    PluginState {
        name: fiber.name().to_string(),
        id: fiber.id.0,
        generation: snapshot.generation,
        state: format!("{:?}", snapshot.state),
        error: snapshot.error.map(|e| e.to_string()),
    }
}

/// The core.
pub struct Kernel {
    runtime: Runtime,
    ctx: Ctx,
    root: PathBuf,
    limits: SearchLimits,
    reports: Arc<Mutex<Vec<SweepReport>>>,
    provider: Option<FiberView>,
    consumer: Option<FiberView>,
    fs: Option<FiberView>,
    gate: Option<FiberView>,
}

impl Kernel {
    /// Build the runtime and root context and load the initial plugin pair.
    ///
    /// `root` is the tree the sweep plugin scans; `patterns` seeds the first
    /// search provider.
    pub fn boot(root: PathBuf, patterns: Vec<String>, limits: SearchLimits) -> Result<Self, String> {
        let runtime = Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|e| format!("kernel runtime: {e}"))?;
        // The handle is the runtime's; the runtime outlives every call.
        let ctx = Ctx::root_with(runtime.handle().clone());

        let mut kernel = Self {
            runtime,
            ctx,
            root,
            limits,
            reports: Arc::new(Mutex::new(Vec::new())),
            provider: None,
            consumer: None,
            fs: None,
            gate: None,
        };
        kernel.load(patterns)?;
        Ok(kernel)
    }

    /// Load the provider/consumer pair, replacing any previous pair.
    ///
    /// The consumer is spawned **before** the providers so its dependency gate is
    /// visible: it stays pending until the engine exists, then starts. The
    /// filesystem provider and the search gate carry no dependencies.
    fn load(&mut self, patterns: Vec<String>) -> Result<(), String> {
        let consumer = self.ctx.plugin(Typed::new(SweepReporter {
            root: self.root.clone(),
            reports: self.reports.clone(),
            preview_len: 3,
        }));
        let provider = self.ctx.plugin(SearchProvider {
            patterns,
            limits: self.limits.clone(),
        });
        let fs = self.ctx.plugin(FsProvider {
            root: self.root.clone(),
        });
        let gate = self.ctx.plugin(SearchGate);

        self.runtime.block_on(async {
            (&provider)
                .await
                .map_err(|e| format!("search provider did not start: {e}"))?;
            (&fs)
                .await
                .map_err(|e| format!("filesystem provider did not start: {e}"))?;
            (&gate)
                .await
                .map_err(|e| format!("search gate did not start: {e}"))?;
            (&consumer)
                .await
                .map_err(|e| format!("sweep reporter did not start: {e}"))?;
            Ok::<(), String>(())
        })?;

        self.provider = Some(provider);
        self.consumer = Some(consumer);
        self.fs = Some(fs);
        self.gate = Some(gate);
        Ok(())
    }

    /// Replace the provider with one compiled for `patterns`.
    ///
    /// Disposing the old provider removes the service, which evicts the consumer;
    /// the new provider re-provides it and the consumer starts again. The caller
    /// gets the new states so the reload is observable rather than asserted.
    pub fn reload(&mut self, patterns: Vec<String>) -> Result<Vec<PluginState>, String> {
        // Dependents first, then the capabilities they read. The filesystem
        // provider is stateless, so reloading it is free and keeps `load` the one
        // place that spawns the set.
        for previous in [
            self.consumer.take(),
            self.provider.take(),
            self.fs.take(),
            self.gate.take(),
        ]
        .into_iter()
        .flatten()
        {
            self.runtime
                .block_on(async { previous.dispose().await })
                .map_err(|e| format!("disposing previous plugins: {e}"))?;
        }
        self.load(patterns)?;
        Ok(self.states())
    }

    /// Current state of the loaded plugins.
    pub fn states(&self) -> Vec<PluginState> {
        let mut states = Vec::new();
        if let Some(consumer) = &self.consumer {
            states.push(state_of(consumer));
        }
        if let Some(provider) = &self.provider {
            states.push(state_of(provider));
        }
        if let Some(fs) = &self.fs {
            states.push(state_of(fs));
        }
        if let Some(gate) = &self.gate {
            states.push(state_of(gate));
        }
        states
    }

    /// The search engine currently registered, read back through rutis's
    /// registry.
    ///
    /// This is a **non-strict** read (`get`), not `require`. `require` enforces
    /// that the calling fiber declared the dependency, and the request handler
    /// is the embedder, not a plugin. A non-strict read is also what makes the
    /// provider-replaced case visible: it returns `None` instead of a stale
    /// reference, so no caller can hold a capability across a reload.
    fn engine(&self) -> Result<Arc<SearchEngine>, String> {
        self.ctx
            .get::<SearchEngine>()
            .ok_or_else(|| "no search provider is registered".to_string())
    }

    /// The filesystem service, read the same way as [`Kernel::engine`].
    fn fs(&self) -> Result<Arc<FsService>, String> {
        self.ctx
            .get::<FsService>()
            .ok_or_else(|| "no filesystem provider is registered".to_string())
    }

    /// Reports recorded so far, oldest first.
    pub fn reports(&self) -> Vec<SweepReport> {
        self.reports
            .lock()
            .map(|reports| reports.clone())
            .unwrap_or_default()
    }

    /// Route one request. Every failure is a structured reply, never a trap:
    /// the host must be able to tell "the core said no" from "the core died".
    pub fn handle(&mut self, request: crate::protocol::Request) -> Response {
        let id = request.id;
        match request.method.as_str() {
            "kernel.hello" => Response::ok(
                id,
                serde_json::json!({
                    "version": VERSION,
                    "target": std::env::consts::ARCH,
                    "os": std::env::consts::OS,
                    "root": self.root.display().to_string(),
                    "methods": METHODS,
                    "runtime": "rutis",
                }),
            ),
            "plugins.state" => Response::ok(id, self.states()),
            "plugins.reload" => {
                let patterns = match string_list(&request.params, "patterns") {
                    Ok(patterns) => patterns,
                    Err(message) => return Response::fail(id, "bad_request", message),
                };
                match self.reload(patterns) {
                    Ok(states) => Response::ok(id, states),
                    Err(message) => Response::fail(id, "reload_failed", message),
                }
            }
            "search.find" => {
                // The engine and the filesystem both come from the registry; a
                // missing capability is reported before anything else happens.
                let engine = match self.engine() {
                    Ok(engine) => engine,
                    Err(message) => return Response::fail(id, "service_unavailable", message),
                };
                let fs = match self.fs() {
                    Ok(fs) => fs,
                    Err(message) => return Response::fail(id, "service_unavailable", message),
                };
                let requested = match optional_string(&request.params, "root") {
                    Ok(Some(root)) => root,
                    Ok(None) => self.root.display().to_string(),
                    Err(message) => return Response::fail(id, "bad_request", message),
                };
                // The search root lives in the same path world as fs: a relative
                // path resolves against the mount root, and an escape is refused
                // right here instead of surfacing as an empty scan.
                let root = match fs.resolve(&requested, None) {
                    Ok(target) => PathBuf::from(target.target_key),
                    Err(failure) => return fail_fs(id, failure),
                };
                // The policy chain sees every scan before the walker starts;
                // a veto names its reason and nothing is read.
                let event = SearchRequested {
                    root: root.display().to_string(),
                };
                let decision = self.runtime.block_on(async {
                    self.ctx
                        .events()
                        .waterfall(&self.ctx, &EventKey::of(), &event, allow_terminal)
                        .await
                });
                match decision {
                    Ok(SearchDecision::Allow) => match engine.find(&root) {
                        Ok(outcome) => Response::ok(id, outcome),
                        Err(message) => Response::fail(id, "search_failed", message),
                    },
                    Ok(SearchDecision::Deny(reason)) => {
                        Response::fail(id, "search_denied", reason)
                    }
                    Err(error) => Response::fail(id, "search_gate_failed", error.to_string()),
                }
            },
            "search.reports" => Response::ok(id, self.reports()),
            "fs.resolve" => match self.fs() {
                Err(message) => Response::fail(id, "service_unavailable", message),
                Ok(fs) => {
                    let path = match required_string(&request.params, "path") {
                        Ok(path) => path,
                        Err(message) => return Response::fail(id, "bad_request", message),
                    };
                    let cwd = optional_string(&request.params, "cwd").ok().flatten();
                    match fs.resolve(&path, cwd.as_deref()) {
                        Ok(target) => Response::ok(
                            id,
                            serde_json::json!({
                                "targetKey": target.target_key,
                                "displayPath": target.display_path,
                            }),
                        ),
                        Err(failure) => fail_fs(id, failure),
                    }
                }
            },
            "fs.stat" => match self.fs() {
                Err(message) => Response::fail(id, "service_unavailable", message),
                Ok(fs) => {
                    let target = match required_string(&request.params, "targetKey") {
                        Ok(target) => target,
                        Err(message) => return Response::fail(id, "bad_request", message),
                    };
                    match fs.stat(&target) {
                        Ok(None) => Response::ok(id, serde_json::Value::Null),
                        Ok(Some(info)) => Response::ok(
                            id,
                            serde_json::json!({
                                "version": info.version,
                                "type": info.kind,
                                "size": info.size,
                            }),
                        ),
                        Err(failure) => fail_fs(id, failure),
                    }
                }
            },
            "fs.readText" => match self.fs() {
                Err(message) => Response::fail(id, "service_unavailable", message),
                Ok(fs) => {
                    let target = match required_string(&request.params, "targetKey") {
                        Ok(target) => target,
                        Err(message) => return Response::fail(id, "bad_request", message),
                    };
                    match fs.read_text(&target) {
                        Ok(content) => Response::ok(id, serde_json::json!({ "content": content })),
                        Err(failure) => fail_fs(id, failure),
                    }
                }
            },
            "fs.processPath" => match self.fs() {
                Err(message) => Response::fail(id, "service_unavailable", message),
                Ok(fs) => {
                    let target = match required_string(&request.params, "targetKey") {
                        Ok(target) => target,
                        Err(message) => return Response::fail(id, "bad_request", message),
                    };
                    match fs.process_path(&target) {
                        Ok(path) => Response::ok(id, serde_json::json!({ "path": path })),
                        Err(failure) => fail_fs(id, failure),
                    }
                }
            },
            other => Response::fail(id, "unknown_method", format!("no such method: {other}")),
        }
    }

    /// Shut the pair down. Used by tests and by the host's orderly unload path.
    pub fn shutdown(&mut self) {
        self.provider = None;
        self.consumer = None;
        host::log("info", "kernel shut down");
    }
}

/// Read one required string from request params.
fn required_string(params: &serde_json::Value, key: &str) -> Result<String, String> {
    params
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing parameter: {key}"))
}

/// Turn a filesystem failure into a reply whose code the host can map onto
/// `FsError` without parsing the message.
fn fail_fs(id: u64, failure: crate::fs::FsFailure) -> Response {
    Response::fail(
        id,
        match failure.code {
            FsFailureCode::NotFound => "fs_not_found",
            FsFailureCode::NotRegularFile => "fs_not_regular_file",
            FsFailureCode::OutsideRoot => "fs_outside_root",
            FsFailureCode::TooLarge => "fs_too_large",
            FsFailureCode::Io => "fs_io_error",
        },
        format!("{}: {}", failure.code.fs_code(), failure.message),
    )
}

/// Read a required array of strings from request params.
fn string_list(params: &serde_json::Value, key: &str) -> Result<Vec<String>, String> {    let value = params
        .get(key)
        .ok_or_else(|| format!("missing parameter: {key}"))?;
    let array = value
        .as_array()
        .ok_or_else(|| format!("parameter {key} must be an array"))?;
    if array.is_empty() {
        return Err(format!("parameter {key} must not be empty"));
    }
    array
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("parameter {key} must contain only strings"))
        })
        .collect()
}

/// Read an optional string from request params.
fn optional_string(params: &serde_json::Value, key: &str) -> Result<Option<String>, String> {
    match params.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(|s| Some(s.to_string()))
            .ok_or_else(|| format!("parameter {key} must be a string")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Request;

    fn request(id: u64, method: &str, params: serde_json::Value) -> Request {
        Request {
            id,
            method: method.to_string(),
            params,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("evobuddy-kernel-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn kernel(name: &str) -> (Kernel, PathBuf) {
        let root = scratch(name);
        std::fs::write(root.join("a.txt"), "alpha\nneedle here\n").unwrap();
        std::fs::write(root.join("b.txt"), "another needle\n").unwrap();
        let kernel = Kernel::boot(root.clone(), vec!["needle".to_string()], SearchLimits::default())
            .expect("kernel boots");
        (kernel, root)
    }

    #[test]
    fn boot_serves_a_search_through_the_registry() {
        let (mut kernel, root) = kernel("boot");
        let response = kernel.handle(request(1, "search.find", serde_json::json!({})));
        assert!(response.ok);
        let result = response.result.unwrap();
        assert_eq!(result["totalMatches"], 2);
        assert_eq!(result["filesScanned"], 2);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn consumer_sweeps_once_per_provider_generation() {
        let (mut kernel, root) = kernel("sweep");
        assert_eq!(kernel.reports().len(), 1, "one sweep for the initial provider");

        let response = kernel.handle(request(2, "plugins.reload", serde_json::json!({
            "patterns": ["alpha"]
        })));

        assert!(response.ok);
        let reports = kernel.reports();
        assert_eq!(reports.len(), 2, "the consumer restarted and swept again");
        assert_eq!(reports[1].patterns, vec!["alpha".to_string()]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reload_replaces_both_fibers_and_the_consumer_sweeps_again() {
        let (mut kernel, root) = kernel("reload");
        let before = kernel.states();
        let after = kernel
            .handle(request(3, "plugins.reload", serde_json::json!({ "patterns": ["alpha"] })))
            .result
            .unwrap();
        let after: Vec<PluginState> = serde_json::from_value(after).unwrap();

        for name in ["sweep-reporter", "search-provider"] {
            let previous = before.iter().find(|s| s.name == name).unwrap();
            let current = after.iter().find(|s| s.name == name).unwrap();
            assert_ne!(previous.id, current.id, "{name} must be a new fiber");
            assert_eq!(current.state, "Active");
            assert!(current.error.is_none());
        }

        let reports = kernel.reports();
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[1].patterns, vec!["alpha".to_string()]);
        assert_eq!(reports[1].total_matches, 1);
        // The replacement is a new engine, not a mutated one: the old pattern
        // set must not survive into the new generation.
        assert_eq!(reports[1].preview[0].pattern, "alpha");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unknown_method_is_a_structured_failure_not_a_panic() {
        let (mut kernel, root) = kernel("unknown");
        let response = kernel.handle(request(4, "does.not.exist", serde_json::json!({})));

        assert!(!response.ok);
        assert_eq!(response.error.unwrap().code, "unknown_method");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_parameters_are_rejected_before_any_work() {
        let (mut kernel, root) = kernel("badparams");
        for (method, params) in [
            ("plugins.reload", serde_json::json!({})),
            ("plugins.reload", serde_json::json!({ "patterns": [] })),
            ("plugins.reload", serde_json::json!({ "patterns": "needle" })),
            ("plugins.reload", serde_json::json!({ "patterns": [1] })),
        ] {
            let response = kernel.handle(request(5, method, params));
            assert!(!response.ok, "expected rejection for {method}");
            assert_eq!(response.error.unwrap().code, "bad_request");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn hello_describes_the_core() {
        let (mut kernel, root) = kernel("hello");
        let result = kernel
            .handle(request(6, "kernel.hello", serde_json::json!({})))
            .result
            .unwrap();
        assert_eq!(result["runtime"], "rutis");
        assert_eq!(result["version"], VERSION);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn search_outside_the_mount_is_refused_before_anything_is_read() {
        let (mut kernel, root) = kernel("gated");
        let outside = root.parent().unwrap().join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();

        let response = kernel.handle(
            request(13, "search.find", serde_json::json!({ "root": outside.display().to_string() })),
        );

        assert!(!response.ok);
        // 解析即拒绝:检索根与 fs 走同一个路径世界,逃逸到不了 walker。
        assert_eq!(response.error.unwrap().code, "fs_outside_root");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn fs_handlers_resolve_stat_and_read_a_confined_file() {        let (mut kernel, root) = kernel("fshandlers");

        let resolved = kernel.handle(request(7, "fs.resolve", serde_json::json!({ "path": "a.txt" })));
        assert!(resolved.ok, "resolve failed: {:?}", resolved.error);
        let target_key = resolved.result.unwrap()["targetKey"].as_str().unwrap().to_string();

        let stat = kernel.handle(request(8, "fs.stat", serde_json::json!({ "targetKey": target_key })));
        assert_eq!(stat.result.unwrap()["type"], "file");

        let read = kernel.handle(request(9, "fs.readText", serde_json::json!({ "targetKey": target_key })));
        assert_eq!(read.result.unwrap()["content"], "alpha\nneedle here\n");

        let process = kernel
            .handle(request(10, "fs.processPath", serde_json::json!({ "targetKey": target_key })))
            .result
            .unwrap();
        assert_eq!(process["path"], target_key);

        // Absence and escape attempts are structured failures with mappable codes.
        let absent = kernel.handle(request(11, "fs.resolve", serde_json::json!({ "path": "gone.txt" })));
        assert_eq!(absent.error.unwrap().code, "fs_not_found");
        let outside = root.parent().unwrap().join("escape.txt");
        std::fs::write(&outside, "x").unwrap();
        let escaped = kernel.handle(
            request(12, "fs.resolve", serde_json::json!({ "path": outside.display().to_string() })),
        );
        assert_eq!(escaped.error.unwrap().code, "fs_outside_root");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
