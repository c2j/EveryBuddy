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
    /// Every file the agent has written for this task, oldest first, one
    /// entry per name (a rewrite moves the file to the end with fresh
    /// content). The panel shows these as tabs; `artifact` stays the newest
    /// one so the open/reopen logic does not change.
    pub artifacts: Vec<Artifact>,
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
    /// Pinned tasks float to the top of their sidebar section.
    pub pinned: bool,
    /// Archived tasks sink to the bottom of their section, greyed out, until
    /// unarchived from the task menu.
    pub archived: bool,
    /// A turn ended in failure and the user has not opened the task since, so
    /// the sidebar row keeps its red badge. The product gates the same badge
    /// on "seen" (its telemetry calls it `sidebar-red-icon-rendered`).
    pub failure_unseen: bool,
    /// A turn settled while the task was in the background, so the row shows
    /// the unread dot until the task is opened. The product keeps this as an
    /// `unread` column on the conversation and renders it as
    /// `showUnreadDot: conv.unread && !working`.
    pub unread: bool,
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
        /// Unix seconds when the turn was started; the transcript reveals it
        /// under the message on hover. `None` on older saved sessions, which
        /// then simply show no timestamp.
        #[serde(default)]
        time: Option<u64>,
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
    /// Agent reasoning streamed apart from the answer (ACP
    /// `agent_thought_chunk`). No product capture covers its look, so the host
    /// renders it as a light, muted line — same discipline as `Plan`.
    Thought { text: String },
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
    /// The user closed the artifact panel; stay closed until the task
    /// changes, even when new files arrive.
    pub artifact_dismissed: bool,
    /// One-shot tab pick for the artifact panel: set when an artifact card in
    /// the conversation is clicked, consumed by the next paint.
    pub artifact_select: Option<usize>,
    /// Sidebar task filter: 0 All Status, 1 Working, 2 Completed, 3 Failed,
    /// 4 Pending, 5 Planning.
    pub filter_status: usize,
    /// Sidebar task filter: 0 All Time, 1 Today, 2 Last 7 days, 3 Last 30
    /// days.
    pub filter_time: usize,
    /// The artifact panel's "expand" button: the panel takes the whole window
    /// instead of sharing it with the conversation.
    pub detail_maximised: bool,
    /// Menu selections, so the dropdowns hold their state.
    pub model: usize,
    /// Workspace picker choice: 0 = auto/create, 1 = unassigned, 2.. =
    /// workspaces[index - 2].
    pub workspace: usize,
    pub permission: usize,
    /// Whether each sidebar section is folded: 0 = Pinned, 1 = Tasks,
    /// 2 = Workspaces. The Pinned slot exists even while that section is not
    /// drawn, so the other two keep their index.
    pub collapsed: [bool; 3],
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
            artifact_dismissed: false,
            artifact_select: None,
            filter_status: 0,
            filter_time: 0,
            detail_maximised: false,
            model: 0,
            workspace: 0,
            permission: 0,
            collapsed: [false, false, false],
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
        self.artifact_dismissed = false;
        self.artifact_select = None;
    }

    pub fn open_task(&mut self, index: usize) {
        if index >= self.tasks.len() {
            return;
        }
        self.nav = None;
        self.route = Route::Task(index);
        self.artifact_dismissed = false;
        self.artifact_select = None;
        self.detail_visible = self.tasks[index].artifact.is_some();
        // Opening the task is what "seen" means: the row's red badge clears.
        if let Some(task) = self.tasks.get_mut(index) {
            task.failure_unseen = false;
            task.unread = false;
        }
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
            meta: "Just now".to_string(),
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
                    time: Some(unix_now()),
                },
            ],
            artifact: None,
            artifacts: Vec::new(),
            session_id: None,
            workspace_id: None,
            agent_route: None,
            last_route: None,
            usage: None,
            started_at: Some(std::time::Instant::now()),
            updated_at: Some(unix_now()),
            pinned: false,
            archived: false,
            failure_unseen: false,
            unread: false,
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
            time: Some(unix_now()),
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
            // The tab strip accumulates one entry per name; a rewrite moves
            // the file to the end with fresh content. The list is capped so a
            // long run cannot grow the strip without bound.
            task.artifacts.retain(|kept| kept.name != artifact.name);
            task.artifacts.push(artifact.clone());
            const MAX_TABS: usize = 12;
            let overflow = task.artifacts.len().saturating_sub(MAX_TABS);
            task.artifacts.drain(..overflow);
            task.artifact = Some(artifact);
            // A manual close wins: the panel stays hidden for this task even
            // when further files arrive (WorkBuddy behaves the same). Only the
            // open task pops the panel; a background task's files surface when
            // the user switches to it (`open_task` re-shows from `artifact`).
            if !self.artifact_dismissed
                && matches!(self.route, Route::Task(i) if i == index)
            {
                self.detail_visible = true;
            }
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
                    ..
                } => Some((name, status, blocks, streaming)),
                Turn::User { .. } => None,
            })
        else {
            return false;
        };

        let terminal = matches!(event, AgentEvent::Finished | AgentEvent::Failed { .. });
        let failed = matches!(event, AgentEvent::Failed { .. });
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
            AgentEvent::Thought { text } => {
                if let Some(Block::Thought { text: existing }) = blocks.last_mut() {
                    existing.push_str(&text);
                } else {
                    blocks.push(Block::Thought { text });
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
            // A turn that finishes while the user is looking at another task is
            // the product's `unread`; the open task never marks itself.
            let open = matches!(self.route, Route::Task(i) if i == index);
            task.unread = !open;
        }
        if failed {
            task.failure_unseen = true;
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
            task.artifacts = artifact.clone().into_iter().collect();
            task.artifact = artifact;
            if task.turns.len() == 2 {
                task.turns[1] = answer;
            }
        }
        self.detail_visible = has_artifact;
    }
}

/// A task's sidebar label is its first line, clipped to something that fits.
pub fn unix_now() -> u64 {
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
    fn thought_events_merge_into_one_block() {
        let mut app = App::new();
        let index = app.begin_task("hello").unwrap();
        app.apply_agent_event(index, AgentEvent::Thought { text: "Thinking: ".into() });
        app.apply_agent_event(index, AgentEvent::Thought { text: "greet.".into() });
        app.apply_agent_event(index, AgentEvent::Delta { text: "Hi!".into() });
        let blocks = match &app.tasks[index].turns[1] {
            Turn::Agent { blocks, .. } => blocks,
            _ => panic!("agent turn expected"),
        };
        assert_eq!(blocks.len(), 2);
        assert!(matches!(&blocks[0], Block::Thought { text } if text == "Thinking: greet."));
        assert!(matches!(&blocks[1], Block::Text(text) if text == "Hi!"));
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

    /// A background task (not the open route) writing a file must not pop the
    /// panel over the conversation the user is looking at; the file surfaces
    /// when they switch to that task.
    #[test]
    fn a_running_task_shows_the_sidebar_spinner_only_while_streaming() {
        let mut app = App::new();
        let index = app.begin_task("hello").unwrap();
        assert!(task_is_running(&app.tasks[index]), "begun, not yet answered");
        app.apply_agent_event(index, AgentEvent::Delta { text: "hi".into() });
        assert!(task_is_running(&app.tasks[index]), "mid-stream");
        app.apply_agent_event(index, AgentEvent::Finished);
        assert!(!task_is_running(&app.tasks[index]), "settled clean");

        let second = app.begin_task("again").unwrap();
        app.apply_agent_event(
            second,
            AgentEvent::Failed {
                message: "Cancelled".into(),
            },
        );
        assert!(!task_is_running(&app.tasks[second]), "cancelled is terminal");
    }

    #[test]
    fn a_background_artifact_does_not_pop_the_panel() {
        let mut app = App::new();
        let first = app.begin_task("first").unwrap();
        let second = app.begin_task("second").unwrap();
        assert!(matches!(app.route, Route::Task(i) if i == second));
        app.detail_visible = false;
        app.apply_agent_event(
            first,
            AgentEvent::Artifact {
                artifact: Artifact {
                    name: "quiet.py".into(),
                    code: "x".into(),
                },
            },
        );
        assert!(!app.detail_visible, "the foreground panel stays closed");
        assert!(
            app.tasks[first].artifact.is_some(),
            "the file is still recorded on its own task"
        );
        app.open_task(first);
        assert!(app.detail_visible, "switching to the task shows its artifact");
    }

    /// The tab strip accumulates one entry per written file; a rewrite moves
    /// the file to the end with fresh content instead of adding a duplicate.
    #[test]
    fn artifacts_accumulate_as_tabs() {
        let mut app = App::new();
        let index = app.begin_task("write files").unwrap();
        let write = |app: &mut App, name: &str, code: &str| {
            app.apply_agent_event(
                index,
                AgentEvent::Artifact {
                    artifact: Artifact {
                        name: name.into(),
                        code: code.into(),
                    },
                },
            );
        };
        write(&mut app, "a.py", "1");
        write(&mut app, "b.py", "2");
        assert_eq!(
            app.tasks[index]
                .artifacts
                .iter()
                .map(|f| (f.name.as_str(), f.code.as_str()))
                .collect::<Vec<_>>(),
            vec![("a.py", "1"), ("b.py", "2")]
        );

        write(&mut app, "a.py", "3");
        assert_eq!(
            app.tasks[index]
                .artifacts
                .iter()
                .map(|f| (f.name.as_str(), f.code.as_str()))
                .collect::<Vec<_>>(),
            vec![("b.py", "2"), ("a.py", "3")]
        );
        // `artifact` stays the newest file so the open/reopen gate is unchanged.
        assert_eq!(app.tasks[index].artifact.as_ref().unwrap().name, "a.py");

        for n in 0..20 {
            write(&mut app, &format!("f{n}.py"), "x");
        }
        assert_eq!(app.tasks[index].artifacts.len(), 12);
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

    fn settled_agent(status: Option<&str>, blocks: Vec<Block>) -> Task {
        Task {
            title: "t".into(),
            meta: String::new(),
            turns: vec![Turn::Agent {
                name: "A".into(),
                status: status.map(String::from),
                blocks,
                streaming: false,
                time: None,
            }],
            artifact: None,
            artifacts: Vec::new(),
            session_id: None,
            workspace_id: None,
            agent_route: None,
            last_route: None,
            usage: None,
            started_at: None,
            updated_at: None,
            pinned: false,
            archived: false,
            failure_unseen: false,
            unread: false,
        }
    }

    fn plan_block(entries: &[&str]) -> Block {
        Block::Plan {
            entries: entries
                .iter()
                .map(|status| PlanEntry {
                    content: "step".into(),
                    status: (*status).to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn task_status_covers_the_five_sidebar_states() {
        // Freshly begun: streaming, nothing has arrived yet.
        let mut app = App::new();
        let index = app.begin_task("hello").unwrap();
        assert_eq!(task_status(&app.tasks[index]), TaskStatus::Pending);

        // First event lands → Working.
        app.apply_agent_event(index, AgentEvent::Delta { text: "hi".into() });
        assert_eq!(task_status(&app.tasks[index]), TaskStatus::Working);

        // A trailing unfinished plan → Planning; finishing it clears that.
        app.apply_agent_event(
            index,
            AgentEvent::Block {
                block: plan_block(&["in_progress"]),
            },
        );
        assert_eq!(task_status(&app.tasks[index]), TaskStatus::Planning);
        app.apply_agent_event(
            index,
            AgentEvent::Block {
                block: plan_block(&["completed"]),
            },
        );
        assert_eq!(task_status(&app.tasks[index]), TaskStatus::Working);

        // Failed turns keep their terminal status; settled clean turns are
        // Completed.
        app.apply_agent_event(
            index,
            AgentEvent::Failed {
                message: "boom".into(),
            },
        );
        assert_eq!(task_status(&app.tasks[index]), TaskStatus::Failed);

        let done = settled_agent(None, vec![Block::Text("x".into())]);
        assert_eq!(task_status(&done), TaskStatus::Completed);

        // A stopped turn left mid-plan still reads as Planning.
        let stopped = settled_agent(None, vec![plan_block(&["pending", "in_progress"])]);
        assert_eq!(task_status(&stopped), TaskStatus::Planning);
    }

    /// The row's red badge is a "has the user seen this failure yet" flag, not
    /// a rendering of the status: it goes up when a turn fails and stays down
    /// once the task has been opened, even though the task is still Failed.
    #[test]
    fn a_failure_keeps_its_row_badge_until_the_task_is_open() {
        let mut app = App::new();
        let index = app.begin_task("explode").unwrap();
        app.apply_agent_event(
            index,
            AgentEvent::Failed {
                message: "boom".into(),
            },
        );
        assert!(app.tasks[index].failure_unseen, "the row shows the badge");

        app.open_task(index);
        assert!(
            !app.tasks[index].failure_unseen,
            "opening the task is what counts as seen"
        );
        assert_eq!(
            task_status(&app.tasks[index]),
            TaskStatus::Failed,
            "and the task is still Failed"
        );
    }

    /// The unread dot is the other half of "seen": a turn that settles while
    /// another task is on screen marks its row, and opening that row reads it.
    #[test]
    fn a_background_turn_settles_unread_and_opening_it_clears() {
        let mut app = App::new();
        let in_sight = app.begin_task("in sight").unwrap();
        app.apply_agent_event(in_sight, AgentEvent::Finished);
        assert!(
            !app.tasks[in_sight].unread,
            "the open task never marks itself"
        );

        let behind = app.begin_task("behind").unwrap();
        app.open_task(in_sight);
        app.apply_agent_event(behind, AgentEvent::Finished);
        assert!(app.tasks[behind].unread, "it settled out of sight");

        app.open_task(behind);
        assert!(!app.tasks[behind].unread, "opening it reads the row");
    }

    #[test]
    fn filters_select_each_status() {
        let failed = settled_agent(Some("Failed: boom"), vec![]);
        let completed = settled_agent(None, vec![Block::Text("x".into())]);
        let planning = settled_agent(None, vec![plan_block(&["pending"])]);
        for (filter, task, want) in [
            (3, &failed, true),
            (3, &completed, false),
            (2, &completed, true),
            (2, &failed, false),
            (5, &planning, true),
            (0, &planning, true),
        ] {
            assert_eq!(
                task_matches_filters(task, filter, 0, 0),
                want,
                "filter {filter}"
            );
        }
    }
}

/// WorkBuddy-style sidebar filters.
pub const FILTER_STATUS_LABELS: [&str; 6] = [
    "All Status",
    "Working",
    "Completed",
    "Failed",
    "Pending",
    "Planning",
];
pub const FILTER_TIME_LABELS: [&str; 4] =
    ["All Time", "Today", "Last 7 days", "Last 30 days"];

/// The sidebar status filter's five coarse states, derived from a task's
/// turns — no extra bookkeeping, so restored history filters the same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStatus {
    Working,
    Completed,
    Failed,
    Pending,
    Planning,
}

/// A task's status, read off its latest agent turn:
/// - streaming with no events yet is Pending (the agent has not begun);
/// - a trailing plan with unfinished entries is Planning, streaming or not;
/// - a terminal turn whose processing status starts with "Failed" is Failed;
/// - any other streaming turn is Working, any other settled turn Completed.
pub fn task_status(task: &Task) -> TaskStatus {
    let Some((status, blocks, streaming)) =
        task.turns.iter().rev().find_map(|turn| match turn {
            Turn::Agent {
                status,
                blocks,
                streaming,
                ..
            } => Some((status, blocks, *streaming)),
            Turn::User { .. } => None,
        })
    else {
        return TaskStatus::Completed;
    };

    if status.as_deref().is_some_and(|s| s.starts_with("Failed")) {
        return TaskStatus::Failed;
    }
    let planning = match blocks.last() {
        Some(Block::Plan { entries }) => entries
            .iter()
            .any(|entry| entry.status != "completed" && entry.status != "done"),
        _ => false,
    };
    if planning {
        return TaskStatus::Planning;
    }
    if streaming {
        if blocks.is_empty() && status.is_none() {
            return TaskStatus::Pending;
        }
        return TaskStatus::Working;
    }
    TaskStatus::Completed
}

/// Whether the task's latest agent turn is still streaming. The sidebar row
/// shows a spinner in place of its timestamp while this holds — including
/// turns streaming in the background while another task is open.
pub fn task_is_running(task: &Task) -> bool {
    matches!(
        task.turns.iter().rev().find_map(|turn| match turn {
            Turn::Agent { streaming, .. } => Some(*streaming),
            Turn::User { .. } => None,
        }),
        Some(true)
    )
}

/// Whether `task` passes the sidebar status/time filters.
pub fn task_matches_filters(task: &Task, status: usize, time: usize, now: u64) -> bool {
    let current = task_status(task);
    let status_ok = match status {
        0 => true,
        1 => current == TaskStatus::Working,
        2 => current == TaskStatus::Completed,
        3 => current == TaskStatus::Failed,
        4 => current == TaskStatus::Pending,
        5 => current == TaskStatus::Planning,
        _ => true,
    };
    if !status_ok {
        return false;
    }
    let age = match (time, task.updated_at) {
        (0, _) => return true,
        (_, Some(updated)) if updated > 0 => now.saturating_sub(updated),
        _ => return false,
    };
    match time {
        1 => age <= 86_400,
        2 => age <= 7 * 86_400,
        3 => age <= 30 * 86_400,
        _ => true,
    }
}
