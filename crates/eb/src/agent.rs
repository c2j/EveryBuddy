//! The in-flight agent turn.
//!
//! An [`AnswerSource`] runs off the GTK main thread and reports progress through
//! an [`AgentSink`]. This module owns the receiving side: it drains events on
//! the main loop, applies them to [`crate::model::App`], and asks for a repaint.
//! Plugin code never sees a widget.
//!
//! The demo agent emits its whole turn synchronously inside `start`, so the
//! first drain usually completes before this function returns. The fake agent
//! (`EB_FAKE_AGENT=1`) sleeps between chunks, which is what exercises the timer
//! and cancellation paths.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Arc;
use std::time::Duration;

use gtk::prelude::*;

use eb_api::{
    AgentEvent, AgentSink, AgentStream, AgentStreamError, AnswerHandle, AnswerSource,
    PermissionDecision, PermissionPrompter, PermissionRequest,
};

use crate::model::Store;
use crate::shell::Refresher;

struct Active {
    task: usize,
    handle: Box<dyn AnswerHandle>,
    stream: AgentStream,
}

/// A single thread that owns the agent runtime. `AnswerSource::start` runs here,
/// not on the GTK main thread; the current-thread tokio runtime also stays
/// alive so an agent that spawns tasks can make progress between starts.
pub struct AgentRuntime {
    tx: Option<tokio::sync::mpsc::UnboundedSender<Job>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

type Job = Box<dyn FnOnce() + Send + 'static>;

impl AgentRuntime {
    pub fn new() -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Job>();
        let thread = std::thread::Builder::new()
            .name("evobuddy-agent".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("agent runtime");
                runtime.block_on(async move {
                    while let Some(job) = rx.recv().await {
                        job();
                    }
                });
            })
            .expect("agent runtime thread");
        AgentRuntime {
            tx: Some(tx),
            thread: Some(thread),
        }
    }

    /// Run `source.start` on the runtime thread and wait for its handle. The
    /// contract says `start` returns promptly; the wait keeps the handle/stream
    /// handoff synchronous, which the rest of the host relies on.
    pub fn start(
        &self,
        source: Arc<dyn AnswerSource>,
        prompt: String,
        sink: AgentSink,
        permissions: Arc<dyn PermissionPrompter>,
    ) -> Result<Box<dyn AnswerHandle>, String> {
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        let tx = self
            .tx
            .as_ref()
            .ok_or_else(|| "the agent runtime is gone".to_string())?;
        tx.send(Box::new(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                source.start_with_permissions(&prompt, sink, permissions)
            }))
            .map_err(|_| "the agent panicked while starting".to_string());
            let _ = reply_tx.send(result);
        }))
        .map_err(|_| "the agent runtime is gone".to_string())?;
        reply_rx
            .recv()
            .map_err(|_| "the agent runtime dropped the start request".to_string())?
    }
}

impl Drop for AgentRuntime {
    fn drop(&mut self) {
        // Close the queue first, then wait for the loop to observe it and exit.
        self.tx.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Default for AgentRuntime {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Runner {
    store: Store,
    refresher: Refresher,
    source: Arc<dyn AnswerSource>,
    runtime: Arc<AgentRuntime>,
    active: RefCell<Option<Active>>,
    /// One timer serves whichever answer is current; starting another timer per
    /// submit would double-drain the same stream.
    polling: Cell<bool>,
    window: gtk::ApplicationWindow,
    permission_rx: RefCell<Receiver<PermissionJob>>,
    permissions: Arc<PermissionBridge>,
}

/// A permission request crossing from the agent worker to the GTK thread.
struct PermissionJob {
    request: PermissionRequest,
    reply: SyncSender<PermissionDecision>,
}

/// The worker-side half of the permission bridge. `ask` blocks the agent thread;
/// the Runner polls the other end on the GTK main loop.
struct PermissionBridge {
    tx: mpsc::Sender<PermissionJob>,
}

impl PermissionPrompter for PermissionBridge {
    fn ask(&self, request: PermissionRequest) -> PermissionDecision {
        let (reply, answer) = mpsc::sync_channel(1);
        if self.tx.send(PermissionJob { request, reply }).is_err() {
            return PermissionDecision::Cancelled;
        }
        answer.recv().unwrap_or(PermissionDecision::Cancelled)
    }
}

impl Runner {
    pub fn new(
        store: Store,
        refresher: Refresher,
        source: Arc<dyn AnswerSource>,
        runtime: Arc<AgentRuntime>,
        window: gtk::ApplicationWindow,
    ) -> Rc<Self> {
        let (permission_tx, permission_rx) = mpsc::channel();
        let permissions = Arc::new(PermissionBridge { tx: permission_tx });
        let runner = Rc::new(Runner {
            store,
            refresher,
            source,
            runtime,
            active: RefCell::new(None),
            polling: Cell::new(false),
            window,
            permission_rx: RefCell::new(permission_rx),
            permissions,
        });

        // Permission requests can arrive while no agent event is being drained;
        // poll independently of the 16ms stream timer.
        let poll = runner.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(50), move || {
            poll.poll_permissions();
            gtk::glib::ControlFlow::Continue
        });
        runner
    }

    /// Start an answer for `text`. A previous in-flight answer is marked
    /// cancelled before the new task is created, so a task can never be left
    /// "streaming" after the user moves on.
    pub fn submit(self: &Rc<Self>, text: String) {
        self.supersede();

        let task = self.store.borrow_mut().begin_task(&text);
        let Some(task) = task else {
            self.refresher.request();
            return;
        };

        let (sink, stream) = AgentSink::channel();
        let handle =
            match self
                .runtime
                .start(self.source.clone(), text, sink, self.permissions.clone())
            {
                Ok(handle) => handle,
                Err(message) => {
                    if std::env::var_os("EB_DEBUG").is_some() {
                        eprintln!("agent: start failed: {message}");
                    }
                    self.store
                        .borrow_mut()
                        .apply_agent_event(task, AgentEvent::Failed { message });
                    self.refresher.request();
                    return;
                }
            };
        *self.active.borrow_mut() = Some(Active {
            task,
            handle,
            stream,
        });

        // Synchronous sources (the demo agent) have already sent everything.
        self.drain();
        self.schedule();
        self.refresher.request();

        if let Some(ms) = cancel_after_ms() {
            let runner = self.clone();
            gtk::glib::timeout_add_local_once(Duration::from_millis(ms), move || {
                runner.cancel();
            });
        }
    }

    /// Cancel the in-flight answer; the producer reports the terminal event and
    /// the normal drain path marks the turn finished.
    pub fn cancel(&self) {
        if std::env::var_os("EB_DEBUG").is_some() {
            eprintln!("agent: cancel requested");
        }
        if let Some(active) = self.active.borrow().as_ref() {
            active.handle.cancel();
        }
    }

    fn poll_permissions(&self) {
        let jobs: Vec<PermissionJob> = {
            let rx = self.permission_rx.borrow();
            let mut jobs = Vec::new();
            while let Ok(job) = rx.try_recv() {
                jobs.push(job);
            }
            jobs
        };
        for job in jobs {
            self.ask_permission(job);
        }
    }

    fn ask_permission(&self, job: PermissionJob) {
        let options = job.request.options.clone();
        // Headless harness path: answer without opening a window. The default
        // (unset) is the real modal dialog below.
        if let Ok(mode) = std::env::var("EB_PERMISSION_AUTO") {
            let wanted: &[&str] = if mode == "allow" {
                &["allow_once", "allow_always"]
            } else {
                &["reject_once", "reject_always"]
            };
            let selected = options
                .iter()
                .find(|option| wanted.contains(&option.kind.as_str()))
                .map(|option| PermissionDecision::Selected(option.id.clone()))
                .unwrap_or(PermissionDecision::Cancelled);
            let _ = job.reply.send(selected);
            return;
        }
        let dialog = gtk::MessageDialog::new(
            Some(&self.window),
            gtk::DialogFlags::MODAL | gtk::DialogFlags::DESTROY_WITH_PARENT,
            gtk::MessageType::Question,
            gtk::ButtonsType::None,
            &job.request.title,
        );
        dialog.set_title("Permission required");
        dialog.format_secondary_text(Some("Allow this tool call?"));
        for (index, option) in options.iter().enumerate() {
            dialog.add_button(&option.label, gtk::ResponseType::Other(index as u16));
        }
        dialog.add_button("Reject", gtk::ResponseType::Cancel);
        let response = dialog.run();
        dialog.close();

        let decision = match response {
            gtk::ResponseType::Other(index) => options
                .get(index as usize)
                .map(|option| PermissionDecision::Selected(option.id.clone()))
                .unwrap_or(PermissionDecision::Cancelled),
            _ => PermissionDecision::Cancelled,
        };
        let _ = job.reply.send(decision);
    }

    /// Whether an answer is still streaming. The fake-agent harness polls this
    /// and quits when the stream reaches a terminal event.
    pub fn is_busy(&self) -> bool {
        self.active.borrow().is_some()
    }

    fn supersede(&self) {
        let task = self.active.borrow().as_ref().map(|active| active.task);
        if let Some(task) = task {
            self.store.borrow_mut().apply_agent_event(
                task,
                AgentEvent::Failed {
                    message: "Cancelled".to_string(),
                },
            );
        }
        if let Some(active) = self.active.borrow_mut().take() {
            active.handle.cancel();
        }
    }

    fn drain(self: &Rc<Self>) {
        let mut closed = false;
        let mut applied = false;
        {
            let mut slot = self.active.borrow_mut();
            let Some(active) = slot.as_mut() else {
                return;
            };
            loop {
                match active.stream.try_recv() {
                    Ok(event) => {
                        if std::env::var_os("EB_DEBUG").is_some() {
                            eprintln!("agent: task {} event {event:?}", active.task);
                        }
                        let done =
                            matches!(event, AgentEvent::Finished | AgentEvent::Failed { .. });
                        self.store
                            .borrow_mut()
                            .apply_agent_event(active.task, event);
                        applied = true;
                        if done {
                            closed = true;
                            break;
                        }
                    }
                    Err(AgentStreamError::Empty | AgentStreamError::Timeout) => break,
                    Err(AgentStreamError::Closed) => {
                        self.store.borrow_mut().apply_agent_event(
                            active.task,
                            AgentEvent::Failed {
                                message: "agent stream closed".to_string(),
                            },
                        );
                        applied = true;
                        closed = true;
                        break;
                    }
                }
            }
        }
        if closed {
            self.active.borrow_mut().take();
        }
        if applied {
            // Even when a source finishes inside `start` (and submit is about to
            // refresh too), make the event-to-paint path unconditional.
            self.refresher.request();
        }
    }

    fn schedule(self: &Rc<Self>) {
        if self.active.borrow().is_none() || self.polling.replace(true) {
            return;
        }
        let runner = self.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(16), move || {
            runner.drain();
            runner.refresher.request();
            if runner.active.borrow().is_some() {
                gtk::glib::ControlFlow::Continue
            } else {
                runner.polling.set(false);
                gtk::glib::ControlFlow::Break
            }
        });
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        if let Some(active) = self.active.get_mut().take() {
            active.handle.cancel();
        }
    }
}

fn cancel_after_ms() -> Option<u64> {
    std::env::var("EB_CANCEL_AFTER_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
}
