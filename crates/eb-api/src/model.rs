//! Application state and the transitions between screens.
//!
//! This build has no agent behind it (`see ui-spec §9`), so a submitted prompt
//! is answered by whichever plugin provides [`super::AnswerSource`] — the
//! built-in demo agent — and the host hands the turn to [`App::submit_turns`].
//! That keeps the UI honest — nothing pretends a model replied on its own —
//! while still rendering the whole message vocabulary.

use std::cell::RefCell;
use std::rc::Rc;

/// Shared, mutated state. GTK signal handlers are `'static`, so anything a
/// handler needs to read or write lives behind this handle rather than in a
/// builder's stack frame.
pub type Store = Rc<RefCell<App>>;

pub fn store(app: App) -> Store {
    Rc::new(RefCell::new(app))
}

/// The Work | Code segmented control.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scene {
    Work,
    Code,
}

/// Which screen the content pane is showing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Route {
    /// The centred landing column (the product's `wb-home-route`).
    Welcome,
    /// A task's conversation, by index into [`App::tasks`].
    Task(usize),
    /// A nav row whose screen has not been built yet. Kept explicit so the UI
    /// can say so rather than silently showing the wrong thing.
    Unbuilt(&'static str),
}

/// A file the agent produced, shown in the artifact panel.
pub struct Artifact {
    pub name: String,
    pub code: String,
}

/// One conversation in the sidebar list.
pub struct Task {
    pub title: String,
    /// Right-hand row meta: a duration once settled, empty while running.
    pub meta: String,
    pub turns: Vec<Turn>,
    pub artifact: Option<Artifact>,
}

pub enum Turn {
    User {
        text: String,
        time: String,
    },
    Agent {
        name: String,
        /// `cr-agent__processing-status`, e.g. "Thinking" or "Finished".
        status: Option<String>,
        blocks: Vec<Block>,
        /// Renders `conversation-streaming-footer` and the stop-shaped send
        /// button instead of the send arrow.
        streaming: bool,
    },
}

/// The block types the message body renders.
pub enum Block {
    Text(String),
    /// A tool call. `detail` is the muted secondary text (duration, target).
    Tool {
        status: String,
        primary: String,
        detail: Option<String>,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

pub struct App {
    /// The selected sidebar nav row, or `None` when a task is open — the
    /// capture shows no row highlighted while a conversation is on screen, and
    /// the task list highlights itself instead.
    pub nav: Option<usize>,
    pub scene: Scene,
    pub route: Route,
    pub tasks: Vec<Task>,
    pub sidebar_visible: bool,
    pub detail_visible: bool,
    /// The artifact panel's "expand" button: the panel takes the whole window
    /// instead of sharing it with the conversation.
    pub detail_maximised: bool,
    /// Menu selections, so the dropdowns hold their state.
    pub model: usize,
    pub workspace: usize,
    pub permission: usize,
    /// Whether each sidebar section is folded: 0 = Tasks, 1 = Workspaces.
    pub collapsed: [bool; 2],
}

impl App {
    pub fn new() -> App {
        App {
            nav: Some(0),
            scene: Scene::Work,
            route: Route::Welcome,
            tasks: Vec::new(),
            sidebar_visible: true,
            detail_visible: false,
            detail_maximised: false,
            model: 0,
            workspace: 0,
            permission: 0,
            collapsed: [false, false],
        }
    }

    /// The sidebar's "New Task" row and the active task are mutually exclusive,
    /// so selecting one clears the other. The route name comes from the nav
    /// plugin's row (it is the plugin's declaration, not the state's).
    pub fn select_nav(&mut self, index: usize, unbuilt_route: &'static str) {
        if index == 0 {
            self.new_task();
        } else {
            self.nav = Some(index);
            self.route = Route::Unbuilt(unbuilt_route);
        }
    }

    /// Return to a blank landing page. Also the effect of the composer's send
    /// button once a task exists.
    pub fn new_task(&mut self) {
        self.nav = Some(0);
        self.route = Route::Welcome;
        self.detail_visible = false;
    }

    pub fn open_task(&mut self, index: usize) {
        if index >= self.tasks.len() {
            return;
        }
        self.nav = None;
        self.route = Route::Task(index);
        self.detail_visible = self.tasks[index].artifact.is_some();
    }

    pub fn set_scene(&mut self, scene: Scene) {
        self.scene = scene;
    }

    /// Record a user turn and the answer turn an [`super::AnswerSource`] produced.
    pub fn submit_turns(&mut self, text: &str, answer: Turn, artifact: Option<Artifact>) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }

        let index = self.tasks.len();
        self.tasks.push(Task {
            title: summarize(text),
            meta: "now".to_string(),
            turns: vec![
                Turn::User {
                    text: text.to_string(),
                    time: "now".to_string(),
                },
                answer,
            ],
            artifact,
        });

        self.nav = None;
        self.route = Route::Task(index);
        self.detail_visible = self.tasks[index].artifact.is_some();
    }
}

/// A task's sidebar label is its first line, clipped to something that fits.
fn summarize(text: &str) -> String {
    let first = text.lines().find(|line| !line.trim().is_empty()).unwrap_or("").trim();
    let mut title: String = first.chars().take(60).collect();
    if first.chars().count() > 60 {
        title.push('…');
    }
    title
}
