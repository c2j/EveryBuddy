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

use crate::model::{Artifact, Store};
use crate::shell::Refresher;

struct Active {
    task: usize,
    handle: Box<dyn AnswerHandle>,
    stream: AgentStream,
}

/// How long the main thread waits for `AnswerSource::start` to hand back a
/// handle. Generous compared with a well-behaved source (which returns after a
/// spawn), small enough that a contract-breaking source surfaces as an error
/// bubble instead of a frozen window.
const START_ACK_TIMEOUT: Duration = Duration::from_secs(10);

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
    ///
    /// The wait is bounded: a source that ignores the contract and blocks
    /// inside `start` would otherwise freeze the GTK main thread here forever,
    /// which is how the intermittent hangs used to present.
    pub fn start(
        &self,
        source: Arc<dyn AnswerSource>,
        prompt: String,
        sink: AgentSink,
        permissions: Arc<dyn PermissionPrompter>,
        workspace: Option<std::path::PathBuf>,
        route: Option<String>,
    ) -> Result<Box<dyn AnswerHandle>, String> {
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        let tx = self
            .tx
            .as_ref()
            .ok_or_else(|| "the agent runtime is gone".to_string())?;
        tx.send(Box::new(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                source.start_in_route(&prompt, sink, permissions, workspace, route.as_deref())
            }))
            .map_err(|_| "the agent panicked while starting".to_string());
            let _ = reply_tx.send(result);
        }))
        .map_err(|_| "the agent runtime is gone".to_string())?;
        await_start(reply_rx, START_ACK_TIMEOUT)
    }
}

/// Wait for `source.start` to hand back its handle, with a bound. The wait
/// happens on the GTK main thread, so a source that blocks inside `start`
/// would otherwise freeze the window indefinitely — which is how the
/// intermittent hangs used to present.
fn await_start(
    reply: Receiver<Result<Box<dyn AnswerHandle>, String>>,
    budget: Duration,
) -> Result<Box<dyn AnswerHandle>, String> {
    reply.recv_timeout(budget).map_err(|error| match error {
        mpsc::RecvTimeoutError::Disconnected => "the agent runtime dropped the start request".to_string(),
        mpsc::RecvTimeoutError::Timeout => {
            "the agent did not acknowledge the start; it may still be running in the background"
                .to_string()
        }
    })?
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

/// Build a bounded plain-text transcript when a conversation switches routes.
/// ACP cannot export its internal session, so this is the portable context the
/// next agent receives. Tool rows are summarized rather than replayed.
/// Whether this turn's prompt should carry the prior visible conversation.
///
/// The custom route is a stateless HTTP chat call, so it needs the composed
/// context on every turn after the first; stateful sources (ACP keeps a
/// server-side session, dsh bridges one) only need it when the conversation
/// migrates across agents.
fn needs_migrated_context(
    route: &str,
    has_prior_answer: bool,
    last_route: Option<&str>,
    had_turns: bool,
) -> bool {
    if route == "custom" {
        return has_prior_answer;
    }
    (last_route.is_some() || had_turns) && last_route.as_deref() != Some(route)
}

fn migrated_prompt(task: &crate::model::Task, prompt: &str) -> String {
    migrated_prompt_with(task, prompt, None)
}

fn migrated_prompt_with(
    task: &crate::model::Task,
    prompt: &str,
    note: Option<&str>,
) -> String {
    let mut context = String::new();
    let end = task.turns.len().saturating_sub(2);
    for turn in task.turns.iter().take(end) {
        match turn {
            crate::model::Turn::User { text, .. } => {
                context.push_str("User: ");
                context.push_str(text.trim());
                context.push('\n');
            }
            crate::model::Turn::Agent { blocks, .. } => {
                let mut body = String::new();
                for block in blocks {
                    match block {
                        crate::model::Block::Text(text) => {
                            if !body.is_empty() {
                                body.push('\n');
                            }
                            body.push_str(text.trim());
                        }
                        crate::model::Block::Tool { primary, .. } => {
                            body.push_str(&format!("\n[tool: {primary}]"));
                        }
                        crate::model::Block::Exec { command, .. } => {
                            body.push_str(&format!("\n[exec: {command}]"));
                        }
                        crate::model::Block::Diff { path, .. } => {
                            body.push_str(&format!("\n[diff: {path}]"));
                        }
                        crate::model::Block::Group { blocks, .. } => {
                            for nested in blocks {
                                if let crate::model::Block::Tool { primary, .. } = nested {
                                    body.push_str(&format!("\n[tool: {primary}]"));
                                }
                            }
                        }
                        crate::model::Block::Plan { entries } => {
                            body.push_str(&format!("\n[plan: {} steps]", entries.len()));
                        }
                        crate::model::Block::Table { .. } => body.push_str("\n[table]"),
                        // Reasoning is deliberately not replayed into the next
                        // agent's context; only the answer's substance migrates.
                        crate::model::Block::Thought { .. } => {}
                    }
                }
                if !body.trim().is_empty() {
                    context.push_str("Assistant: ");
                    context.push_str(body.trim());
                    context.push('\n');
                }
            }
        }
    }
    const MAX: usize = 24_000;
    if context.len() > MAX {
        let start = context.len() - MAX;
        let start = context
            .char_indices()
            .map(|(index, _)| index)
            .find(|index| *index >= start)
            .unwrap_or(context.len());
        context = format!("[… earlier context omitted …]\n{}", &context[start..]);
    }
    match note {
        Some(note) => format!(
            "The conversation is being continued in a different agent. The following is the prior visible conversation, provided so you can continue without losing context. Do not repeat it verbatim.\n\n{context}\n---\nNew request:\n{prompt}{note}"
        ),
        None => format!(
            "The conversation is being continued in a different agent. The following is the prior visible conversation, provided so you can continue without losing context. Do not repeat it verbatim.\n\n{context}\n---\nNew request:\n{prompt}"
        ),
    }
}

pub struct Runner {
    store: Store,
    refresher: Refresher,
    source: Arc<dyn AnswerSource>,
    runtime: Arc<AgentRuntime>,
    /// Every in-flight turn, one per task that is currently streaming. The
    /// list is ordered oldest-submit first; a new submit never displaces an
    /// earlier task's turn.
    active: RefCell<Vec<Active>>,
    /// One timer drains every active stream; starting another timer per
    /// submit would double-drain the same streams.
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


/// Paths of files a reported block wrote, in report order. `Diff` blocks carry
/// the path directly; tool rows often carry it in the title ("Write /abs/path").
fn collect_written_paths(block: &crate::model::Block) -> Vec<String> {
    use crate::model::Block;
    match block {
        Block::Diff { path, .. } => vec![path.clone()],
        Block::Tool { primary, .. } => {
            let trimmed = primary.trim();
            for prefix in ["Write ", "Edit ", "Create ", "Wrote ", "Created "] {
                if let Some(path) = trimmed.strip_prefix(prefix) {
                    let path = path.trim().trim_matches('"');
                    if path.is_empty() {
                        return Vec::new();
                    }
                    // Candidates in report order; `resolve_file_artifact`
                    // walks them newest-first and keeps only paths that exist
                    // on disk, so a wrong guess is harmless. The whole
                    // remainder catches paths with spaces ("Write /App
                    // Support/x.py"); the first token catches trailing notes
                    // ("Write /x.py created").
                    let mut candidates = Vec::new();
                    if let Some(token) = path.split_whitespace().next() {
                        if token != path {
                            candidates.push(token.to_string());
                        }
                    }
                    candidates.push(path.to_string());
                    return candidates;
                }
            }
            Vec::new()
        }
        Block::Group { blocks, .. } => blocks
            .iter()
            .flat_map(collect_written_paths)
            .collect(),
        _ => Vec::new(),
    }
}


/// Read the newest written file that exists under any of `bases` (absolute
/// paths win as-is). The newest candidate wins: within a batch the last
/// written file is the one the user just produced.
fn resolve_file_artifact(
    paths: &[String],
    bases: &[std::path::PathBuf],
) -> Option<Artifact> {
    const MAX: usize = 512 * 1024;
    for path in paths.iter().rev() {
        for base in bases {
            let resolved = if std::path::Path::new(path).is_absolute() {
                std::path::PathBuf::from(path)
            } else {
                base.join(path)
            };
            // This runs on the GTK main thread while a turn drains, so the read
            // is bounded up front: slurping a multi-gigabyte log first and
            // slicing afterwards would freeze the window (and clipping a String
            // mid-character panics).
            let Ok(file) = std::fs::File::open(&resolved) else { continue };
            let mut buffer = Vec::new();
            let mut preview = std::io::Read::take(file, MAX as u64);
            if std::io::Read::read_to_end(&mut preview, &mut buffer).is_err() {
                continue;
            }
            let content = match String::from_utf8(buffer) {
                Ok(text) => text,
                Err(error) => {
                    let valid = error.utf8_error().valid_up_to();
                    let bytes = error.into_bytes();
                    // A clipped final character is still a text file; a break
                    // deeper in means it is binary, so keep looking.
                    if bytes.len() - valid > 3 {
                        continue;
                    }
                    std::str::from_utf8(&bytes[..valid])
                        .unwrap_or_default()
                        .to_string()
                }
            };
            let name = resolved
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            return Some(Artifact { name, code: content });
        }
    }
    None
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
            active: RefCell::new(Vec::new()),
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

    /// Start an answer for `text`. Turns already streaming in *other* tasks
    /// keep running; they drain into their own task in the background. Only a
    /// send into the very task that is mid-stream replaces that turn: it is
    /// marked cancelled before the new turn begins.
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
        // Supersede only the turn belonging to the task this send continues.
        // A fresh task (welcome page, "New Task") leaves every background turn
        // running.
        {
            let app = self.store.borrow();
            match app.route {
                crate::model::Route::Task(index)
                    if !force_new_task && app.tasks.get(index).is_some() =>
                {
                    drop(app);
                    self.supersede_task(index);
                }
                _ => {}
            }
        }

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
        // Carry the composer's model choice into a brand-new task. Without
        // this, the first turn falls back to the global default while the
        // picker still shows the model the user picked.
        {
            let model_choice = self.store.borrow().model;
            let mut app = self.store.borrow_mut();
            if let Some(task_ref) = app.tasks.get_mut(task) {
                if task_ref.agent_route.is_none() {
                    task_ref.agent_route =
                        crate::router::route_for_menu(model_choice).map(str::to_string);
                }
            }
        }
        // A workspace selected on the welcome page is the destination for the
        // new task. Otherwise the first prompt creates a Temporary one below.
        {
            let mut app = self.store.borrow_mut();
            if app.tasks.get(task).is_some_and(|task| task.workspace_id.is_none()) {
                let selected = app.workspace;
                if selected >= 2 && selected <= app.workspaces.len() + 1 {
                    let id = app.workspaces[selected - 2].id.clone();
                    if let Some(task) = app.tasks.get_mut(task) {
                        task.workspace_id = Some(id);
                    }
                } else if selected == 1 {
                    // Explicitly unassigned: do not auto-create a workspace.
                }
            }
        }
        // Resolve the task's Workspace, or create a Temporary one on its first
        // prompt. The workspace owns the execution root; the task stores only
        // the id.
        let (workspace_id, title) = {
            let app = self.store.borrow();
            let task_ref = app.tasks.get(task);
            (
                task_ref.and_then(|task| task.workspace_id.clone()),
                task_ref.map(|task| task.title.clone()).unwrap_or_default(),
            )
        };
        let existing_root = workspace_id.as_deref().and_then(|id| {
            self.store
                .borrow()
                .workspaces
                .iter()
                .find(|workspace| workspace.id == id)
                .map(|workspace| workspace.root.clone())
        });
        let workspace = match existing_root {
            Some(root) => Some(std::path::PathBuf::from(root)),
            None if self.store.borrow().workspace == 1 => None,
            None => {
                let created = crate::workspace::create(&title);
                if let Some(workspace) = &created {
                    let root = workspace.root.clone();
                    let id = workspace.id.clone();
                    let mut app = self.store.borrow_mut();
                    app.workspaces.push(workspace.clone());
                    if let Some(task_ref) = app.tasks.get_mut(task) {
                        task_ref.workspace_id = Some(id);
                    }
                    drop(app);
                    crate::state::save(&self.store);
                    Some(std::path::PathBuf::from(root))
                } else {
                    None
                }
            }
        };
        // Show the user's message before the bridge round-trips below: a wedged
        // dsh bridge can cost the main thread a bounded second per call, and a
        // window that appears to ignore the keystroke reads as a hang.
        self.refresh_now();
        // Bind a stable dsh session to this task before the first event, unless
        // an external ACP agent owns the conversation.
        let session_id = format!("task-{task}-{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|time| time.as_millis())
            .unwrap_or(0));
        crate::bridge::ensure();
        if let Err(error) = crate::bridge::create_session(&session_id) {
            eprintln!("phase1: create_session failed: {error}");
        } else {
            if let Some(task) = self.store.borrow_mut().tasks.get_mut(task) {
                task.session_id = Some(session_id);
            }
            crate::state::save(&self.store);
        }
        let prompt = self.store.borrow().tasks.get(task).and_then(|task| {
                task.turns.iter().rev().find_map(|turn| match turn {
                    crate::model::Turn::User { text, .. } => Some(text.clone()),
                    _ => None,
                })
            });
        if !crate::dsh_agent::enabled() {
            if let Some(prompt) = prompt {
                let _ = crate::bridge::begin_turn(&prompt);
            }
        }
        // Paint the user's turn before waiting for any agent handshake.
        self.refresh_now();
        let (route_override, last_route, had_turns) = {
            let app = self.store.borrow();
            let task_ref = app.tasks.get(task);
            (
                task_ref.and_then(|task| task.agent_route.clone()),
                task_ref.and_then(|task| task.last_route.clone()),
                task_ref.map(|task| task.turns.len() > 2).unwrap_or(false),
            )
        };
        let route = crate::router::effective_route(route_override.as_deref());
        // The custom source is a stateless HTTP chat call, so every turn
        // after the first must carry the prior visible conversation itself;
        // stateful sources (ACP keeps a session, dsh bridges one) only need
        // the composed context when the conversation migrates across agents.
        let has_prior_answer = {
            let app = self.store.borrow();
            app.tasks
                .get(task)
                .map(|task_ref| {
                    task_ref
                        .turns
                        .iter()
                        .any(|turn| matches!(turn, crate::model::Turn::Agent { .. }))
                })
                .unwrap_or(false)
        };
        let migrate = needs_migrated_context(
            route.as_str(),
            has_prior_answer,
            last_route.as_deref(),
            had_turns,
        );

        let agent_text = crate::catalog::prompt_for(&text);
        let agent_text = if migrate {
            let app = self.store.borrow();
            match app.tasks.get(task) {
                Some(task_ref) => {
                    // The composed context mentions prior tool calls in a
                    // `[tool: ...]` notation. On a toolless channel the model
                    // imitates that notation and fabricates execution logs
                    // (observed in the wild), so the note is explicit.
                    let note = (route == "custom").then(|| {
                        "\nNote: in this channel you have NO tool execution. Never claim to have written files, run commands, or verified anything; answer from knowledge only."
                    });
                    migrated_prompt_with(task_ref, &agent_text, note)
                }
                None => agent_text,
            }
        } else {
            agent_text
        };
        let (sink, stream) = AgentSink::channel();
        let handle =
            match self
                .runtime
                .start(
                    self.source.clone(),
                    agent_text,
                    sink,
                    self.permissions.clone(),
                    workspace.clone(),
                    Some(route.clone()),
                )
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
        if let Some(task_ref) = self.store.borrow_mut().tasks.get_mut(task) {
            task_ref.last_route = Some(route.clone());
        }
        crate::state::save(&self.store);
        self.active.borrow_mut().push(Active {
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

    /// Cancel the turn streaming in the task the user is looking at; the
    /// producer reports the terminal event and the normal drain path marks the
    /// turn finished. Background turns in other tasks are untouched.
    pub fn cancel(&self) {
        if std::env::var_os("EB_DEBUG").is_some() {
            eprintln!("agent: cancel requested");
        }
        let task = match self.store.borrow().route {
            crate::model::Route::Task(index) => index,
            _ => return,
        };
        let slot = self.active.borrow();
        if let Some(active) = slot.iter().rev().find(|active| active.task == task) {
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

    /// Whether any answer is still streaming. The fake-agent harness polls
    /// this and quits when every stream reaches a terminal event.
    pub fn is_busy(&self) -> bool {
        !self.active.borrow().is_empty()
    }

    fn supersede_task(&self, task: usize) {
        let stale: Vec<Active> = {
            let mut slot = self.active.borrow_mut();
            let mut stale = Vec::new();
            let mut index = 0;
            while index < slot.len() {
                if slot[index].task == task {
                    stale.push(slot.remove(index));
                } else {
                    index += 1;
                }
            }
            stale
        };
        for active in stale {
            self.store.borrow_mut().apply_agent_event(
                task,
                AgentEvent::Failed {
                    message: "Cancelled".to_string(),
                },
            );
            active.handle.cancel();
        }
    }

    /// The file a just-reported block wrote, if it exists on disk. `Diff`
    /// blocks carry the path directly; tool rows often carry it in the title
    /// ("Write /abs/path").
    fn written_file_artifact(&self, task: usize, event: &AgentEvent) -> Option<Artifact> {
        let AgentEvent::Block { block } = event else {
            return None;
        };
        let paths = collect_written_paths(block);
        if paths.is_empty() {
            return None;
        }
        let app = self.store.borrow();
        let task = app.tasks.get(task)?;
        let workspace_root = task.workspace_id.as_deref().and_then(|id| {
            app.workspaces
                .iter()
                .find(|workspace| workspace.id == id)
                .map(|workspace| workspace.root.clone())
        });
        let mut bases: Vec<std::path::PathBuf> = Vec::new();
        if let Some(root) = workspace_root {
            bases.push(std::path::PathBuf::from(root));
        }
        bases.push(std::path::PathBuf::from("."));
        resolve_file_artifact(&paths, &bases)
    }

    fn drain(self: &Rc<Self>) {
        let mut applied = false;
        let mut closed_tasks: Vec<usize> = Vec::new();
        {
            let mut slot = self.active.borrow_mut();
            let mut index = 0;
            while index < slot.len() {
                let task = slot[index].task;
                // The dsh bridge projects one session at a time. Only the turn
                // whose task still owns the bridge's active session may write
                // its live deltas there; a background task must not append
                // into the conversation the user is looking at.
                let mirrored = !crate::dsh_agent::enabled() && {
                    let app = self.store.borrow();
                    app.tasks
                        .get(task)
                        .and_then(|task| task.session_id.clone())
                        .is_some_and(|id| {
                            crate::bridge::active_session().as_deref() == Some(id.as_str())
                        })
                };
                let mut closed = false;
                loop {
                    match slot[index].stream.try_recv() {
                        Ok(event) => {
                            if std::env::var_os("EB_DEBUG").is_some() {
                                eprintln!("agent: task {task} event {event:?}");
                            }
                            let done = matches!(
                                event,
                                AgentEvent::Finished | AgentEvent::Failed { .. }
                            );
                            // WorkBuddy surfaces every file the agent writes in
                            // the artifact panel. Do the same: when a block
                            // names a written file, read it and pop the panel.
                            self.store
                                .borrow_mut()
                                .apply_agent_event(task, event.clone());
                            if let Some(artifact) = self.written_file_artifact(task, &event) {
                                self.store.borrow_mut().apply_agent_event(
                                    task,
                                    AgentEvent::Artifact { artifact },
                                );
                            }
                            if !done && mirrored {
                                let text = self
                                    .store
                                    .borrow()
                                    .tasks
                                    .get(task)
                                    .and_then(|task| {
                                        task.turns.iter().rev().find_map(|turn| match turn {
                                            crate::model::Turn::Agent { blocks, .. } => Some(
                                                blocks
                                                    .iter()
                                                    .filter_map(|block| match block {
                                                        crate::model::Block::Text(text) => {
                                                            Some(text.clone())
                                                        }
                                                        _ => None,
                                                    })
                                                    .collect::<Vec<_>>()
                                                    .join("\n"),
                                            ),
                                            _ => None,
                                        })
                                    })
                                    .unwrap_or_default();
                                if !text.is_empty() {
                                    let _ = crate::bridge::append_attempt(&text);
                                }
                            }
                            applied = true;
                            if done {
                                closed = true;
                                break;
                            }
                        }
                        Err(AgentStreamError::Empty | AgentStreamError::Timeout) => break,
                        Err(AgentStreamError::Closed) => {
                            self.store.borrow_mut().apply_agent_event(
                                task,
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
                if closed {
                    slot.remove(index);
                    closed_tasks.push(task);
                } else {
                    index += 1;
                }
            }
        }
        let closed_any = !closed_tasks.is_empty();
        for task in closed_tasks {
            self.commit_finished_turn(task);
        }
        if closed_any {
            // Persist the completed turns so switching tasks after a restart
            // shows the conversations, not just the titles.
            crate::state::save(&self.store);
        }
        if applied {
            // Agent events must paint even when no UI handler follows them.
            // `request_now` avoids relying on an idle callback that can be
            // starved while a modal permission popover owns a nested loop.
            self.refresh_now();
        }
    }

    /// Commit a finished turn to the task's dsh session — but only while that
    /// session is still the bridge's active one. If the user has since opened
    /// another task, the bridge has moved on and this projection is skipped.
    fn commit_finished_turn(&self, task: usize) {
        if crate::dsh_agent::enabled() {
            return;
        }
        let (session_id, prompt, answer, tools) = {
            let app = self.store.borrow();
            let Some(task_ref) = app.tasks.get(task) else {
                return;
            };
            let prompt = task_ref
                .turns
                .iter()
                .rev()
                .find_map(|turn| match turn {
                    crate::model::Turn::User { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let mut answer = String::new();
            let mut tools: Vec<(String, String, String)> = Vec::new();
            if let Some(crate::model::Turn::Agent { blocks, .. }) = task_ref
                .turns
                .iter()
                .rev()
                .find(|turn| matches!(turn, crate::model::Turn::Agent { .. }))
            {
                collect_blocks(blocks, &mut answer, &mut tools);
            }
            (task_ref.session_id.clone(), prompt, answer, tools)
        };
        if let Some(session_id) = session_id {
            if crate::bridge::active_session().as_deref() == Some(session_id.as_str())
                && !answer.is_empty()
            {
                let _ = crate::bridge::finish_turn(&answer, &tools);
            }
        }
    }

    fn schedule(self: &Rc<Self>) {
        if self.active.borrow().is_empty() || self.polling.replace(true) {
            return;
        }
        let runner = self.clone();
        let mut ticks = 0u32;
        gtk::glib::timeout_add_local(Duration::from_millis(16), move || {
            runner.drain();
            ticks = ticks.wrapping_add(1);
            // Deltas repaint from drain(), but the elapsed status needs a
            // clock even when the agent is silent. One repaint per ~0.5s is
            // enough; repainting every 16ms burns a full core while waiting.
            if ticks % 30 == 0 {
                // EB_FAKE_STREAM=1 fabricates a Delta on every elapsed
                // repaint, exercising the streaming render path (in-place
                // text updates) without a live agent.
                if std::env::var_os("EB_FAKE_STREAM").is_some() {
                    // Once per stream, write a real file and report it the way
                    // an agent edit does — exercises the artifact pop-open.
                    static FAKE_FILE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
                    if !FAKE_FILE.get().copied().unwrap_or(false) {
                        let _ = FAKE_FILE.set(true);
                        // Exercise the streamed-thought render path (the same
                        // shape ACP's agent_thought_chunk produces).
                        if let Some(active) = runner.active.borrow().last() {
                            runner.store.borrow_mut().apply_agent_event(
                                active.task,
                                eb_api::AgentEvent::Thought {
                                    text: "Considering the edit… ".into(),
                                },
                            );
                        }
                        let dir = std::env::temp_dir().join("evobuddy-fake-stream");
                        let _ = std::fs::create_dir_all(&dir);
                        let file = dir.join("quine.c");
                        let _ = std::fs::write(
                            &file,
                            "#include <stdio.h>\nint main(){char *s=\"#include <stdio.h>%cint main()%c{%char *s=%c%s%c;printf(s,10,34,s,34);}\";printf(s,10,34,s,34);}\n",
                        );
                        let fake_block = eb_api::AgentEvent::Block {
                            block: crate::model::Block::Diff {
                                path: file.display().to_string(),
                                added: 2,
                                removed: 0,
                                lines: Vec::new(),
                            },
                        };
                        if let Some(active) = runner.active.borrow().last() {
                            let task = active.task;
                            runner.store.borrow_mut().apply_agent_event(task, fake_block.clone());
                            // Same interception the drain applies to real
                            // agents: the written file pops the panel open.
                            if let Some(artifact) =
                                runner.written_file_artifact(task, &fake_block)
                            {
                                runner.store.borrow_mut().apply_agent_event(
                                    task,
                                    eb_api::AgentEvent::Artifact { artifact },
                                );
                            }
                        }
                    }
                    let applied = if let Some(active) = runner.active.borrow().last() {
                        runner.store.borrow_mut().apply_agent_event(
                            active.task,
                            eb_api::AgentEvent::Delta { text: " tick".into() },
                        )
                    } else {
                        false
                    };
                    if std::env::var_os("EB_DEBUG").is_some() {
                        eprintln!("fake-stream: delta applied={applied}");
                    }
                }
                runner.refresher.request();
            }
            if !runner.active.borrow().is_empty() {
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
        for active in std::mem::take(self.active.get_mut()) {
            active.handle.cancel();
        }
    }
}

/// Flatten ACP/agent blocks into the text and tool rows stored in dsh.
fn collect_blocks(
    blocks: &[crate::model::Block],
    answer: &mut String,
    tools: &mut Vec<(String, String, String)>,
) {
    for block in blocks {
        match block {
            crate::model::Block::Text(text) => {
                if !answer.is_empty() && !text.is_empty() {
                    answer.push('\n');
                }
                answer.push_str(text);
            }
            crate::model::Block::Tool { primary, detail, .. } => {
                tools.push((
                    primary.clone(),
                    detail.clone().unwrap_or_default(),
                    String::new(),
                ));
            }
            crate::model::Block::Exec { command, output, exit_code } => {
                tools.push((
                    "exec".into(),
                    serde_json::json!({ "command": command }).to_string(),
                    serde_json::json!({ "output": output, "exitCode": exit_code }).to_string(),
                ));
            }
            crate::model::Block::Diff { path, added, removed, lines } => {
                let lines: Vec<_> = lines
                    .iter()
                    .map(|line| match line {
                        crate::model::DiffLine::Add(text) => serde_json::json!({ "kind": "add", "text": text }),
                        crate::model::DiffLine::Remove(text) => serde_json::json!({ "kind": "remove", "text": text }),
                        crate::model::DiffLine::Context(text) => serde_json::json!({ "kind": "context", "text": text }),
                    })
                    .collect();
                tools.push((
                    "diff".into(),
                    serde_json::json!({ "path": path, "added": added, "removed": removed }).to_string(),
                    serde_json::json!({ "lines": lines }).to_string(),
                ));
            }
            crate::model::Block::Group { blocks, .. } => collect_blocks(blocks, answer, tools),
            _ => {}
        }
    }
}

fn cancel_after_ms() -> Option<u64> {
    std::env::var("EB_CANCEL_AFTER_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_route_always_carries_context_after_the_first_turn() {
        // First message: nothing to carry.
        assert!(!needs_migrated_context("custom", false, None, false));
        // Every later turn: the stateless HTTP call needs the history.
        assert!(needs_migrated_context("custom", true, Some("custom"), true));
        assert!(needs_migrated_context("custom", true, None, true));
    }

    #[test]
    fn migrated_prompt_carries_the_toolless_note_for_custom() {
        let mut task = task_with_turns();
        task.turns.push(crate::model::Turn::User {
            text: "run it".into(),
            time: "now".into(),
        });
        let prompt = migrated_prompt_with(&task, "next", Some("\nNote: no tools"));
        assert!(prompt.ends_with("New request:\nnext\nNote: no tools"));
        assert!(prompt.contains("[tool:"));
        // The plain migration keeps the original shape.
        assert!(!migrated_prompt(&task, "next").ends_with("no tools"));
    }

    #[test]
    fn stateful_routes_only_migrate_across_agents() {
        // Same agent, its own session keeps the context.
        assert!(!needs_migrated_context("acp:opencode", true, Some("acp:opencode"), true));
        // The conversation moved to another agent: carry it.
        assert!(needs_migrated_context("acp:grok", true, Some("acp:opencode"), true));
        assert!(needs_migrated_context("acp:opencode", true, None, true));
    }

    fn task_with_turns() -> crate::model::Task {
        crate::model::Task {
            title: "demo".into(),
            meta: String::new(),
            turns: vec![
                crate::model::Turn::User { text: "first question".into(), time: "now".into() },
                crate::model::Turn::Agent {
                    name: "ACP".into(),
                    status: Some("Finished".into()),
                    blocks: vec![crate::model::Block::Text("first answer".into())],
                    streaming: false,
                },
                crate::model::Turn::User { text: "current question".into(), time: "now".into() },
                crate::model::Turn::Agent {
                    name: "EvoBuddy".into(),
                    status: None,
                    blocks: Vec::new(),
                    streaming: true,
                },
            ],
            artifact: None,
            artifacts: Vec::new(),
            session_id: None,
            workspace_id: None,
            agent_route: Some("acp:grok".into()),
            last_route: Some("acp:opencode".into()),
            usage: None,
            started_at: None,
            updated_at: None,
            pinned: false,
            archived: false,
        }
    }

    #[test]
    fn migrated_prompt_contains_prior_turns_not_current_turn() {
        let prompt = migrated_prompt(&task_with_turns(), "current question");
        assert!(prompt.contains("first question"));
        assert!(prompt.contains("first answer"));
        assert!(!prompt.contains("User: current question"));
        assert!(prompt.ends_with("New request:\ncurrent question"));
    }

    #[test]
    fn migrated_prompt_bounds_very_long_context() {
        let mut task = task_with_turns();
        task.turns[1] = crate::model::Turn::Agent {
            name: "ACP".into(),
            status: None,
            blocks: vec![crate::model::Block::Text("x".repeat(40_000))],
            streaming: false,
        };
        let prompt = migrated_prompt(&task, "next");
        assert!(prompt.len() < 30_000);
        assert!(prompt.contains("earlier context omitted"));
    }

    /// The regression the intermittent hangs came from: a source that blocks
    /// inside `start` must cost the main thread its budget, not the window.
    #[test]
    fn the_start_handshake_wait_is_bounded() {
        let (_tx, rx) = mpsc::sync_channel(1);
        let started = std::time::Instant::now();
        let error = match await_start(rx, Duration::from_millis(30)) {
            Ok(_) => panic!("a silent agent must not hang the caller"),
            Err(error) => error,
        };
        assert!(error.contains("did not acknowledge"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());

        let (tx, rx) = mpsc::sync_channel(1);
        tx.send(Ok(Box::new(eb_api::NoopHandle) as Box<dyn AnswerHandle>))
            .unwrap();
        assert!(await_start(rx, Duration::from_secs(1)).is_ok());

        let (_tx, rx) = mpsc::sync_channel::<Result<Box<dyn AnswerHandle>, String>>(1);
        drop(_tx);
        let error = match await_start(rx, Duration::from_secs(1)) {
            Ok(_) => panic!("a dropped reply is an error"),
            Err(error) => error,
        };
        assert!(error.contains("dropped the start request"), "{error}");
    }

    /// Artifact resolution reads on the main thread, so a huge or binary file
    /// must cost a bounded read instead of an allocation, a panic, or a stall.
    #[test]
    fn artifact_reads_are_bounded_and_character_safe() {
        const MAX: usize = 512 * 1024;
        let dir = std::env::temp_dir().join("evobuddy-bounded-artifact");
        std::fs::create_dir_all(&dir).unwrap();

        // A multi-byte character straddling the read limit used to make
        // `String::truncate` panic on the UI thread.
        let big = dir.join("huge.txt");
        let mut text = "a".repeat(MAX - 1);
        text.push_str("éé");
        std::fs::write(&big, &text).unwrap();
        let artifact = resolve_file_artifact(
            &[big.display().to_string()],
            &[std::path::PathBuf::from(".")],
        )
        .expect("the bounded prefix is still text");
        assert!(artifact.code.len() < MAX);
        assert_eq!(artifact.name, "huge.txt");

        // Invalid UTF-8 deeper in the file means it is not a preview at all.
        let binary = dir.join("blob.bin");
        let mut bytes = vec![b'x'; 64];
        bytes[32] = 0xff;
        bytes[33] = 0xfe;
        std::fs::write(&binary, &bytes).unwrap();
        assert!(resolve_file_artifact(
            &[binary.display().to_string()],
            &[std::path::PathBuf::from(".")],
        )
        .is_none());
    }
}


    use super::*;

    /// A Diff block naming a written file must surface that file's content as
    /// the artifact WorkBuddy-style: the panel pops open with the source.
    #[test]
    fn written_files_surface_as_artifacts() {
        let dir = std::env::temp_dir().join("evobuddy-artifact-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("quine.c");
        std::fs::write(&file, "#include <stdio.h>\nint main(){}\n").unwrap();

        let block = crate::model::Block::Diff {
            path: file.display().to_string(),
            added: 2,
            removed: 0,
            lines: Vec::new(),
        };
        let paths = collect_written_paths(&block);
        assert_eq!(paths, vec![file.display().to_string()]);

        let artifact = resolve_file_artifact(&paths, &[std::path::PathBuf::from(".")])
            .expect("written file resolves");
        assert_eq!(artifact.name, "quine.c");
        assert!(artifact.code.contains("#include <stdio.h>"));
    }

    #[test]
    fn tool_rows_with_paths_surface_too() {
        let block = crate::model::Block::Tool {
            status: "Ran".into(),
            primary: "Write /tmp/evobuddy-tool-path/example.txt".into(),
            detail: None,
        };
        assert_eq!(
            collect_written_paths(&block),
            vec!["/tmp/evobuddy-tool-path/example.txt".to_string()]
        );
        // A path with spaces used to be dropped outright; now the whole
        // remainder is a candidate and existence decides.
        let spaced = std::env::temp_dir()
            .join("evobuddy tool path")
            .join("example.txt")
            .display()
            .to_string();
        let block = crate::model::Block::Tool {
            status: "Ran".into(),
            primary: format!("Write {spaced}"),
            detail: None,
        };
        assert_eq!(
            collect_written_paths(&block),
            vec![
                spaced.split(' ').next().unwrap().to_string(),
                spaced.clone(),
            ]
        );
        let dir = std::env::temp_dir().join("evobuddy tool path");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("example.txt"), "spaced").unwrap();
        let artifact = resolve_file_artifact(&collect_written_paths(&block), &[std::path::PathBuf::from(".")])
            .expect("spaced path resolves");
        assert_eq!(artifact.name, "example.txt");
        assert_eq!(artifact.code, "spaced");
        // A trailing note after the path still resolves via the first token.
        let plain = std::env::temp_dir()
            .join("evobuddy-note")
            .join("x.txt")
            .display()
            .to_string();
        let dir = std::env::temp_dir().join("evobuddy-note");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.txt"), "noted").unwrap();
        let block = crate::model::Block::Tool {
            status: "Ran".into(),
            primary: format!("Write {plain} created"),
            detail: None,
        };
        let artifact = resolve_file_artifact(&collect_written_paths(&block), &[std::path::PathBuf::from(".")])
            .expect("noted path resolves");
        assert_eq!(artifact.code, "noted");
        // relative paths resolve against the given bases
        let block = crate::model::Block::Diff {
            path: "src/main.rs".into(),
            added: 1,
            removed: 0,
            lines: Vec::new(),
        };
        let paths = collect_written_paths(&block);
        // cargo test runs with the package root as cwd
        assert!(resolve_file_artifact(&paths, &[std::path::PathBuf::from(".")]).is_some());
    }

