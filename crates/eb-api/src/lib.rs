//! EvoBuddy's plugin surface — the only crate a plugin may depend on.
//!
//! The architecture is *data plugins, host rendering*: a plugin runs wherever
//! the host runs it, looks at the data it is handed, and produces plain values —
//! view models, menu items, notice copy. It never touches GTK; the host owns
//! every widget and the rendering policy. That keeps the thread contract to one
//! sentence (plugin code never touches a widget) and keeps the pixel-faithful
//! rendering in one place.
//!
//! Three layers live here:
//!
//! | module     | holds                                                          |
//! |------------|----------------------------------------------------------------|
//! | `model`    | the application data vocabulary and its transitions            |
//! | `contrib`  | what a plugin can contribute: nav rows, menus, notices, chips… |
//! | the traits | [`EbPlugin`] (the mount entry point) and [`AnswerSource`]      |
//!
//! `Ui` is the handle a plugin's `mount` receives: it exposes the rutis context
//! (for providing services) and the contribution registries (for declaring UI
//! content). Plugins depend on this crate and on nothing else; the host must
//! never depend on a plugin crate's internals — the workspace layout enforces
//! both directions.

pub mod contrib;
pub mod model;

pub use rutis::CordisError;
use rutis::Ctx;

use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

/// Everything a plugin needs at mount time.
pub struct Ui<'a> {
    /// The rutis context. Provide services through it (`ctx.provide_as`); what
    /// a plugin provides is visible to the host and to other plugins.
    pub ctx: &'a Ctx,
    /// The contribution registries. Declare UI content through it. The mutex
    /// exists to satisfy the mount futures' `Send` bound; all mounts run on one
    /// thread, so it is never contended.
    pub contribs: &'a std::sync::Mutex<contrib::Contribs>,
}

/// A plugin's mount entry point.
///
/// `mount` declares contributions and provides services; it should not do long
/// work. A failing `mount` fails the plugin's rutis fiber, which rolls the load
/// back rather than leaving a half-registered plugin behind.
pub trait EbPlugin: Send + Sync + 'static {
    fn id(&self) -> &'static str;
    fn mount(&self, ui: &Ui) -> Result<(), CordisError>;
}

/// One event in an agent's answer. Pure data: it can cross a thread boundary,
/// and the host — never the plugin — turns it into widgets.
#[derive(Clone, Debug)]
pub enum AgentEvent {
    Started {
        name: String,
    },
    Status {
        text: String,
    },
    /// A text increment. The host appends it to the current text block.
    Delta {
        text: String,
    },
    /// A reasoning/streaming thought increment (ACP `agent_thought_chunk`).
    /// The host appends it to the current thought block, kept visually apart
    /// from the answer.
    Thought {
        text: String,
    },
    /// A complete block (tool row, table, …).
    Block {
        block: model::Block,
    },
    Artifact {
        artifact: model::Artifact,
    },
    Usage {
        used: u32,
        total: u32,
    },
    /// A tool call needs the user's decision. The transcript renders the
    /// request inline and answers through the responder.
    PermissionRequested {
        request: PermissionRequest,
        responder: PermissionResponder,
    },
    Finished,
    Failed {
        message: String,
    },
}

/// One option offered by an agent's permission request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionOption {
    pub id: String,
    pub label: String,
    /// ACP `kind` (`allow_once`, `reject_once`, …); UI uses it only for styling.
    pub kind: String,
}

/// A tool permission request that must be answered before the agent continues.
#[derive(Clone, Debug)]
pub struct PermissionRequest {
    pub title: String,
    pub options: Vec<PermissionOption>,
}

/// The user's answer to [`PermissionRequest`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionDecision {
    Selected(String),
    Cancelled,
}

/// The reply half of an inline permission request: the host hands it back to
/// the answering UI, which sends the user's decision to the blocked worker.
#[derive(Clone)]
pub struct PermissionResponder(
    Arc<std::sync::Mutex<Option<std::sync::mpsc::Sender<PermissionDecision>>>>,
);

impl PermissionResponder {
    pub fn new(sender: std::sync::mpsc::Sender<PermissionDecision>) -> Self {
        Self(Arc::new(std::sync::Mutex::new(Some(sender))))
    }

    /// Deliver the decision exactly once; `false` means the waiter is gone.
    pub fn respond(&self, decision: PermissionDecision) -> bool {
        self.0
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
            .map(|sender| sender.send(decision).is_ok())
            .unwrap_or(false)
    }
}

impl std::fmt::Debug for PermissionResponder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("PermissionResponder")
    }
}

/// Host-side permission UI. Called on the agent worker thread; implementations
/// must marshal to the UI thread and block until the user answers.
pub trait PermissionPrompter: Send + Sync {
    fn ask(&self, request: PermissionRequest) -> PermissionDecision;
}

/// A safe default for sources/tests that never ask for permission.
pub struct DenyAll;

impl PermissionPrompter for DenyAll {
    fn ask(&self, _request: PermissionRequest) -> PermissionDecision {
        PermissionDecision::Cancelled
    }
}

/// The sending half of an agent event stream.
///
/// Cheap to clone and safe to move to an agent's worker thread. A send fails
/// once the host has dropped the stream, which is how a cancelled/closed task
/// tells the producer to stop.
#[derive(Clone)]
pub struct AgentSink {
    tx: Arc<Mutex<mpsc::SyncSender<AgentEvent>>>,
}

impl AgentSink {
    /// Create a sink/stream pair. The host keeps the stream; the sink goes to
    /// [`AnswerSource::start`].
    pub fn channel() -> (AgentSink, AgentStream) {
        Self::channel_with_capacity(64)
    }

    /// A bounded stream: once the queue is full, `send` blocks the producer
    /// (not the GTK main thread) instead of growing memory without limit.
    pub fn channel_with_capacity(capacity: usize) -> (AgentSink, AgentStream) {
        let (tx, rx) = mpsc::sync_channel(capacity);
        (
            AgentSink {
                tx: Arc::new(Mutex::new(tx)),
            },
            AgentStream { rx },
        )
    }

    pub fn send(&self, event: AgentEvent) -> Result<(), AgentClosed> {
        let tx = self.tx.lock().map_err(|_| AgentClosed)?;
        tx.send(event).map_err(|_| AgentClosed)
    }
}

/// The receiving half of an agent event stream; the host drains it on the GTK
/// main thread.
pub struct AgentStream {
    rx: mpsc::Receiver<AgentEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentStreamError {
    Empty,
    Timeout,
    Closed,
}

impl AgentStream {
    /// Non-blocking poll. `Empty` means "call again later"; `Closed` means the
    /// producer dropped every sink.
    pub fn try_recv(&self) -> Result<AgentEvent, AgentStreamError> {
        match self.rx.try_recv() {
            Ok(event) => Ok(event),
            Err(mpsc::TryRecvError::Empty) => Err(AgentStreamError::Empty),
            Err(mpsc::TryRecvError::Disconnected) => Err(AgentStreamError::Closed),
        }
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<AgentEvent, AgentStreamError> {
        match self.rx.recv_timeout(timeout) {
            Ok(event) => Ok(event),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(AgentStreamError::Timeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(AgentStreamError::Closed),
        }
    }
}

/// The host has dropped the receiving end; the agent should stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentClosed;

impl std::fmt::Display for AgentClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the host closed the agent stream")
    }
}

impl std::error::Error for AgentClosed {}

/// A cancel handle returned by [`AnswerSource::start`].
pub trait AnswerHandle: Send + Sync {
    fn cancel(&self);
}

/// A handle for a source that produces its turn synchronously and has nothing
/// to cancel.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopHandle;

impl AnswerHandle for NoopHandle {
    fn cancel(&self) {}
}

/// Who answers a submitted prompt.
///
/// The built-in implementation is the demo agent (fixed placeholder turns that
/// exercise every message block), plus a fake streaming agent used by the
/// harness. A real agent replaces them by providing this service.
///
/// `start` must return promptly: it launches the answer and reports progress
/// through `AgentEvent`s, so the GTK main thread is never blocked by a model,
/// network call or tool run. `cancel` is idempotent and should make the
/// producer stop as soon as possible.
pub trait AnswerSource: Send + Sync + 'static {
    fn start(&self, prompt: &str, sink: AgentSink) -> Box<dyn AnswerHandle>;

    /// Start with the task's isolated workspace, when the host has one.
    fn start_in(
        &self,
        prompt: &str,
        sink: AgentSink,
        permissions: std::sync::Arc<dyn PermissionPrompter>,
        workspace: Option<std::path::PathBuf>,
    ) -> Box<dyn AnswerHandle> {
        let _ = (permissions, workspace);
        self.start(prompt, sink)
    }

    /// Start with an optional per-conversation route override. Sources that do
    /// not route simply ignore it.
    fn start_in_route(
        &self,
        prompt: &str,
        sink: AgentSink,
        permissions: std::sync::Arc<dyn PermissionPrompter>,
        workspace: Option<std::path::PathBuf>,
        route: Option<&str>,
    ) -> Box<dyn AnswerHandle> {
        let _ = route;
        self.start_in(prompt, sink, permissions, workspace)
    }

    /// Start with a host-provided permission prompt. Sources that never ask for
    /// permission can keep implementing only [`AnswerSource::start`].
    fn start_with_permissions(
        &self,
        prompt: &str,
        sink: AgentSink,
        permissions: std::sync::Arc<dyn PermissionPrompter>,
    ) -> Box<dyn AnswerHandle> {
        let _ = permissions;
        self.start(prompt, sink)
    }
}

/// Provide the answer source as a rutis service. The host reads it with
/// [`answer_source`]; both sides go through this helper so the type key — the
/// one piece of rutis plumbing a plugin should not have to know — stays here.
pub fn provide_answer(
    ctx: &Ctx,
    source: std::sync::Arc<dyn AnswerSource>,
) -> Result<rutis::Disposer, CordisError> {
    ctx.provide_as::<dyn AnswerSource>(rutis::TypeKey::of::<dyn AnswerSource>(), source)
}

/// Read the answer source, wherever a plugin provided it.
pub fn answer_source(ctx: &Ctx) -> Option<std::sync::Arc<dyn AnswerSource>> {
    ctx.get_as::<dyn AnswerSource>(rutis::TypeKey::of::<dyn AnswerSource>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn a_full_queue_blocks_the_producer_until_the_host_drains() {
        let (sink, stream) = AgentSink::channel_with_capacity(1);
        sink.send(AgentEvent::Finished).unwrap();

        let sent = Arc::new(AtomicBool::new(false));
        let producer = {
            let sink = sink.clone();
            let sent = sent.clone();
            thread::spawn(move || {
                sink.send(AgentEvent::Finished).unwrap();
                sent.store(true, Ordering::SeqCst);
            })
        };

        thread::sleep(Duration::from_millis(50));
        assert!(!sent.load(Ordering::SeqCst), "the second send should block");

        assert!(matches!(stream.try_recv(), Ok(AgentEvent::Finished)));
        assert!(matches!(
            stream.recv_timeout(Duration::from_secs(1)),
            Ok(AgentEvent::Finished)
        ));
        producer.join().unwrap();
        assert!(sent.load(Ordering::SeqCst));
    }
}
