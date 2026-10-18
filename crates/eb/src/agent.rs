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
    permission_anchor: RefCell<Option<gtk::Widget>>,
    immediate_refresh: RefCell<Option<Rc<dyn Fn()>>>,
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
            permission_anchor: RefCell::new(None),
            immediate_refresh: RefCell::new(None),
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

    /// Anchor permission prompts to an in-window widget. Without this the
    /// fallback modal dialog is used; the popover avoids a separate OS window
    /// that can map behind the main window on some window managers.
    pub fn set_permission_anchor(&self, widget: gtk::Widget) {
        *self.permission_anchor.borrow_mut() = Some(widget);
    }

    /// Install the direct repaint the agent uses after applying events.
    pub fn set_immediate_refresh(&self, refresh: Rc<dyn Fn()>) {
        *self.immediate_refresh.borrow_mut() = Some(refresh);
    }

    fn refresh_now(&self) {
        if std::env::var_os("EB_DEBUG").is_some() {
            eprintln!("runner: refresh_now {:?}", std::thread::current().id());
        }
        if let Some(refresh) = self.immediate_refresh.borrow().as_ref() {
            refresh();
        } else {
            self.refresher.request_now();
        }
    }

    /// Start an answer for `text`. A previous in-flight answer is marked
    /// cancelled before the new task is created, so a task can never be left
    /// "streaming" after the user moves on.
    pub fn submit(self: &Rc<Self>, text: String) {
        self.submit_inner(text, false);
    }

    /// Start a fresh sidebar task even when a conversation is open. Used by
    /// harnesses that explicitly need the "second task" screenshot.
    pub fn submit_as_new(self: &Rc<Self>, text: String) {
        self.submit_inner(text, true);
    }

    fn submit_inner(self: &Rc<Self>, text: String, force_new_task: bool) {
        if std::env::var_os("EB_DEBUG").is_some() {
            eprintln!("runner: submit start {:?}", std::thread::current().id());
        }
        self.supersede();

        let task = {
            let mut app = self.store.borrow_mut();
            let current = match app.route {
                crate::model::Route::Task(index) if app.tasks.get(index).is_some() => Some(index),
                _ => None,
            };
            match (force_new_task, current) {
                (false, Some(index)) => app.begin_turn(index, &text),
                _ => app.begin_task(&text),
            }
        };
        let Some(task) = task else {
            self.refresher.request();
            return;
        };
        // Paint the user's turn before waiting for any agent handshake.
        self.refresh_now();

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
        self.refresh_now();

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
        if !jobs.is_empty() && std::env::var_os("EB_DEBUG").is_some() {
            eprintln!("permission: {} pending", jobs.len());
        }
        for job in jobs {
            self.ask_permission(job);
        }
    }

    fn ask_permission(&self, job: PermissionJob) {
        let options = job.request.options.clone();
        // Headless harness path: answer without opening a window. The default
        // (unset) is the in-window popover below.
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
        if let Some(anchor) = self.permission_anchor.borrow().clone() {
            self.ask_permission_popover(job, anchor);
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
        if std::env::var_os("EB_DEBUG").is_some() {
            eprintln!(
                "permission: showing {:?} ({} options) parent_visible={}",
                job.request.title,
                options.len(),
                self.window.is_visible()
            );
        }
        for (index, option) in options.iter().enumerate() {
            dialog.add_button(&option.label, gtk::ResponseType::Other(index as u16));
        }
        dialog.add_button("Reject", gtk::ResponseType::Cancel);
        dialog.set_keep_above(true);
        dialog.set_position(gtk::WindowPosition::CenterOnParent);
        dialog.show_all();
        dialog.present();
        if std::env::var_os("EB_DEBUG").is_some() {
            eprintln!(
                "permission: dialog visible={} mapped={} allocation={:?}",
                dialog.is_visible(),
                dialog.is_mapped(),
                dialog.allocation()
            );
        }
        let response = dialog.run();
        dialog.close();

        let decision = match response {
            gtk::ResponseType::Other(index) => options
                .get(index as usize)
                .map(|option| PermissionDecision::Selected(option.id.clone()))
                .unwrap_or(PermissionDecision::Cancelled),
            _ => PermissionDecision::Cancelled,
        };
        if std::env::var_os("EB_DEBUG").is_some() {
            eprintln!("permission: decision {decision:?}");
        }
        let _ = job.reply.send(decision);
    }

    fn ask_permission_popover(&self, job: PermissionJob, anchor: gtk::Widget) {
        let popover = gtk::Popover::new(Some(&anchor));
        popover.set_modal(true);
        popover.set_position(gtk::PositionType::Top);
        popover.set_constrain_to(gtk::PopoverConstraint::Window);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        let title = gtk::Label::new(Some(&job.request.title));
        title.set_xalign(0.0);
        title.set_line_wrap(true);
        title.set_width_chars(1);
        title.set_max_width_chars(60);
        content.pack_start(&title, false, false, 0);

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::End);
        let decision = Rc::new(RefCell::new(None));
        let main_loop = gtk::glib::MainLoop::new(None, false);
        let mut auto_click: Option<gtk::Button> = None;
        let click_mode = std::env::var("EB_PERMISSION_CLICK").ok();
        for option in &job.request.options {
            let button = gtk::Button::with_label(&option.label);
            if auto_click.is_none()
                && click_mode.as_deref() == Some("allow")
                && option.kind.starts_with("allow")
            {
                auto_click = Some(button.clone());
            }
            button.connect_clicked({
                let decision = decision.clone();
                let popover = popover.clone();
                let main_loop = main_loop.clone();
                let id = option.id.clone();
                move |_| {
                    *decision.borrow_mut() = Some(PermissionDecision::Selected(id.clone()));
                    popover.popdown();
                    main_loop.quit();
                }
            });
            buttons.pack_start(&button, false, false, 0);
        }
        // ACP normally includes a reject option already; only add the escape
        // hatch when it does not.
        if !job
            .request
            .options
            .iter()
            .any(|option| option.kind.starts_with("reject"))
        {
            let reject = gtk::Button::with_label("Reject");
            reject.connect_clicked({
                let decision = decision.clone();
                let popover = popover.clone();
                let main_loop = main_loop.clone();
                move |_| {
                    *decision.borrow_mut() = Some(PermissionDecision::Cancelled);
                    popover.popdown();
                    main_loop.quit();
                }
            });
            if auto_click.is_none() && click_mode.as_deref() == Some("reject") {
                auto_click = Some(reject.clone());
            }
            buttons.pack_start(&reject, false, false, 0);
        }
        content.pack_start(&buttons, false, false, 0);

        popover.connect_closed({
            let main_loop = main_loop.clone();
            move |_| main_loop.quit()
        });
        popover.add(&content);
        content.show_all();
        if std::env::var_os("EB_DEBUG").is_some() {
            eprintln!(
                "permission: popover {:?} ({} options)",
                job.request.title,
                job.request.options.len()
            );
        }
        popover.popup();
        if let Some(button) = auto_click {
            gtk::glib::timeout_add_local_once(Duration::from_millis(100), move || {
                button.clicked();
            });
        }
        main_loop.run();
        let decision = decision
            .borrow_mut()
            .take()
            .unwrap_or(PermissionDecision::Cancelled);
        if std::env::var_os("EB_DEBUG").is_some() {
            eprintln!("permission: decision {decision:?}");
        }
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
            // Agent events must paint even when no UI handler follows them.
            // `request_now` avoids relying on an idle callback that can be
            // starved while a modal permission popover owns a nested loop.
            self.refresh_now();
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
