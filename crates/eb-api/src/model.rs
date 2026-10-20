//! Application state and the transitions between screens.
//!
//! This build has no agent behind it (`see ui-spec §9`), so a submitted prompt
//! is answered by whichever plugin provides [`super::AnswerSource`] — the
//! built-in demo agent — and the host hands the turn to [`App::submit_turns`].
//! That keeps the UI honest — nothing pretends a model replied on its own —
//! while still rendering the whole message vocabulary.

use std::cell::RefCell;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

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
#[derive(Clone, Debug)]
pub struct Artifact {
    pub name: String,
    pub code: String,
}

/// A named, persistent working environment that owns zero or more tasks and
/// provides the root in which their agents run. Paths are an implementation
/// detail of the workspace, never its identity.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub root: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub state: WorkspaceState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceState {
    Temporary,
    Kept,
}

/// One conversation in the sidebar list.
#[derive(Debug)]
pub struct Task {
    pub title: String,
    /// Right-hand row meta: a duration once settled, empty while running.
    pub meta: String,
    pub turns: Vec<Turn>,
    pub artifact: Option<Artifact>,
    /// The dsh Session backing this task, once the bridge has bound it.
    pub session_id: Option<String>,
    /// The Workspace this task belongs to. None means unassigned and shown
    /// under Tasks; the workspace owns the execution root.
    pub workspace_id: Option<String>,
    /// Per-conversation route override (`acp:opencode`, `acp:grok`, `dsh`,
    /// `custom`). None means the global default.
    pub agent_route: Option<String>,
    /// The concrete route used by the last turn, used to detect a migration.
    pub last_route: Option<String>,
    /// The last context-usage reading reported by the agent, if any.
    pub usage: Option<(u32, u32)>,
    /// When the current streaming turn began, for elapsed-time feedback.
    pub started_at: Option<std::time::Instant>,
    /// Last time the task changed, as Unix seconds for the sidebar's relative
    /// date (3d, 2d) across restarts.
    pub updated_at: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

/// One entry in an agent plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEntry {
    pub content: String,
    pub status: String,
}

/// One line of a file-diff block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffLine {
    Add(String),
    Remove(String),
    Context(String),
}

/// The block types the message body renders.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Block {
    Text(String),
    /// A tool call. `detail` is the muted secondary text (duration, target).
    Tool {
        status: String,
        primary: String,
        detail: Option<String>,
    },
    /// A write/edit result: a file path, the counts shown in the tool row, and
    /// the lines the product renders in its light-green diff card.
    Diff {
        path: String,
        added: u32,
        removed: u32,
        lines: Vec<DiffLine>,
    },
    /// A command result: title, command and the (possibly long) output.
    Exec {
        command: String,
        output: String,
        exit_code: Option<i32>,
    },
    /// A collapsible tool group (`cr-collapse`). `collapsed` is the initial
    /// state; the widget toggles locally.
    Group {
        title: String,
        blocks: Vec<Block>,
        collapsed: bool,
    },
    /// An ACP/agent plan. Product capture does not cover a plan card, so this
    /// is intentionally a light, host-rendered list.
    Plan { entries: Vec<PlanEntry> },
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
    pub workspaces: Vec<Workspace>,
    pub sidebar_visible: bool,
    pub detail_visible: bool,
    /// The artifact panel's "expand" button: the panel takes the whole window
    /// instead of sharing it with the conversation.
    pub detail_maximised: bool,
    /// Menu selections, so the dropdowns hold their state.
    pub model: usize,
    /// Workspace picker choice: 0 = auto/create, 1 = unassigned, 2.. =
    /// workspaces[index - 2].
    pub workspace: usize,
    pub permission: usize,
    /// Whether each sidebar section is folded: 0 = Tasks, 1 = Workspaces.
    pub collapsed: [bool; 2],
    /// Catalog tab: 0 = Experts, 1 = Skills, 2 = Connectors.
    pub catalog_tab: usize,
    /// Catalog text filter applied on Enter.
    pub catalog_query: String,
    /// Catalog category: 0 = all, 1 = built-in, 2 = user, 3 = enabled.
    pub catalog_filter: usize,
    /// Experts category index and Experts/Expert Teams section.
    pub expert_category: usize,
    pub expert_teams: bool,
    /// Library section: 0 recent, 1 local artifacts, 2 my materials.
    pub library_section: usize,
    pub library_query: String,
    /// Last connector test result: (connector name, message).
    pub connector_test: Option<(String, String)>,
}

impl App {
    pub fn new() -> App {
        App {
            nav: Some(0),
            scene: Scene::Work,
            route: Route::Welcome,
            tasks: Vec::new(),
            workspaces: Vec::new(),
            sidebar_visible: true,
            detail_visible: false,
            detail_maximised: false,
            model: 0,
            workspace: 0,
            permission: 0,
            collapsed: [false, false],
            catalog_tab: 1,
            catalog_query: String::new(),
            catalog_filter: 0,
            expert_category: 0,
            expert_teams: false,
            library_section: 0,
            library_query: String::new(),
            connector_test: None,
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

    /// Start a task with the user's prompt and an empty agent turn. The host
    /// applies [`crate::AgentEvent`]s as the answer streams in.
    pub fn begin_task(&mut self, text: &str) -> Option<usize> {
        let text = text.trim();
        if text.is_empty() {
            return None;
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
                Turn::Agent {
                    name: "EvoBuddy".to_string(),
                    status: None,
                    blocks: Vec::new(),
                    streaming: true,
                },
            ],
            artifact: None,
            session_id: None,
            workspace_id: None,
            agent_route: None,
            last_route: None,
            usage: None,
            started_at: Some(std::time::Instant::now()),
            updated_at: Some(unix_now()),
        });

        self.nav = None;
        self.route = Route::Task(index);
        self.detail_visible = false;
        Some(index)
    }

    /// Append another turn pair to an existing conversation. Sending from the
    /// chat composer continues the task the user is looking at; only the
    /// landing page's first send (or "New Task") starts a new sidebar task.
    pub fn begin_turn(&mut self, index: usize, text: &str) -> Option<usize> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let task = self.tasks.get_mut(index)?;
        task.meta = "now".to_string();
        task.turns.push(Turn::User {
            text: text.to_string(),
            time: "now".to_string(),
        });
        task.turns.push(Turn::Agent {
            name: "EvoBuddy".to_string(),
            status: None,
            blocks: Vec::new(),
            streaming: true,
        });
        task.started_at = Some(std::time::Instant::now());
        task.updated_at = Some(unix_now());
        self.nav = None;
        self.route = Route::Task(index);
        Some(index)
    }

    /// Apply one event to the agent turn of task `index`. Returns false if the
    /// task or turn is gone (for example after the user opened another task).
    pub fn apply_agent_event(&mut self, index: usize, event: crate::AgentEvent) -> bool {
        use crate::AgentEvent;

        if let AgentEvent::Usage { used, total } = event {
            let Some(task) = self.tasks.get_mut(index) else {
                return false;
            };
            task.usage = Some((used, total));
            return true;
        }

        // The artifact lives on the task, not the turn; handle it before the
        // mutable borrow of the turn below.
        if let AgentEvent::Artifact { artifact } = event {
            let Some(task) = self.tasks.get_mut(index) else {
                return false;
            };
            task.artifact = Some(artifact);
            self.detail_visible = true;
            return true;
        }

        let Some(task) = self.tasks.get_mut(index) else {
            return false;
        };
        let Some((name, status, blocks, streaming)) =
            task.turns.iter_mut().rev().find_map(|turn| match turn {
                Turn::Agent {
                    name,
                    status,
                    blocks,
                    streaming,
                } => Some((name, status, blocks, streaming)),
                Turn::User { .. } => None,
            })
        else {
            return false;
        };

        let terminal = matches!(event, AgentEvent::Finished | AgentEvent::Failed { .. });
        match event {
            AgentEvent::Started { name: agent_name } => *name = agent_name,
            AgentEvent::Status { text } => *status = Some(text),
            AgentEvent::Delta { text } => {
                if let Some(Block::Text(existing)) = blocks.last_mut() {
                    existing.push_str(&text);
                } else {
                    blocks.push(Block::Text(text));
                }
            }
            AgentEvent::Block { block } => blocks.push(block),
            AgentEvent::Usage { .. } => unreachable!("handled above"),
            AgentEvent::Finished => {
                *streaming = false;
                *status = None;
            }
            AgentEvent::Failed { message } => {
                *streaming = false;
                *status = Some(format!("Failed: {message}"));
            }
            AgentEvent::Artifact { .. } => unreachable!("handled above"),
        }
        if terminal {
            task.started_at = None;
        }
        true
    }

    /// Record a completed user + agent turn. Kept for callers/tests that do not
    /// stream; the live path uses [`Self::begin_task`] plus
    /// [`Self::apply_agent_event`].
    pub fn submit_turns(&mut self, text: &str, answer: Turn, artifact: Option<Artifact>) {
        let Some(index) = self.begin_task(text) else {
            return;
        };
        let has_artifact = artifact.is_some();
        if let Some(task) = self.tasks.get_mut(index) {
            task.artifact = artifact;
            if task.turns.len() == 2 {
                task.turns[1] = answer;
            }
        }
        self.detail_visible = has_artifact;
    }
}

/// A task's sidebar label is its first line, clipped to something that fits.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs())
        .unwrap_or(0)
}

fn summarize(text: &str) -> String {
    let first = text.lines().find(|line| !line.trim().is_empty()).unwrap_or("").trim();
    let mut title: String = first.chars().take(60).collect();
    if first.chars().count() > 60 {
        title.push('…');
    }
    title
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentEvent;

    #[test]
    fn a_chat_send_appends_to_the_current_task() {
        let mut app = App::new();
        let index = app.begin_task("first").unwrap();
        assert!(matches!(app.route, Route::Task(current) if current == index));
        assert_eq!(app.tasks.len(), 1);

        let again = app.begin_turn(index, "second").unwrap();
        assert_eq!(again, index);
        assert_eq!(app.tasks.len(), 1);
        assert_eq!(app.tasks[index].turns.len(), 4);
        assert!(matches!(
            app.tasks[index].turns[2],
            Turn::User { ref text, .. } if text == "second"
        ));
        assert!(matches!(
            app.tasks[index].turns[3],
            Turn::Agent {
                streaming: true,
                ..
            }
        ));
    }

    #[test]
    fn streamed_events_form_one_agent_turn() {
        let mut app = App::new();
        let index = app.begin_task("hello").unwrap();
        assert!(matches!(
            app.tasks[index].turns[1],
            Turn::Agent {
                streaming: true,
                ..
            }
        ));

        app.apply_agent_event(index, AgentEvent::Started { name: "Test".into() });
        app.apply_agent_event(index, AgentEvent::Delta { text: "one ".into() });
        app.apply_agent_event(index, AgentEvent::Delta { text: "two".into() });
        app.apply_agent_event(
            index,
            AgentEvent::Block {
                block: Block::Tool {
                    status: "Ran".into(),
                    primary: "echo".into(),
                    detail: None,
                },
            },
        );
        app.apply_agent_event(
            index,
            AgentEvent::Artifact {
                artifact: Artifact {
                    name: "a.txt".into(),
                    code: "x".into(),
                },
            },
        );
        app.apply_agent_event(index, AgentEvent::Finished);

        let task = &app.tasks[index];
        assert!(matches!(
            &task.turns[1],
            Turn::Agent { name, blocks, streaming: false, .. }
                if name == "Test"
                    && blocks.len() == 2
                    && matches!(&blocks[0], Block::Text(text) if text == "one two")
                    && matches!(&blocks[1], Block::Tool { .. })
        ));
        assert!(task.artifact.is_some());
        assert!(app.detail_visible);
    }

    #[test]
    fn usage_event_updates_the_task_meter() {
        let mut app = App::new();
        let index = app.begin_task("hello").unwrap();
        assert!(app.apply_agent_event(
            index,
            AgentEvent::Usage {
                used: 42,
                total: 100,
            },
        ));
        assert_eq!(app.tasks[index].usage, Some((42, 100)));
    }

    #[test]
    fn failed_event_closes_the_stream() {
        let mut app = App::new();
        let index = app.begin_task("hello").unwrap();
        app.apply_agent_event(
            index,
            AgentEvent::Failed {
                message: "boom".into(),
            },
        );
        assert!(matches!(
            &app.tasks[index].turns[1],
            Turn::Agent { streaming: false, status: Some(status), .. } if status == "Failed: boom"
        ));
    }

    #[test]
    fn empty_prompt_is_ignored() {
        let mut app = App::new();
        assert!(app.begin_task("   ").is_none());
        assert!(app.tasks.is_empty());
    }
}
