//! Application state and the transitions between screens.
//!
//! This build has no agent behind it (`see ui-spec §9`), so `submit` records the
//! user's turn and appends a *placeholder* answer that exercises every message
//! block type. That keeps the UI honest — nothing pretends a model replied —
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
    /// Which placeholder answer comes next.
    answers: usize,
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
            answers: 0,
        }
    }

    /// The sidebar's "New Task" row and the active task are mutually exclusive,
    /// so selecting one clears the other.
    pub fn select_nav(&mut self, index: usize) {
        if index == 0 {
            self.new_task();
        } else {
            self.nav = Some(index);
            self.route = Route::Unbuilt(crate::spec::NAV[index].1);
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

    /// Record a user turn and the placeholder answer that follows it.
    pub fn submit(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }

        let (answer, artifact) = placeholder_answer(self.answers);
        self.answers += 1;

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

/// The stand-in reply. Two variants, so repeated sends look different, and each
/// one deliberately covers text, tool and table blocks.
///
/// `status` is always `None` and `streaming` always `false`: the capture only
/// ever showed a processing status *while* a turn was arriving, which cannot
/// happen without an agent. The renderer still handles both fields so that
/// wiring a real backend later does not require touching it.
fn placeholder_answer(nth: usize) -> (Turn, Option<Artifact>) {
    let agent = |blocks: Vec<Block>| Turn::Agent {
        name: "EvoBuddy".to_string(),
        status: None,
        blocks,
        streaming: false,
    };

    if nth % 2 == 1 {
        return (
            agent(vec![
                Block::Text(
                    "Nothing is running behind this window yet — EvoBuddy is a UI build, so \
                     this reply is fixed text rather than a model's answer. The blocks below \
                     are here so every message type the real client draws has something to \
                     render."
                        .to_string(),
                ),
                Block::Tool {
                    status: "Ran".to_string(),
                    primary: "echo placeholder".to_string(),
                    detail: Some("0.02s".to_string()),
                },
                Block::Text(
                    "A tool row is one muted line: a status, the command, and how long it \
                     took."
                        .to_string(),
                ),
            ]),
            None,
        );
    }

    let artifact = Artifact {
        name: "demo/main.rs".to_string(),
        code: PLACEHOLDER_CODE.to_string(),
    };

    (
        agent(vec![
            Block::Text(
                "Nothing is running behind this window yet — EvoBuddy is a UI build, so this \
                 reply is fixed text rather than a model's answer. What follows exercises the \
                 message vocabulary the real client draws."
                    .to_string(),
            ),
            Block::Tool {
                status: "Wrote".to_string(),
                primary: "demo/main.rs".to_string(),
                detail: Some("+42 −0".to_string()),
            },
            Block::Text(
                "A table renders as a rounded card with a tinted header row and hairline \
                 separators."
                    .to_string(),
            ),
            Block::Table {
                headers: vec!["Block".to_string(), "Renders as".to_string()],
                rows: vec![
                    vec!["Text".to_string(), "Paragraph on a 24px line".to_string()],
                    vec!["Tool".to_string(), "One muted 26px row".to_string()],
                    vec!["Table".to_string(), "A card with a tinted header".to_string()],
                ],
            },
            Block::Text(
                "Because this turn wrote a file, the artifact panel opens beside the \
                 conversation and the panes can be dragged."
                    .to_string(),
            ),
        ]),
        Some(artifact),
    )
}

/// Stand-in artifact body. Deliberately self-describing so nobody mistakes it
/// for a real generated file.
const PLACEHOLDER_CODE: &str = r#"// Placeholder artifact for the EvoBuddy UI build.
//
// The real client shows a generated file here in a Monaco editor. This pane
// only needs to demonstrate the frame: a tab strip, a title row, and a
// monospaced body that scrolls.

fn main() {
    let message = "no agent is attached to this build";
    println!("{message}");
}
"#;
