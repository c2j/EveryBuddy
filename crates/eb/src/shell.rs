//! Assembly: the three panes, the state -> widget refresh, and the signal wiring.
//!
//! Two rules shape this module:
//!
//!   1. **Widgets never read state directly.** Every handler mutates [`App`]
//!      through a small closure and then asks for a repaint, and
//!      [`Views::refresh`] is the only place that reads state back out. One
//!      direction of data flow means there is exactly one place to look when a
//!      control shows the wrong thing.
//!   2. **Repaints are deferred to an idle callback.** A refresh rebuilds
//!      regions, so a handler that rebuilt the region containing its own button
//!      would destroy the widget mid-signal. Deferring also coalesces a burst of
//!      changes into a single repaint.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;

use eb_api::contrib::Contribs;
use eb_api::AnswerSource;
use crate::agent::{AgentRuntime, Runner};
use crate::composer::{self, Composer, Picker, Variant};
use crate::conversation::{self, Conversation};
use crate::detail::{self, Detail};
use crate::model::{App, Route, Scene, Store, Turn};
use crate::sidebar::{self, Sidebar, TaskRow};
use crate::spec;
use crate::widgets::*;
use crate::welcome::{self, Welcome};

/// Widgets worth measuring in the smoke test, addressed by CSS class.
pub type Probes = Vec<(&'static str, gtk::Widget)>;

fn note(probes: &mut Probes, name: &'static str, widget: &impl WidgetExt) {
    probes.push((name, widget.clone().upcast::<gtk::Widget>()));
}

/// A one-shot indirection to the repaint.
///
/// The widget builders need a "something changed, repaint" callback, but the
/// repaint needs the view tree that those same builders are still producing.
#[derive(Clone)]
pub struct Refresher {
    deferred: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
    immediate: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
    /// Set while a throttled agent refresh is scheduled. Streaming delivers
    /// one event per token, and each refresh tears the conversation down and
    /// rebuilds it; coalescing the burst into one frame-rate rebuild is what
    /// keeps X11 from flickering (Kylin among them).
    pending: Rc<std::cell::Cell<bool>>,
}

impl Refresher {
    pub fn new() -> Refresher {
        Refresher {
            deferred: Rc::new(RefCell::new(None)),
            immediate: Rc::new(RefCell::new(None)),
            pending: Rc::new(std::cell::Cell::new(false)),
        }
    }

    pub fn install(&self, refresh: Rc<dyn Fn()>) {
        *self.deferred.borrow_mut() = Some(refresh);
    }

    /// Install a direct repaint for agent events. UI handlers still use the
    /// deferred path; an agent stream may be the only thing driving a frame
    /// when no input event follows it.
    pub fn install_immediate(&self, refresh: Rc<dyn Fn()>) {
        *self.immediate.borrow_mut() = Some(refresh);
    }

    pub fn request(&self) {
        if let Some(refresh) = self.deferred.borrow().as_ref() {
            refresh();
        }
    }

    pub fn request_now(&self) {
        if std::env::var_os("EB_DEBUG").is_some() {
            eprintln!("refresher: now {:?}", std::thread::current().id());
        }
        if self.pending.replace(true) {
            return; // a rebuild is already scheduled inside this frame window
        }
        let pending = self.pending.clone();
        let immediate = self.immediate.clone();
        let deferred = self.deferred.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(33), move || {
            pending.set(false);
            if let Some(refresh) = immediate.borrow().as_ref() {
                refresh();
            } else if let Some(refresh) = deferred.borrow().as_ref() {
                refresh();
            }
        });
    }
}

pub struct Views {
    pub window: gtk::ApplicationWindow,
    /// sidebar | conversation
    pub inner: gtk::Paned,
    /// (sidebar + conversation) | artifact panel
    pub outer: gtk::Paned,
    pub sidebar: Sidebar,
    pub stack: gtk::Stack,
    pub welcome: Welcome,
    pub conversation: Conversation,
    pub detail: Detail,
    pub unbuilt_root: gtk::Box,
    pub unbuilt_title: gtk::Label,
    unbuilt_note: gtk::Label,
    unbuilt_info: gtk::Box,
    unbuilt_body: gtk::Box,
    pub welcome_composer: Rc<Composer>,
    pub chat_composer: Rc<Composer>,
    /// Every "show sidebar" button: one per route's topbar, since only the
    /// visible route's button can be clicked.
    pub reveals: Vec<gtk::Button>,
    /// Owns the in-flight agent turn and its event drain loop.
    pub agent: Rc<Runner>,
    refresher: Refresher,
    /// Sidebar row click: show that task's conversation. Public so the switch
    /// harness can drive the same path a click does.
    pub open_task: Rc<dyn Fn(usize)>,
    /// The topbar right-panel toggle, reused by the ⇧⌘B accelerator.
    toggle_artifact: Rc<dyn Fn()>,
    /// A sidebar section header was clicked.
    section_toggle: Rc<dyn Fn(usize)>,
    workspace_action: Rc<dyn Fn(crate::sidebar::WorkspaceAction, String)>,
    new_workspace: Rc<dyn Fn()>,
    task_action: Rc<dyn Fn(crate::sidebar::TaskAction, usize)>,
    /// The last `(sidebar_visible, artifact_open, artifact_maximised,
    /// window_width, artifact_min_width)` tuple that positioned the panes.
    /// Tracking it keeps a repaint from undoing a sash the user dragged.
    laid_out: Cell<Option<(bool, bool, bool, i32, i32)>>,
}

/// A pane for a nav row whose screen has not been built.
struct Unbuilt {
    root: gtk::Box,
    title: gtk::Label,
    note: gtk::Label,
    info: gtk::Box,
    body: gtk::Box,
    reveal: gtk::Button,
}

pub fn build(
    application: &gtk::Application,
    store: &Store,
    contribs: &Contribs,
    answer: Arc<dyn AnswerSource>,
    agent_runtime: Arc<AgentRuntime>,
) -> (Views, Probes) {
    let app = store.borrow();
    let refresher = Refresher::new();
    let mut probes = Probes::new();

    let window = gtk::ApplicationWindow::new(application);
    window.set_title(spec::APP_NAME);
    fit_to_workarea(&window);
    window.set_size_request(640, 480);
    class(&window, "eb-window");

    // ---- actions --------------------------------------------------------
    // The nav rows come from the nav plugin; their route names are copied into
    // the handler because GTK handlers are 'static.
    let routes: Vec<&'static str> = contribs.nav.iter().map(|row| row.unbuilt_route).collect();
    let nav = act1(store, &refresher, move |app, index: usize| {
        let route = routes.get(index).copied().unwrap_or("");
        app.select_nav(index, route);
    });
    let open_task = {
        let store = store.clone();
        let refresher = refresher.clone();
        Rc::new(move |index: usize| {
            let mut app = store.borrow_mut();
            app.open_task(index);
            app.model = crate::router::menu_index(
                app.tasks.get(index).and_then(|task| task.agent_route.as_deref()),
            );
            drop(app);
            refresher.request();
        }) as Rc<dyn Fn(usize)>
    };
    let scene = act1(store, &refresher, |app, scene: Scene| app.set_scene(scene));
    let agent = Runner::new(store.clone(), refresher.clone(), answer, agent_runtime, window.clone());
    let submit = {
        let agent = agent.clone();
        Rc::new(move |text: String| {
            if std::env::var_os("EB_DEBUG").is_some() {
                eprintln!("submit: {} chars", text.chars().count());
            }
            agent.submit(text)
        })
    };
    let pick = {
        let store = store.clone();
        let refresher = refresher.clone();
        let window = window.clone();
        Rc::new(move |picker: Picker, index: usize| {
            let mut app = store.borrow_mut();
            // "+ New Workspace" / "Open Local Folder" run their own dialogs
            // and select the result themselves.
            let workspace_count = app.workspaces.len();
            if picker == Picker::Workspace && index >= workspace_count + 2 {
                drop(app);
                if index == workspace_count + 2 {
                    crate::workspace_ui::prompt_new(&window, &store, &refresher);
                } else {
                    crate::workspace_ui::open_local(&window, &store, &refresher);
                }
                return;
            }
            match picker {
                Picker::Model => {
                    app.model = index;
                    let route = crate::router::route_for_menu(index).map(str::to_string);
                    if let Route::Task(task_index) = app.route {
                        if let Some(task) = app.tasks.get_mut(task_index) {
                            task.agent_route = route;
                        }
                    }
                }
                Picker::Workspace => {
                    app.workspace = index;
                    let task_index = match app.route {
                        Route::Task(index) if app.tasks.get(index).is_some() => Some(index),
                        _ => None,
                    };
                    let target = if index == 0 {
                        // "Auto workspace" only affects the next new task; it
                        // must not silently detach an existing task.
                        None
                    } else if index == 1 {
                        Some(None)
                    } else if index <= app.workspaces.len() + 1 {
                        Some(Some(app.workspaces[index - 2].id.clone()))
                    } else {
                        let title = task_index
                            .and_then(|index| app.tasks.get(index))
                            .map(|task| task.title.clone())
                            .unwrap_or_else(|| "Workspace".into());
                        crate::workspace::create(&title).map(|workspace| {
                            let id = workspace.id.clone();
                            app.workspaces.push(workspace);
                            app.workspace = app.workspaces.len() + 1;
                            Some(id)
                        })
                    };
                    if let (Some(index), Some(workspace_id)) = (task_index, target) {
                        if let Some(task) = app.tasks.get_mut(index) {
                            task.workspace_id = workspace_id;
                            task.updated_at = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|time| time.as_secs())
                                .ok();
                        }
                    }
                }
                Picker::Permission => {
                    app.permission = index;
                    crate::router::persist_permission_mode(index);
                }
                // The add-context menu performs an action; there is nothing to
                // remember between openings.
                Picker::Add => {}
            }
            drop(app);
            // Unassigning can leave an empty Temporary workspace behind.
            let removed = {
                let mut app = store.borrow_mut();
                let crate::model::App { workspaces, tasks, .. } = &mut *app;
                let removed = crate::workspace::cleanup_temporary(workspaces, tasks);
                if removed > 0 && app.workspace > 1 {
                    app.workspace = 0;
                }
                removed
            };
            let _ = removed;
            crate::state::save(&store);
            refresher.request();
        }) as Rc<dyn Fn(Picker, usize)>
    };
    let collapse = act0(store, &refresher, |app| app.sidebar_visible = false);
    let reveal = act0(store, &refresher, |app| app.sidebar_visible = true);
    let maximise = act0(store, &refresher, |app| {
        app.detail_maximised = !app.detail_maximised
    });
    let feedback_root: Rc<RefCell<Option<gtk::Box>>> = Rc::new(RefCell::new(None));
    let feedback_outer: Rc<RefCell<Option<gtk::Paned>>> = Rc::new(RefCell::new(None));
    let hide_detail_base = act0(store, &refresher, |app| {
        app.detail_visible = false;
        // Stay closed for this task even when further files arrive.
        app.artifact_dismissed = true;
    });
    let hide_detail: Rc<dyn Fn()> = {
        let base = hide_detail_base.clone();
        let feedback_root = feedback_root.clone();
        let feedback_outer = feedback_outer.clone();
        Rc::new(move || {
            #[cfg(target_os = "macos")]
            {
                crate::theme_view::close_panel();
                crate::theme_view::close_embedded();
            }
            if let Some(root) = feedback_root.borrow().clone() {
                root.set_size_request(-1, -1);
            }
            if let Some(outer) = feedback_outer.borrow().clone() {
                outer.set_position(spec::WINDOW_WIDTH);
            }
            base();
        })
    };
    // A tab's `×` drops that one file from the task's strip. Closing the last
    // tab hides the panel (without the header close's permanent-dismiss flag,
    // so the next written file still opens it); the reopen affordance covers
    // the rest.
    let close_tab: Rc<dyn Fn(usize)> = act1(store, &refresher, |app, index| {
        let Route::Task(task) = app.route.clone() else {
            return;
        };
        let Some(entry) = app.tasks.get_mut(task) else { return };
        if index >= entry.artifacts.len() {
            return;
        }
        entry.artifacts.remove(index);
        entry.artifact = entry.artifacts.last().cloned();
        let empty = entry.artifacts.is_empty();
        if empty {
            app.detail_visible = false;
        }
    });
    let filter_state: Rc<dyn Fn() -> (usize, usize)> = {
        let store = store.clone();
        Rc::new(move || {
            let app = store.borrow();
            (app.filter_status, app.filter_time)
        })
    };
    let filter_select: Rc<dyn Fn(usize, usize)> = {
        let store = store.clone();
        let refresher = refresher.clone();
        Rc::new(move |kind, index| {
            {
                let mut app = store.borrow_mut();
                match kind {
                    0 => app.filter_status = index,
                    _ => app.filter_time = index,
                }
            }
            refresher.request();
        })
    };
    let filter_reset: Rc<dyn Fn()> = {
        let store = store.clone();
        let refresher = refresher.clone();
        Rc::new(move || {
            {
                let mut app = store.borrow_mut();
                app.filter_status = 0;
                app.filter_time = 0;
            }
            refresher.request();
        })
    };
    /// The WorkBuddy-style global search popover: an entry, All / Tasks /
    /// Spaces / Artifacts / Library tabs, live results, and the keyboard
    /// hints footer.
    let global_search: Rc<dyn Fn(&gtk::Button)> = {
        let store = store.clone();
        let open_task = open_task.clone();
        let refresher = std::rc::Rc::new(refresher.clone());
        Rc::new(move |anchor: &gtk::Button| {
            let popover = gtk::Popover::new(Some(anchor));
            popover.set_position(gtk::PositionType::Bottom);
            crate::widgets::class(&popover, "eb-menu");
            let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
            crate::widgets::class(&column, "eb-menu-body");
            column.set_size_request(520, -1);

            // GtkSearchEntry paints `edit-find-symbolic`, then
            // `edit-clear-symbolic` once the field has text. The bare Debian 10
            // bundle ships neither icon. GTK falls back to its built-in
            // missing-image PNG, fails to decode that too, and aborts in
            // `gtk_icon_helper`. A plain entry never asks the icon theme; our
            // own pixbufs are optional and simply omitted when they do not decode.
            let entry = gtk::Entry::new();
            entry.set_placeholder_text(Some("Search"));
            entry.set_margin_top(10);
            entry.set_margin_start(12);
            entry.set_margin_end(12);
            let entry_icon = |name: &str| {
                crate::icons::image(name, 16).and_then(|widget| widget.pixbuf())
            };
            let clear_icon = entry_icon("close");
            if let Some(icon) = entry_icon("search") {
                entry.set_icon_from_pixbuf(gtk::EntryIconPosition::Primary, Some(&icon));
            }
            entry.connect_icon_press(|entry, position, _| {
                if position == gtk::EntryIconPosition::Secondary {
                    entry.set_text("");
                }
            });
            column.pack_start(&entry, false, false, 0);

            let tab_state: Rc<Cell<usize>> = Rc::new(Cell::new(0));
            let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            crate::widgets::class(&tabs, "eb-search-tabs");
            for (index, label) in ["All", "Tasks", "Spaces", "Artifacts", "Library"]
                .iter()
                .enumerate()
            {
                let tab = flat_button("eb-menu-item");
                tab.add(&text(label, "eb-menu-item-label"));
                let tab_state = tab_state.clone();
                let label = (*label).to_string();
                tab.connect_clicked(move |_| {
                    // Tab order mirrors the labels above.
                    let index = match label.as_str() {
                        "All" => 0,
                        "Tasks" => 1,
                        "Spaces" => 2,
                        "Artifacts" => 3,
                        _ => 4,
                    };
                    tab_state.set(index);
                });
                tabs.pack_start(&tab, false, false, 0);
            }
            tabs.set_margin_top(8);
            tabs.set_margin_start(12);
            tabs.set_margin_end(12);
            column.pack_start(&tabs, false, false, 0);

            let results = gtk::Box::new(gtk::Orientation::Vertical, 0);
            crate::widgets::class(&results, "eb-search-results");
            column.pack_start(&results, false, false, 0);

            let hints = text(
                "↑↓ Navigate   ↵ Open   ESC Close",
                "eb-search-hints",
            );
            hints.set_halign(gtk::Align::Center);
            hints.set_margin_top(8);
            hints.set_margin_bottom(8);
            column.pack_start(&hints, false, false, 0);

            let rebuild: Rc<dyn Fn()> = {
                let store = store.clone();
                let entry = entry.clone();
                let tab_state = tab_state.clone();
                let results = results.clone();
                let open_task = open_task.clone();
                let refresher = refresher.clone();
                Rc::new(move || {
                    for child in results.children() {
                        results.remove(&child);
                    }
                    let query = entry.text().to_lowercase();
                    let tab = tab_state.get();
                    let app = store.borrow();
                    let matches = |text: &str| {
                        !query.is_empty() && text.to_lowercase().contains(&query)
                    };
                    let turn_text = |turn: &eb_api::model::Turn| match turn {
                        eb_api::model::Turn::User { text, .. } => text.clone(),
                        eb_api::model::Turn::Agent { blocks, .. } => blocks
                            .iter()
                            .filter_map(|block| match block {
                                eb_api::model::Block::Text(text) => Some(text.clone()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                    };
                    let mut rows: Vec<(String, usize)> = Vec::new();
                    if tab == 0 || tab == 1 {
                        for (index, task) in app.tasks.iter().enumerate() {
                            if matches(&task.title)
                                || task
                                    .turns
                                    .iter()
                                    .any(|turn| matches(&turn_text(turn)))
                            {
                                rows.push((task.title.clone(), index));
                            }
                        }
                    }
                    if tab == 0 || tab == 2 {
                        for workspace in &app.workspaces {
                            let name = crate::workspace::display_name(workspace);
                            if matches(&name) {
                                rows.push((name, usize::MAX));
                            }
                        }
                    }
                    if tab == 0 || tab == 3 {
                        for task in &app.tasks {
                            if let Some(artifact) = &task.artifact {
                                if matches(&artifact.name) {
                                    rows.push((artifact.name.clone(), usize::MAX));
                                }
                            }
                        }
                    }
                    if rows.is_empty() {
                        let empty = text("Type a keyword to search", "eb-search-empty");
                        empty.set_halign(gtk::Align::Center);
                        empty.set_margin_top(24);
                        empty.set_margin_bottom(24);
                        results.pack_start(&empty, false, false, 0);
                    }
                    for (label, index) in rows {
                        let row = flat_button("eb-menu-item");
                        row.add(&text(&label, "eb-menu-item-label"));
                        let store = store.clone();
                        let open_task = open_task.clone();
                        let refresher = refresher.clone();
                        row.connect_clicked(move |_| {
                            if index != usize::MAX {
                                store.borrow_mut().route =
                                    eb_api::model::Route::Task(index);
                                refresher.request();
                            }
                        });
                        results.pack_start(&row, false, false, 0);
                    }
                    results.show_all();
                })
            };
            {
                let rebuild = rebuild.clone();
                let clear_icon = clear_icon.clone();
                entry.connect_changed(move |entry| {
                    entry.set_icon_from_pixbuf(
                        gtk::EntryIconPosition::Secondary,
                        if entry.text().is_empty() {
                            None
                        } else {
                            clear_icon.as_ref()
                        },
                    );
                    rebuild();
                });
            }
            {
                let rebuild = rebuild.clone();
                for tab in tabs.children() {
                    if let Some(tab) = tab.downcast_ref::<gtk::Button>() {
                        let rebuild = rebuild.clone();
                        tab.connect_clicked(move |_| rebuild());
                    }
                }
            }
            rebuild();
            popover.add(&column);
            column.show_all();
            crate::widgets::hide_from_show_all(&popover);
            popover.popup();
        })
    };
    let section = act1(store, &refresher, |app, index: usize| {
        if let Some(slot) = app.collapsed.get_mut(index) {
            *slot = !*slot;
        }
    });

    // ---- composers ------------------------------------------------------
    // Built once and never rebuilt: a rebuild would discard typed text. Two
    // instances exist because a GTK widget has exactly one parent and the two
    // routes measure differently.
    let cancel = {
        let agent = agent.clone();
        Rc::new(move || agent.cancel())
    };
    let workspace_items = {
        let store = store.clone();
        Rc::new(move || {
            let app = store.borrow();
            let mut items = vec!["Auto workspace".to_string(), "No workspace".to_string()];
            items.extend(app.workspaces.iter().map(crate::workspace::display_name));
            items.push("New Workspace…".to_string());
            items.push("Open Local Folder…".to_string());
            items
        }) as Rc<dyn Fn() -> Vec<String>>
    };
    let composer_wiring = composer::Wiring {
        submit: submit.clone(),
        cancel,
        pick: pick.clone(),
        workspace_items,
    };
    let welcome_composer = Rc::new(Composer::new(
        Variant::Welcome,
        app.model,
        app.workspace,
        app.permission,
        composer_wiring.clone(),
        contribs,
    ));
    let chat_composer = Rc::new(Composer::new(
        Variant::Chat,
        app.model,
        app.workspace,
        app.permission,
        composer_wiring,
        contribs,
    ));
    agent.set_permission_anchor(chat_composer.root.clone().upcast());

    let chip = {
        let composer = welcome_composer.clone();
        let prompts: Vec<&'static str> = contribs.chips.iter().map(|c| c.prompt).collect();
        Rc::new(move |index: usize| {
            if let Some(prompt) = prompts.get(index) {
                composer.fill(prompt);
            }
        })
    };

    // ---- panes ----------------------------------------------------------
    let task_action: Rc<dyn Fn(crate::sidebar::TaskAction, usize)> = {
        let store = store.clone();
        let refresher = refresher.clone();
        let window = window.clone();
        Rc::new(move |action, index| {
            crate::workspace_ui::run_task(&window, &store, &refresher, action, index);
        })
    };
    let workspace_action: Rc<dyn Fn(crate::sidebar::WorkspaceAction, String)> = {
        let store = store.clone();
        let refresher = refresher.clone();
        let window = window.clone();
        Rc::new(move |action, id| {
            crate::workspace_ui::run(&window, &store, &refresher, action, &id);
        })
    };
    let new_workspace: Rc<dyn Fn()> = {
        let store = store.clone();
        let refresher = refresher.clone();
        let window = window.clone();
        Rc::new(move || {
            crate::workspace_ui::prompt_new(&window, &store, &refresher);
        })
    };
    let sidebar = sidebar::build(
        &sidebar::Wiring {
            nav: nav.clone(),
            collapse: collapse.clone(),
            filter_state: filter_state.clone(),
            filter_select: filter_select.clone(),
            filter_reset: filter_reset.clone(),
            search: global_search.clone(),
            account: std::rc::Rc::new(|index| {
                if index == 0 {
                    crate::settings::open();
                }
            }),
            workspace_action: workspace_action.clone(),
            new_workspace: new_workspace.clone(),
            task_action: {
                let store = store.clone();
                let refresher = refresher.clone();
                let window = window.clone();
                Rc::new(move |action, index| {
                    crate::workspace_ui::run_task(&window, &store, &refresher, action, index);
                })
            },
        },
        contribs,
    );
    note(&mut probes, "eb-sidebar", &sidebar.root);
    note(&mut probes, "eb-sidebar-topbar", &sidebar.topbar);
    note(&mut probes, "eb-sidebar-header", &sidebar.header);
    note(&mut probes, "eb-sidebar-content", &sidebar.content);
    note(&mut probes, "eb-sidebar-footer", &sidebar.footer);

    let welcome = welcome::build(
        &welcome_composer,
        &welcome::Wiring {
            scene: scene.clone(),
            chip,
            reveal: reveal.clone(),
        },
        contribs,
    );
    note(&mut probes, "eb-content-topbar", &welcome.topbar);
    note(&mut probes, "eb-hero", &welcome.hero);
    note(&mut probes, "eb-segmented", &welcome.segmented);
    note(&mut probes, "eb-chips", &welcome.chips);
    note(&mut probes, "eb-composer-card", &welcome_composer.card);
    note(&mut probes, "eb-composer-toolbar", &welcome_composer.toolbar);
    note(&mut probes, "eb-composer-footer", &welcome.footer);
    note(&mut probes, "eb-notice", &welcome.notice);

    let open_artifact: Rc<dyn Fn()> = {
        let store = store.clone();
        let refresher = refresher.clone();
        Rc::new(move || {
            {
                let mut app = store.borrow_mut();
                app.artifact_dismissed = false;
                let has = match &app.route {
                    crate::model::Route::Task(index) => app
                        .tasks
                        .get(*index)
                        .and_then(|task| task.artifact.as_ref())
                        .is_some(),
                    _ => false,
                };
                app.detail_visible = has;
            }
            refresher.request();
        })
    };
    // An artifact card (or "View all artifacts") in a message: open the panel
    // and take the tab pick with it, consumed by the next paint.
    let open_artifact_at: Rc<dyn Fn(usize)> = {
        let store = store.clone();
        let refresher = refresher.clone();
        Rc::new(move |index| {
            {
                let mut app = store.borrow_mut();
                app.artifact_dismissed = false;
                app.artifact_select = Some(index);
                let has = match &app.route {
                    crate::model::Route::Task(index) => app
                        .tasks
                        .get(*index)
                        .and_then(|task| task.artifact.as_ref())
                        .is_some(),
                    _ => false,
                };
                app.detail_visible = has;
            }
            refresher.request();
        })
    };
    // The topbar panel toggle: close when open, open when dismissed (and a
    // no-op on a task with no files, same as `open_artifact`).
    let toggle_artifact: Rc<dyn Fn()> = {
        let store = store.clone();
        let open = open_artifact.clone();
        let hide = hide_detail.clone();
        Rc::new(move || {
            let open_now = {
                let app = store.borrow();
                match &app.route {
                    crate::model::Route::Task(index) => {
                        app.detail_visible
                            && app
                                .tasks
                                .get(*index)
                                .and_then(|task| task.artifact.as_ref())
                                .is_some()
                    }
                    _ => false,
                }
            };
            if open_now {
                hide();
            } else {
                open();
            }
        })
    };
    let conversation = conversation::build(
        &conversation::Wiring {
            reveal: reveal.clone(),
            toggle_artifact: toggle_artifact.clone(),
            open_artifact_at: open_artifact_at.clone(),
            // "Edit and resend" refills the chat composer, exactly as if the
            // user had typed the message again; "Regenerate" resubmits the
            // prompt through the normal submit path.
            edit: {
                let composer = chat_composer.clone();
                Rc::new(move |text: String| composer.fill(&text))
            },
            submit: submit.clone(),
            // Scrolling to the top of a long conversation asks for a repaint,
            // which is where the earlier slice of the transcript is attached.
            load_more: {
                let refresher = refresher.clone();
                Rc::new(move || refresher.request())
            },
            // The inline tool-permission card hands the user's decision back
            // to the waiting worker and repaints so the card becomes the
            // tool's outcome block.
            resolve_permission: {
                let store = store.clone();
                let refresher = refresher.clone();
                Rc::new(move |decision| {
                    let route = store.borrow().route.clone();
                    if let Route::Task(index) = route {
                        if let Some(pending) = store.borrow_mut().take_pending_permission(index)
                        {
                            let _ = pending.responder.respond(decision.clone());
                        }
                    }
                    refresher.request();
                })
            },
        },
        &chat_composer,
    );
    note(&mut probes, "eb-chat-topbar", &conversation.topbar);
    note(&mut probes, "eb-chat-actions", &conversation.actions);
    note(&mut probes, "eb-messages", &conversation.messages);
    note(&mut probes, "eb-input-area", &chat_composer.root);
    note(&mut probes, "eb-chat-card", &chat_composer.card);

    let feedback_anchor: Rc<RefCell<Option<gtk::Box>>> = Rc::new(RefCell::new(None));
    let open_feedback: Rc<dyn Fn(&'static str)> = {
        let anchor = feedback_anchor.clone();
        let store = store.clone();
        let feedback_root = feedback_root.clone();
        let feedback_outer = feedback_outer.clone();
        Rc::new(move |plugin_id: &'static str| {
            #[cfg(target_os = "macos")]
            {
                let Some(anchor) = anchor.borrow().clone() else { return };
                let Some(url) = crate::plugin_panel::url(plugin_id) else { return };
                if let Some(root) = feedback_root.borrow().clone() {
                    root.set_size_request(520, -1);
                }
                if let Some(outer) = feedback_outer.borrow().clone() {
                    if let Some(window) = anchor.toplevel().and_downcast::<gtk::Window>() {
                        outer.set_position(window.size().0 - 520);
                    }
                }
                while gtk::events_pending() {
                    gtk::main_iteration_do(false);
                }
                let _ = plugin_id;
                crate::bridge::ensure();
                crate::preview_server::ensure();
                let anchor = anchor.clone();
                gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                    if std::env::var_os("EB_PANEL_OVERLAY").is_some() {
                        crate::theme_view::open_panel(url, anchor.upcast_ref());
                    } else {
                        crate::theme_view::open_embedded(url, anchor.upcast_ref());
                    }
                });
            }
        })
    };
    let detail = detail::build(
        &detail::Wiring {
            close: hide_detail.clone(),
            tab_close: close_tab,
            expand: maximise,
            plugin: open_feedback,
        },
        contribs,
    );
    *feedback_anchor.borrow_mut() = Some(detail.body.clone());
    *feedback_root.borrow_mut() = Some(detail.root.clone());
    note(&mut probes, "eb-detail", &detail.root);
    note(&mut probes, "eb-detail-header", &detail.header);

    let unbuilt = unbuilt_pane(&reveal);

    // ---- stack ----------------------------------------------------------
    let stack = gtk::Stack::new();
    stack.set_transition_type(gtk::StackTransitionType::None);
    stack.add_named(&welcome.root, "welcome");
    stack.add_named(&conversation.root, "conversation");
    stack.add_named(&unbuilt.root, "unbuilt");

    // ---- paned shell ----------------------------------------------------
    // Nesting matters: the outer sash spans the full height (as the capture
    // shows), which only happens if the sidebar lives inside the outer paned's
    // *first* child rather than beside the detail panel.
    let inner = gtk::Paned::new(gtk::Orientation::Horizontal);
    inner.set_position(spec::SIDEBAR_WIDTH);
    class(&inner, "eb-split");
    inner.pack1(&sidebar.root, false, false);
    inner.pack2(&stack, true, false);

    let outer = gtk::Paned::new(gtk::Orientation::Horizontal);
    outer.set_position(spec::WINDOW_WIDTH);
    *feedback_outer.borrow_mut() = Some(outer.clone());
    class(&outer, "eb-split");
    // Packing: the artifact panel is the *resize* child of the outer paned.
    // GtkPaned's allocation (gtkpaned.c, `gtk_paned_calc_position`) rewrites
    // the position on every frame: a `resize=false` second child silently
    // pulls the sash back to `window - its natural width`, overriding
    // `set_position`. With the panel as the resize child we own the sash:
    // position = window - 580 leaves the panel the remaining 580.
    outer.pack1(&inner, false, false);
    outer.pack2(&detail.root, true, true);

    window.add(&outer);

    // Collected before the view structs are moved into `Views`.
    let reveals = vec![
        welcome.reveal.clone(),
        conversation.reveal.clone(),
        unbuilt.reveal.clone(),
    ];

    let views = Views {
        window,
        inner,
        outer,
        sidebar,
        stack,
        welcome,
        conversation,
        detail,
        unbuilt_root: unbuilt.root,
        unbuilt_title: unbuilt.title,
        unbuilt_note: unbuilt.note,
        unbuilt_info: unbuilt.info,
        unbuilt_body: unbuilt.body,
        welcome_composer,
        chat_composer,
        reveals,
        agent,
        refresher,
        open_task,
        toggle_artifact,
        section_toggle: section,
        workspace_action,
        new_workspace,
        task_action,
        laid_out: Cell::new(None),
    };

    (views, probes)
}

/// Keep the 1440x868 design size when the desktop has room, but never open
/// larger than the monitor workarea. Kylin desktops are often 1366x768; a
/// 1440x868 window there hangs off the screen and clips the composer.
fn session_timestamp(session_id: &str) -> Option<u64> {
    let raw = session_id.rsplit('-').next()?.parse::<u64>().ok()?;
    Some(if raw > 1_000_000_000_000 { raw / 1_000 } else { raw })
}

/// The sidebar clock, mirrored from `formatSidebarConversationTime` in the
/// product bundle: `Just now` / `{n}m` / `{n}h` / `Yesterday` / `{n}d`, then a
/// date once a week has passed - `Oct 28` inside the current year, `25/10/28`
/// across a year end.
///
/// The `d` rung counts *local calendar* days, not 86400s, which is why it goes
/// through GLib: `Yesterday` has to flip at local midnight, not at UTC midnight.
fn sidebar_time(updated_at: u64, now: u64) -> String {
    let elapsed = now.saturating_sub(updated_at);
    if elapsed < 60 {
        return "Just now".to_string();
    }
    if elapsed < 3_600 {
        return format!("{}m", elapsed / 60);
    }
    if elapsed < 86_400 {
        return format!("{}h", elapsed / 3_600);
    }
    let days = local_day_number(now).saturating_sub(local_day_number(updated_at));
    if days < 7 {
        return if days <= 1 {
            "Yesterday".to_string()
        } else {
            format!("{days}d")
        };
    }
    let (year, month, day) = local_ymd(updated_at as i64);
    let (this_year, _, _) = local_ymd(now as i64);
    if year == this_year {
        format!(
            "{} {}",
            MONTH_LABELS[usize::try_from(month - 1).unwrap_or(0)],
            day
        )
    } else {
        format!("{:02}/{:02}/{:02}", year.rem_euclid(100), month, day)
    }
}

const MONTH_LABELS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn local_ymd(epoch: i64) -> (i32, i32, i32) {
    gtk::glib::DateTime::from_unix_local(epoch)
        .map(|time| time.ymd())
        .unwrap_or((1970, 1, 1))
}

fn local_day_number(epoch: u64) -> i64 {
    let (year, month, day) = local_ymd(epoch as i64);
    days_from_civil(i64::from(year), i64::from(month), i64::from(day))
}

/// Days from 1970-01-01 to a proleptic Gregorian date (Hinnant's civil-days),
/// so two local dates can be compared without a timezone table.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let shifted_year = if month <= 2 { year - 1 } else { year };
    let era = if shifted_year >= 0 {
        shifted_year
    } else {
        shifted_year - 399
    } / 400;
    let year_of_era = shifted_year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era =
        year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn fit_to_workarea(window: &gtk::ApplicationWindow) {
    // EB_WINDOW_WIDTH/EB_WINDOW_HEIGHT override the fitted size so the
    // resize-sensitive layout can be screenshotted and asserted at a chosen
    // width (the sash positions derive from the window width).
    let override_size = || -> Option<(i32, i32)> {
        let width = std::env::var("EB_WINDOW_WIDTH").ok()?.parse::<i32>().ok()?;
        let height = std::env::var("EB_WINDOW_HEIGHT").ok()?.parse::<i32>().ok()?;
        Some((width, height))
    };
    let (width, height) = override_size().unwrap_or_else(|| {
        let area = gtk::gdk::Display::default()
            .and_then(|display| display.primary_monitor().or_else(|| display.monitor(0)))
            .map(|monitor| monitor.workarea());
        match area {
            Some(area) if area.width() >= spec::WINDOW_WIDTH
                && area.height() >= spec::WINDOW_HEIGHT =>
            {
                (spec::WINDOW_WIDTH, spec::WINDOW_HEIGHT)
            }
            Some(area) => (
                (area.width() - 16).clamp(640, spec::WINDOW_WIDTH),
                (area.height() - 56).clamp(480, spec::WINDOW_HEIGHT),
            ),
            None => (spec::WINDOW_WIDTH, spec::WINDOW_HEIGHT),
        }
    });
    window.set_default_size(width, height);
    window.set_position(gtk::WindowPosition::Center);
}

/// Finish wiring a view tree: install the repaint and paint the first frame.
pub fn finish(views: &Rc<Views>, store: &Store) {
    let refresh: Rc<dyn Fn()> = {
        let weak = Rc::downgrade(views);
        let store = store.clone();
        Rc::new(move || {
            let Some(views) = weak.upgrade() else {
                return;
            };
            let store = store.clone();
            // Deferred: see the module comment.
            gtk::glib::idle_add_local_once(move || views.refresh(&store));
        })
    };
    views.refresher.install(refresh);
    let immediate: Rc<dyn Fn()> = {
        let weak = Rc::downgrade(views);
        let store = store.clone();
        Rc::new(move || {
            if let Some(views) = weak.upgrade() {
                views.refresh(&store);
            }
        })
    };
    views.refresher.install_immediate(immediate.clone());
    views.agent.set_immediate_refresh(immediate);

    // The panes are positioned from the window width, so a resize has to
    // repaint. Without this the sash keeps the position from the last layout
    // and, say, an artifact panel opened after maximising swallows the extra
    // width. Two rules the handler has to get right:
    //   * Return `false`. configure-event is RUN_LAST, so a handler that returns
    //     true stops GTK's own `gtk_window_configure_event` — the only place the
    //     toplevel records the new frame size. The window server then stretches
    //     the stale surface over the resized frame: the "sidebar fills the whole
    //     window, no conversation" report, where 280px of sidebar was drawn
    //     across a 1470px window.
    //   * Compare the width the *event* carries, not `window.size()`. This
    //     handler runs before the default one, so the cached size is still the
    //     previous frame's; comparing stale against stale misses a resize
    //     outright and repaints one event late. The repaint itself is
    //     idle-deferred, so `refresh` reads the window after GTK has recorded
    //     the new size.
    // Configure events also fire on moves; only a width change counts.
    let last_width = std::cell::Cell::new(0i32);
    views.window.connect_configure_event({
        let refresher = views.refresher.clone();
        move |_window, event| {
            let width = event.size().0 as i32;
            if last_width.replace(width) != width {
                refresher.request();
            }
            false
        }
    });

    // The shortcuts the captured tooltips advertise: ⌘F expands the
    // conversation search, ⇧⌘B toggles the right panel. GTK on macOS reports
    // ⌘ as CONTROL_MASK.
    {
        let bar = views.conversation.search_bar.clone();
        let button = views.conversation.search_button.clone();
        let entry = views.conversation.search_entry.clone();
        let toggle = views.toggle_artifact.clone();
        views.window.connect_key_press_event(move |_, event| {
            let mods = event.state();
            if !mods.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
                return gtk::glib::Propagation::Proceed;
            }
            let shift = mods.contains(gtk::gdk::ModifierType::SHIFT_MASK);
            let key = event.keyval();
            if key == gtk::gdk::keys::constants::f && !shift {
                button.set_visible(false);
                bar.set_visible(true);
                entry.grab_focus();
                gtk::glib::Propagation::Stop
            } else if key == gtk::gdk::keys::constants::b && shift {
                toggle();
                gtk::glib::Propagation::Stop
            } else {
                gtk::glib::Propagation::Proceed
            }
        });
    }

    // Poll the persisted schedule queue. A due job becomes a normal task, so
    // it uses the same workspace/session/agent path as a typed prompt.
    let weak = Rc::downgrade(views);
    gtk::glib::timeout_add_seconds_local(5, move || {
        if let Some(views) = weak.upgrade() {
            for (title, prompt) in crate::catalog::take_due() {
                if !prompt.is_empty() {
                    eprintln!("scheduler: running '{title}'");
                    views.agent.submit(prompt);
                }
            }
        }
        gtk::glib::ControlFlow::Continue
    });

    // `show_all` first, then repaint: `refresh` is what sets the pane
    // visibilities, and running it afterwards means no frame is drawn in
    // between.
    // A window restored off-screen (macOS can remember a position on a display
    // above the built-in one) makes the app look dead even though events apply.
    // Move it to the primary monitor's origin; GDK monitor coordinates are not
    // always the same as CGWindow's, especially with a display arranged above.
    if let Some(display) = gtk::gdk::Display::default() {
        if let Some(monitor) = display.primary_monitor() {
            let geometry = monitor.geometry();
            views.window.move_(geometry.x(), geometry.y());
        }
        if std::env::var_os("EB_DEBUG").is_some() {
            for index in 0..display.n_monitors() {
                if let Some(monitor) = display.monitor(index) {
                    eprintln!(
                        "monitor {index}: {:?} primary={}",
                        monitor.geometry(),
                        monitor.is_primary()
                    );
                }
            }
        }
    }
    views.window.show_all();
    views.refresh(store);
}

impl Views {
    /// Repaint everything from application state.
    ///
    /// Regions are rebuilt rather than diffed. That is cheap at this scale and
    /// removes a whole class of "the widget and the state disagree" bugs.
    pub fn refresh(self: &Rc<Self>, store: &Store) {
        // The task's own `turns` are the source of truth. The dsh bridge
        // projection is global (one live session), so consuming it here used
        // to overwrite the selected task with another task's transcript.
        let debug = std::env::var_os("EB_DEBUG").is_some();
        let started_at = std::time::Instant::now();
        if debug {
            eprintln!("refresh: enter {:?}", std::thread::current().id());
        }
        // The one-shot tab pick from a clicked artifact card, taken before the
        // state borrow below pins the store.
        let artifact_select = store.borrow_mut().artifact_select.take();
        let app = store.borrow();
        if std::env::var_os("EB_DEBUG").is_some() {
            eprintln!("refresh: begin tasks={}", app.tasks.len());
        }

        // ---- the send button becomes stop while an answer streams -------
        let streaming = match &app.route {
            Route::Task(index) => app
                .tasks
                .get(*index)
                .and_then(|task| {
                    task.turns.iter().rev().find_map(|turn| match turn {
                        Turn::Agent { streaming, .. } => Some(*streaming),
                        Turn::User { .. } => None,
                    })
                })
                .unwrap_or(false),
            _ => false,
        };
        self.welcome_composer.set_streaming(false);
        self.chat_composer.set_streaming(streaming);

        // ---- selection classes ------------------------------------------
        paint(&self.sidebar.nav, app.nav, "eb-nav-active");        let scene = match app.scene {
            Scene::Work => 0,
            Scene::Code => 1,
        };
        paint(&self.welcome.segments, Some(scene), "eb-segment-active");
        self.welcome_composer.set_scene(app.scene);
        self.chat_composer.set_scene(app.scene);
        self.welcome_composer.set_model_selection(app.model);
        self.chat_composer.set_model_selection(app.model);
        let (workspace_index, workspace_name) = match app.route {
            Route::Task(index) => app
                .tasks
                .get(index)
                .and_then(|task| task.workspace_id.as_deref())
                .and_then(|id| {
                    app.workspaces
                        .iter()
                        .position(|workspace| workspace.id == id)
                        .map(|position| {
                            (
                                position + 2,
                                crate::workspace::display_name(&app.workspaces[position]),
                            )
                        })
                })
                .unwrap_or((1, "No workspace".to_string())),
            _ => match app.workspace {
                0 => (0, "Auto workspace".to_string()),
                1 => (1, "No workspace".to_string()),
                index => app
                    .workspaces
                    .get(index.saturating_sub(2))
                    .map(|workspace| {
                        (index, crate::workspace::display_name(workspace))
                    })
                    .unwrap_or((0, "Auto workspace".to_string())),
            },
        };
        self.welcome_composer
            .set_workspace_selection(workspace_index, &workspace_name);
        self.chat_composer
            .set_workspace_selection(workspace_index, &workspace_name);
        // WorkBuddy hides the picker whenever an existing task is open
        // (assigned or not); the landing composer keeps it: there the choice
        // still drives where the next task lands.
        let task_open = matches!(app.route, Route::Task(_));
        self.chat_composer.set_workspace_visible(!task_open);

        let usage = match &app.route {
            Route::Task(index) => app.tasks.get(*index).and_then(|task| task.usage),
            _ => None,
        };
        self.welcome_composer.set_usage(None);
        self.chat_composer.set_usage(usage);

        // ---- sidebar list ------------------------------------------------
        let stage = std::time::Instant::now();
        let now_secs = eb_api::model::unix_now();
        let rows: Vec<TaskRow> = app
            .tasks
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, task)| {
                eb_api::model::task_matches_filters(task, app.filter_status, app.filter_time, now_secs)
            })
            .map(|(index, task)| TaskRow {
                title: task.title.clone(),
                meta: task
                    .updated_at
                    .or_else(|| task.session_id.as_deref().and_then(session_timestamp))
                    .map(|updated| sidebar_time(updated, now_secs))
                    .unwrap_or_else(|| task.meta.clone()),
                active: matches!(app.route, Route::Task(active) if active == index),
                status: eb_api::model::task_status(task),
                failure_unseen: task.failure_unseen,
                unread: task.unread,
                index,
                workspace_id: task.workspace_id.clone(),
                pinned: task.pinned,
                archived: task.archived,
            })
            .collect();
        // Pinned tasks float to the top of their section and archived ones
        // sink to the bottom; the stable sort keeps newest-first within each
        // group.
        let mut rows = rows;
        rows.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(a.archived.cmp(&b.archived)));
        sidebar::fill_content(
            &self.sidebar.content,
            &rows,
            &app.workspaces,
            app.collapsed,
            &self.open_task,
            &self.section_toggle,
            &self.workspace_action,
            &self.new_workspace,
            &self.task_action,
        );
        let sidebar_ms = if debug {
            Some(stage.elapsed().as_secs_f64() * 1000.0)
        } else {
            None
        };

        // ---- content pane ------------------------------------------------
        let sidebar = if app.sidebar_visible {
            spec::SIDEBAR_WIDTH
        } else {
            0
        };
        // From the *actual* window width, not the captured 1440: the artifact
        // panel keeps its fixed share and the conversation column absorbs the
        // slack, so a resized window cannot hand the panel 1000+ pixels.
        let width = self.window.size().0.max(spec::SIDEBAR_WIDTH + 360);
        let mut artifact = None;
        let mut shown = "welcome";
        let mut conversation_ms: Option<f64> = None;
        let mut detail_ms: Option<f64> = None;
        // Which task the message list should be showing, so arriving from the
        // welcome page or from another task re-pins the scroll to the bottom.
        let opened = match &app.route {
            Route::Task(index) => Some(*index),
            _ => None,
        };
        // Whether the conversation pane was on screen before this repaint.
        // `painted_task` only tracks which frames are attached, so coming back
        // from the landing page has to be spotted here: the frames are the
        // same ones, and the user still expects the newest message.
        let was_conversation =
            self.stack.visible_child_name().as_deref() == Some("conversation");
        match &app.route {
            Route::Unbuilt(name) => {
                self.unbuilt_title.set_text(name);
                clear_children(&self.unbuilt_info);
                let specialized = *name == "Experts · Skills · Connectors"
                    || *name == "Scheduled"
                    || *name == "Library";
                self.unbuilt_note.set_visible(!specialized);
                self.unbuilt_title.set_visible(!specialized);
                self.unbuilt_body.set_valign(if specialized {
                    gtk::Align::Start
                } else {
                    gtk::Align::Center
                });
                self.unbuilt_body.set_halign(if specialized {
                    gtk::Align::Fill
                } else {
                    gtk::Align::Center
                });
                self.unbuilt_body.set_margin_start(if specialized { 24 } else { 0 });
                self.unbuilt_body.set_margin_end(if specialized { 24 } else { 0 });
                self.unbuilt_body.set_margin_top(if specialized { 16 } else { 0 });
                if *name == "Experts · Skills · Connectors" {
                    fill_catalog(&self.unbuilt_info, &self.refresher, store);
                } else if *name == "Scheduled" {
                    fill_scheduled(&self.unbuilt_info, &self.refresher);
                } else if *name == "Library" {
                    fill_library(&self.unbuilt_info, &self.refresher, store);
                }
                self.unbuilt_info.show_all();
                shown = "unbuilt";
            }
            Route::Task(index) => match app.tasks.get(*index) {
                Some(task) => {
                    self.conversation.title.set_text(&task.title);
                    if !was_conversation || self.conversation.painted_task.get() != opened {
                        self.conversation.follow_now();
                    }
                    let paint_started = std::time::Instant::now();
                    conversation::fill_messages(&self.conversation, task, opened);
                    conversation_ms = Some(paint_started.elapsed().as_secs_f64() * 1000.0);
                    artifact = task.artifact.as_ref();
                    shown = "conversation";
                }
                // A stale index falls back to the landing page rather than
                // panicking on a refresh racing a deletion.
                None => {}
            },
            Route::Welcome => {}
        }

        // Which task's frames are attached is `fill_messages`' business now: a
        // switch moves them into the off-screen cache, and the landing page or
        // an unbuilt pane leaves them where they are.

        // GtkStack measures every *visible* child, so leaving the landing
        // column's 800px minimum on screen would stop the conversation pane
        // shrinking to its measured 580. Only the current route stays visible.
        self.welcome.root.set_visible(shown == "welcome");
        self.conversation.root.set_visible(shown == "conversation");
        self.unbuilt_root.set_visible(shown == "unbuilt");
        self.stack.set_visible_child_name(shown);
        if std::env::var_os("EB_DEBUG").is_some() {
            eprintln!(
                "refresh: stack={:?}",
                self.stack.visible_child_name().map(|name| name.to_string())
            );
        }

        // ---- artifact panel ----------------------------------------------
        let detail_started = std::time::Instant::now();
        let open = app.detail_visible && artifact.is_some();
        if open {
            // The tab strip shows every file the task has written; the
            // newest one arrives selected.
            let files = match &app.route {
                Route::Task(index) => app
                    .tasks
                    .get(*index)
                    .map(|task| task.artifacts.as_slice())
                    .unwrap_or_default(),
                _ => &[][..],
            };
            let files: Vec<crate::model::Artifact> = if files.is_empty() {
                artifact.into_iter().cloned().collect()
            } else {
                files.to_vec()
            };
            detail::fill(&self.detail, &files);
            // A card click named the tab it wants; `fill` above has already
            // settled the file list, so the pick lands on top of it.
            if let Some(index) = artifact_select {
                detail::select(&self.detail, index);
            }
        }
        self.detail.root.set_visible(open);
        // The toggle is always on the topbar (capture 1); only its tooltip
        // follows the panel state.
        self.conversation
            .artifact_button
            .set_tooltip_text(Some(if open {
                "Close right panel ⇧⌘B"
            } else {
                "Open right panel ⇧⌘B"
            }));

        // ---- panes --------------------------------------------------------
        self.sidebar.root.set_visible(app.sidebar_visible);
        for button in &self.reveals {
            button.set_visible(!app.sidebar_visible);
        }

        // The panel's own content minimum is what GtkPaned clamps the sash to.
        // Reading it here — after `detail::fill` rebuilt the tab strip — lets the
        // sash and the message margins agree: at a 1082px window the sash really
        // lands on 750, and centring the messages for 804 let the bubbles spill
        // past the divider into the panel.
        let panel_floor = if open { self.detail.root.preferred_width().0 } else { 0 };
        detail_ms = Some(detail_started.elapsed().as_secs_f64() * 1000.0);
        let sash = outer_sash(width, sidebar, open, app.detail_maximised, panel_floor);

        // Reposition when the *settled* layout changed: a sash the user
        // dragged survives the next repaint. A refresh that runs before the
        // window's real allocation (the `EB_ARTIFACT` seed does) reports the
        // default or a zero size; latching the key then would swallow the one
        // reposition that matters, because the configure-event repaint sees an
        // unchanged key. So an unallocated width positions but never latches.
        let settled = self.window.is_realized() && self.window.size().0 > 1;
        let layout = (app.sidebar_visible, open, app.detail_maximised, width, panel_floor);
        if settled && self.laid_out.get() != Some(layout) || !settled {
            self.inner.set_position(sidebar);
            self.outer.set_position(sash);
            if settled {
                self.laid_out.set(Some(layout));
            }
            if std::env::var_os("EB_DEBUG").is_some() {
                let (min_w, nat_w) = self.detail.root.preferred_width();
                let (cmin, cnat) = self.conversation.root.preferred_width();
                eprintln!(
                    "layout: width={width} open={open} maximised={} pos={} detail_min={min_w} detail_nat={nat_w} conv_min={cmin} conv_nat={cnat} inner={} side={} stack={}",
                    app.detail_maximised,
                    self.outer.position(),
                    self.inner.position(),
                    self.sidebar.root.allocation().width(),
                    self.stack.allocation().width(),
                );
            }
        }

        // The message column and the composer card centre themselves in the
        // pane the sash actually leaves: 500 wide with the panel open, 752
        // without it, and the 832/580 input area cap.
        //
        // The column is centred by its scroller — a content width plus `halign` —
        // because margins inside a viewport ratchet: see conversation.rs. The
        // composer is a direct child of the column, so margins are stable there.
        //
        // `max_content_width` only *truncates*, so a short transcript (the
        // artifact demo asks for 500 and draws 276) would leave the column at
        // its content's natural width. The width request on the column pins the
        // natural up to the design width. It costs nothing in shrinkability:
        // the scroller scrolls horizontally, so it advertises its own minimum
        // (`min_content_width`, 1) and never the child's.
        let pane_width = sash - sidebar;
        let message_width = if open {
            spec::MESSAGE_WIDTH
        } else {
            spec::MESSAGE_WIDTH_CLOSED
        }
        .min(pane_width);
        self.conversation.scroll.set_max_content_width(message_width);
        self.conversation.messages.set_size_request(message_width, -1);
        let input_width = if open {
            spec::CONVERSATION_WIDTH
        } else {
            spec::CHAT_INPUT_AREA_WIDTH_CLOSED
        };
        let input_margin = ((pane_width - input_width) / 2).max(0);
        self.chat_composer.root.set_margin_start(input_margin);
        self.chat_composer.root.set_margin_end(input_margin);
        if debug {
            eprintln!(
                "refresh: done in {:.1}ms (shown={shown} children={} sidebar={:.1} conv={:.1} detail={:.1})",
                started_at.elapsed().as_secs_f64() * 1000.0,
                self.conversation.messages.children().len(),
                sidebar_ms.unwrap_or_default(),
                conversation_ms.unwrap_or_default(),
                detail_ms.unwrap_or_default(),
            );
        }
    }
}

/// The outer sash for one layout state. The pane margins below are derived from
/// its return value, so the sash and the centring can never disagree.
///
/// The panel keeps its captured 580px share; the conversation column takes what
/// the window has left, but never so little that the 500px message bubbles
/// collapse (a 1100px window used to leave the message list 1px wide).
/// Below that the panel keeps its own content minimum so its header buttons
/// stay reachable (§10-31).
fn outer_sash(
    width: i32,
    sidebar: i32,
    open: bool,
    maximised: bool,
    panel_floor: i32,
) -> i32 {
    if !open {
        width
    } else if maximised {
        // Maximised, the artifact panel keeps what the sidebar does not use
        // and the conversation column is squeezed to nothing.
        sidebar
    } else {
        let conversation_floor = sidebar + spec::MESSAGE_WIDTH + 24;
        let sash = (width - spec::ARTIFACT_PANEL_WIDTH).max(conversation_floor);
        let panel_room =
            (width - panel_floor).max(sidebar + spec::CONVERSATION_MIN_WIDTH);
        sash.min(panel_room)
    }
}

/// Paint "this one is selected" onto a row of buttons. `None` clears every row,
/// which is what the capture shows while a task is open.
fn paint(buttons: &[gtk::Button], active: Option<usize>, class_name: &str) {
    for (index, button) in buttons.iter().enumerate() {
        let context = button.style_context();
        if Some(index) == active {
            context.add_class(class_name);
        } else {
            context.remove_class(class_name);
        }
    }
}

// ------------------------------------------------------------- action glue

/// Wrap a state mutation that takes no argument.
fn act0(store: &Store, refresher: &Refresher, change: impl Fn(&mut App) + 'static) -> Rc<dyn Fn()> {
    let store = store.clone();
    let refresher = refresher.clone();
    Rc::new(move || {
        change(&mut store.borrow_mut());
        refresher.request();
    })
}

/// Wrap a state mutation that takes one argument, so every handler repaints.
fn act1<A: 'static>(
    store: &Store,
    refresher: &Refresher,
    change: impl Fn(&mut App, A) + 'static,
) -> Rc<dyn Fn(A)> {
    let store = store.clone();
    let refresher = refresher.clone();
    Rc::new(move |argument: A| {
        change(&mut store.borrow_mut(), argument);
        refresher.request();
    })
}

fn act2<A: 'static, B: 'static>(
    store: &Store,
    refresher: &Refresher,
    change: impl Fn(&mut App, A, B) + 'static,
) -> Rc<dyn Fn(A, B)> {
    let store = store.clone();
    let refresher = refresher.clone();
    Rc::new(move |first: A, second: B| {
        change(&mut store.borrow_mut(), first, second);
        refresher.request();
    })
}

// ----------------------------------------------------------------- unbuilt

/// A placeholder for the nav rows whose screens are not built yet.
///
/// Showing a wrong-but-plausible screen would be worse than saying so.
fn unbuilt_pane(reveal: &Rc<dyn Fn()>) -> Unbuilt {
    let root = vbox(0);
    class(&root, "eb-content");

    let button = icon_button("▶", "eb-icon", "Show sidebar");
    hide_from_show_all(&button);
    button.connect_clicked({
        let reveal = reveal.clone();
        move |_| reveal()
    });

    let topbar = hbox(0);
    class(&topbar, "eb-content-topbar");
    topbar.pack_start(&button, false, false, 0);
    root.pack_start(&topbar, false, false, 0);

    let body = vbox(8);
    body.set_valign(gtk::Align::Center);
    body.set_halign(gtk::Align::Center);
    body.set_vexpand(true);

    let title = gtk::Label::new(Some(""));
    class(&title, "eb-unbuilt-title");
    body.pack_start(&title, false, false, 0);

    let note = gtk::Label::new(Some(
        "This screen is not built yet. EvoBuddy currently covers the landing page and \
         a task's conversation.",
    ));
    note.set_line_wrap(true);
    note.set_justify(gtk::Justification::Center);
    class(&note, "eb-unbuilt-note");
    body.pack_start(&note, false, false, 0);

    let info = vbox(4);
    info.set_halign(gtk::Align::Fill);
    info.set_hexpand(true);
    body.pack_start(&info, false, false, 0);

    root.pack_start(&body, true, true, 0);

    Unbuilt {
        root,
        title,
        note,
        info,
        body,
        reveal: button,
    }
}

/// Replace the active task's text turns with the retained dsh session's
/// projection once the session has a completed assistant message.
fn clear_children(container: &gtk::Box) {
    while let Some(child) = container.children().first() {
        container.remove(child);
    }
}

fn fill_catalog(container: &gtk::Box, refresher: &Refresher, store: &Store) {
    container.set_hexpand(true);
    let (tab, query, filter) = {
        let app = store.borrow();
        (app.catalog_tab, app.catalog_query.clone(), app.catalog_filter)
    };

    let header = hbox(10);
    for (index, label) in ["Experts", "Skills", "Connectors"].iter().enumerate() {
        let button = gtk::Button::with_label(label);
        class(&button, if index == tab { "eb-catalog-tab-active" } else { "eb-catalog-tab" });
        let store = store.clone();
        let refresher = refresher.clone();
        button.connect_clicked(move |_| {
            store.borrow_mut().catalog_tab = index;
            refresher.request();
        });
        header.pack_start(&button, false, false, 0);
    }
    let spacer = gtk::Label::new(None);
    spacer.set_hexpand(true);
    header.pack_start(&spacer, true, true, 0);
    let search = gtk::Entry::new();
    search.set_width_chars(20);
    search.set_placeholder_text(Some(match tab {
        0 => "Search experts",
        2 => "Search connectors",
        _ => "Search skills",
    }));
    search.set_text(&query);
    let store_for_search = store.clone();
    let refresher_for_search = refresher.clone();
    search.connect_activate(move |entry| {
        store_for_search.borrow_mut().catalog_query = entry.text().to_string();
        refresher_for_search.request();
    });
    if tab == 2 {
        let custom = gtk::Button::with_label("Custom connectors");
        class(&custom, "eb-add-skill");
        let store = store.clone();
        let refresher = refresher.clone();
        custom.connect_clicked(move |_| open_add_connector(&store, &refresher, ""));
        header.pack_end(&custom, false, false, 0);
    } else if tab == 0 {
        let mine = gtk::Button::with_label("My Experts");
        class(&mine, "eb-add-skill");
        let store = store.clone();
        let refresher = refresher.clone();
        mine.connect_clicked(move |_| {
            let active = crate::catalog::active_expert().unwrap_or("").to_string();
            store.borrow_mut().catalog_query = active;
            refresher.request();
        });
        header.pack_end(&mine, false, false, 0);
    } else {
        let add = gtk::Button::with_label("+ Add Skill");
        class(&add, "eb-add-skill");
        let store_for_add = store.clone();
        let refresher_for_add = refresher.clone();
        add.connect_clicked(move |_| open_add_skill(&store_for_add, &refresher_for_add));
        header.pack_end(&add, false, false, 0);
        let installed = gtk::Label::new(Some(&format!("Installed ({})", crate::catalog::skills().len())));
        class(&installed, "eb-catalog-count");
        header.pack_end(&installed, false, false, 0);
    }
    header.pack_end(&search, false, false, 0);
    container.pack_start(&header, false, false, 0);

    let divider = gtk::Separator::new(gtk::Orientation::Horizontal);
    divider.set_margin_top(10);
    divider.set_margin_bottom(12);
    container.pack_start(&divider, false, false, 0);

    match tab {
        0 => fill_experts(container, store, refresher, &query),
        2 => fill_connectors(container, store, refresher, &query),
        _ => fill_skills(container, store, refresher, &query, filter),
    }
}

fn fill_experts(container: &gtk::Box, store: &Store, refresher: &Refresher, query: &str) {
    let (category, teams, sort) = {
        let app = store.borrow();
        (app.expert_category, app.expert_teams, app.catalog_filter)
    };
    let categories = [
        "All",
        "Engineering",
        "Data & AI",
        "Product Design",
        "Content & Creative",
        "Sales & Business",
        "Operations",
        "Marketing & Growth",
    ];
    let section = hbox(10);
    let experts_tab = gtk::Button::with_label("Experts");
    class(&experts_tab, if teams { "eb-catalog-tab" } else { "eb-catalog-tab-active" });
    let teams_tab = gtk::Button::with_label("Expert Teams");
    class(&teams_tab, if teams { "eb-catalog-tab-active" } else { "eb-catalog-tab" });
    for (active, set) in [(false, false), (true, true)] {
        let button = if active { teams_tab.clone() } else { experts_tab.clone() };
        let store = store.clone();
        let refresher = refresher.clone();
        button.connect_clicked(move |_| {
            store.borrow_mut().expert_teams = set;
            refresher.request();
        });
    }
    section.pack_start(&experts_tab, false, false, 0);
    section.pack_start(&teams_tab, false, false, 0);
    let spacer = gtk::Label::new(None);
    spacer.set_hexpand(true);
    section.pack_start(&spacer, true, true, 0);
    let sort_box = hbox(6);
    for (index, label) in ["Recommended", "Hottest", "Newest"].iter().enumerate() {
        let button = gtk::Button::with_label(label);
        class(&button, if index == sort { "eb-chip-active" } else { "eb-chip" });
        let store = store.clone();
        let refresher = refresher.clone();
        button.connect_clicked(move |_| {
            store.borrow_mut().catalog_filter = index;
            refresher.request();
        });
        sort_box.pack_start(&button, false, false, 0);
    }
    section.pack_end(&sort_box, false, false, 0);
    container.pack_start(&section, false, false, 0);
    if teams {
        let empty = gtk::Label::new(Some("No expert teams yet. Teams will appear here."));
        empty.set_xalign(0.0);
        class(&empty, "eb-skill-meta");
        container.pack_start(&empty, false, false, 0);
        return;
    }

    let hero_row = gtk::Grid::new();
    hero_row.set_column_spacing(10);
    hero_row.set_row_spacing(10);
    hero_row.set_column_homogeneous(true);
    let heroes = [
        ("Content Creation", "content", "Content Creator"),
        ("Investment Analysis", "data", "Data Analytics Reporter"),
        ("Small Business", "business", "Project Manager"),
        ("E-commerce Operations", "ecommerce", "Frontend Developer"),
    ];
    for (index, (title, class_name, expert)) in heroes.iter().enumerate() {
        let card = vbox(4);
        class(&card, "eb-expert-hero");
        class(&card, class_name);
        let title = gtk::Label::new(Some(title));
        title.set_xalign(0.0);
        class(&title, "eb-expert-hero-title");
        card.pack_start(&title, false, false, 0);
        let name = gtk::Label::new(Some(*expert));
        name.set_xalign(0.0);
        class(&name, "eb-skill-meta");
        card.pack_start(&name, false, false, 0);
        let button = gtk::Button::with_label("Explore");
        button.set_halign(gtk::Align::Start);
        let target = match *class_name {
            "content" => 4,
            "data" => 2,
            "business" => 6,
            _ => 5,
        };
        let store = store.clone();
        let refresher = refresher.clone();
        button.connect_clicked(move |_| {
            store.borrow_mut().expert_category = target;
            store.borrow_mut().expert_teams = false;
            refresher.request();
        });
        card.pack_start(&button, false, false, 0);
        hero_row.attach(&card, index as i32, 0, 1, 1);
    }
    container.pack_start(&hero_row, false, false, 0);

    let chips = hbox(6);
    for (index, label) in categories.iter().enumerate() {
        let button = gtk::Button::with_label(label);
        class(&button, if index == category { "eb-chip-active" } else { "eb-chip" });
        let store = store.clone();
        let refresher = refresher.clone();
        button.connect_clicked(move |_| {
            store.borrow_mut().expert_category = index;
            refresher.request();
        });
        chips.pack_start(&button, false, false, 0);
    }
    container.pack_start(&chips, false, false, 0);

    let query = query.to_ascii_lowercase();
    let mut experts: Vec<_> = crate::catalog::expert_cards()
        .iter()
        .filter(|expert| {
            let matches_category = category == 0 || expert.category == categories[category];
            let haystack = format!("{} {} {}", expert.name, expert.description, expert.tags.join(" ")).to_ascii_lowercase();
            matches_category && (query.is_empty() || haystack.contains(&query))
        })
        .collect();
    if sort == 2 {
        experts.sort_by(|a, b| b.name.cmp(a.name));
    }
    let grid = gtk::Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(12);
    grid.set_column_homogeneous(true);
    let active = crate::catalog::active_expert();
    for (index, expert) in experts.into_iter().enumerate() {
        let card = vbox(4);
        class(&card, "eb-skill-card");
        let head = hbox(8);
        let avatar = gtk::Label::new(Some(expert.avatar));
        class(&avatar, "eb-skill-icon");
        head.pack_start(&avatar, false, false, 0);
        let name = gtk::Label::new(Some(expert.name));
        name.set_xalign(0.0);
        class(&name, "eb-skill-title");
        head.pack_start(&name, true, true, 0);
        let selected = active == Some(expert.name);
        let use_button = gtk::Button::with_label(if selected { "Clear" } else { "Use" });
        let store_for_use = store.clone();
        let refresher_for_use = refresher.clone();
        use_button.connect_clicked(move |_| {
            if selected {
                crate::catalog::set_expert(None);
            } else {
                crate::catalog::set_expert(Some(expert.name));
            }
            refresher_for_use.request();
        });
        head.pack_end(&use_button, false, false, 0);
        let details = gtk::Button::with_label("Details");
        let store_for_details = store.clone();
        let refresher_for_details = refresher.clone();
        let name_for_details = expert.name;
        details.connect_clicked(move |_| {
            open_expert_detail(name_for_details, &store_for_details, &refresher_for_details)
        });
        head.pack_end(&details, false, false, 0);
        card.pack_start(&head, false, false, 0);
        let description = gtk::Label::new(Some(expert.description));
        description.set_xalign(0.0);
        description.set_line_wrap(true);
        class(&description, "eb-skill-meta");
        card.pack_start(&description, false, false, 0);
        let tags = hbox(4);
        for tag in expert.tags {
            let label = gtk::Label::new(Some(tag));
            class(&label, "eb-expert-tag");
            tags.pack_start(&label, false, false, 0);
        }
        card.pack_start(&tags, false, false, 0);
        grid.attach(&card, (index % 3) as i32, (index / 3) as i32, 1, 1);
    }
    container.pack_start(&grid, false, false, 0);
}

fn open_expert_detail(name: &str, store: &Store, refresher: &Refresher) {
    let Some(expert) = crate::catalog::expert_cards().iter().find(|expert| expert.name == name) else {
        return;
    };
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title(expert.name);
    window.set_default_size(720, 640);
    window.set_modal(true);
    window.set_decorated(false);
    window.set_position(gtk::WindowPosition::Center);
    window.set_keep_above(true);
    let root = vbox(10);
    root.set_margin_top(20);
    root.set_margin_bottom(20);
    root.set_margin_start(24);
    root.set_margin_end(24);

    let header = hbox(14);
    let icon = gtk::Label::new(Some(expert.avatar));
    class(&icon, "eb-skill-detail-icon");
    header.pack_start(&icon, false, false, 0);
    let title_box = vbox(6);
    let title = gtk::Label::new(Some(&format!("{}  |  {}", expert.name, expert.role)));
    title.set_xalign(0.0);
    class(&title, "eb-skill-detail-title");
    title_box.pack_start(&title, false, false, 0);
    let selected = crate::catalog::active_expert() == Some(expert.name);
    let summon = gtk::Button::with_label(if selected { "Summoned" } else { "Summon" });
    summon.set_halign(gtk::Align::Start);
    class(&summon, "eb-add-skill");
    let summon_for_click = summon.clone();
    let store_for_summon = store.clone();
    let refresher_for_summon = refresher.clone();
    summon.connect_clicked(move |_| {
        if selected {
            crate::catalog::set_expert(None);
            summon_for_click.set_label("Summon");
        } else {
            crate::catalog::set_expert(Some(expert.name));
            summon_for_click.set_label("Summoned");
        }
        store_for_summon.borrow_mut().catalog_tab = 0;
        refresher_for_summon.request();
    });
    title_box.pack_start(&summon, false, false, 0);
    header.pack_start(&title_box, true, true, 0);
    let close = gtk::Button::with_label("×");
    class(&close, "eb-skill-detail-close");
    let window_for_close = window.clone();
    close.connect_clicked(move |_| window_for_close.close());
    header.pack_end(&close, false, false, 0);
    root.pack_start(&header, false, false, 0);

    let description = gtk::Label::new(Some(expert.description));
    description.set_xalign(0.0);
    description.set_line_wrap(true);
    class(&description, "eb-skill-detail-description");
    root.pack_start(&description, false, false, 0);
    let tags = hbox(6);
    for tag in expert.tags {
        let label = gtk::Label::new(Some(tag));
        class(&label, "eb-expert-tag");
        tags.pack_start(&label, false, false, 0);
    }
    root.pack_start(&tags, false, false, 0);
    root.pack_start(&gtk::Separator::new(gtk::Orientation::Horizontal), false, false, 0);

    let stats = hbox(12);
    for (value, label) in [
        (expert.version.to_string(), "Version"),
        ("No data".to_string(), "Usage"),
    ] {
        let column = vbox(2);
        column.set_hexpand(true);
        let value = gtk::Label::new(Some(&value));
        class(&value, "eb-skill-detail-stat");
        column.pack_start(&value, false, false, 0);
        let label = gtk::Label::new(Some(label));
        class(&label, "eb-skill-stats");
        column.pack_start(&label, false, false, 0);
        stats.pack_start(&column, true, true, 0);
    }
    root.pack_start(&stats, false, false, 0);
    root.pack_start(&gtk::Separator::new(gtk::Orientation::Horizontal), false, false, 0);

    let help = gtk::Label::new(Some("Expert can help with"));
    help.set_xalign(0.0);
    class(&help, "eb-catalog-section");
    root.pack_start(&help, false, false, 0);
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    class(&status, "eb-skill-stats");
    for example in expert.examples {
        let row = hbox(8);
        let label = gtk::Label::new(Some(example));
        label.set_xalign(0.0);
        label.set_line_wrap(true);
        label.set_hexpand(true);
        row.pack_start(&label, true, true, 0);
        let copy = gtk::Button::with_label("Copy");
        let text = (*example).to_string();
        let status_for_copy = status.clone();
        copy.connect_clicked(move |_| {
            let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
            clipboard.set_text(&text);
            clipboard.store();
            status_for_copy.set_text("Copied to clipboard");
        });
        row.pack_end(&copy, false, false, 0);
        root.pack_start(&row, false, false, 0);
    }
    root.pack_start(&status, false, false, 0);

    let more = gtk::Label::new(Some("More"));
    more.set_xalign(0.0);
    class(&more, "eb-catalog-section");
    root.pack_start(&more, false, false, 0);
    let id = gtk::Label::new(Some(&format!(
        "ID: {}",
        crate::catalog::expert_identity(expert)
    )));
    id.set_xalign(0.0);
    class(&id, "eb-skill-meta");
    root.pack_start(&id, false, false, 0);

    window.add(&root);
    window.show_all();
}

fn fill_skills(
    container: &gtk::Box,
    store: &Store,
    refresher: &Refresher,
    query: &str,
    filter: usize,
) {
    let query = query.to_ascii_lowercase();
    let skills: Vec<_> = crate::catalog::skills()
        .into_iter()
        .filter(|skill| {
            let text = format!("{} {} {}", skill.name, skill.description, skill.triggers.join(" "))
                .to_ascii_lowercase();
            let matches_query = query.is_empty() || text.contains(&query);
            let matches_filter = match filter {
                1 => skill.source == "builtin",
                2 => skill.source == "user",
                3 => skill.enabled,
                _ => true,
            };
            matches_query && matches_filter
        })
        .collect();
    let chips = hbox(6);
    for (index, label) in ["All", "Built-in", "User", "Enabled"].iter().enumerate() {
        let button = gtk::Button::with_label(label);
        class(&button, if index == filter { "eb-chip-active" } else { "eb-chip" });
        let store = store.clone();
        let refresher = refresher.clone();
        button.connect_clicked(move |_| {
            store.borrow_mut().catalog_filter = index;
            refresher.request();
        });
        chips.pack_start(&button, false, false, 0);
    }
    container.pack_start(&chips, false, false, 0);

    let featured: Vec<_> = skills.iter().filter(|skill| skill.source == "builtin").cloned().collect();
    let recommended: Vec<_> = skills.iter().filter(|skill| skill.source != "builtin").cloned().collect();
    if !featured.is_empty() {
        let label = gtk::Label::new(Some("Featured"));
        label.set_xalign(0.0);
        class(&label, "eb-catalog-section");
        container.pack_start(&label, false, false, 0);
        container.pack_start(&skill_grid(&featured, store, refresher), false, false, 0);
    }
    let label = gtk::Label::new(Some("Recommended"));
    label.set_xalign(0.0);
    class(&label, "eb-catalog-section");
    container.pack_start(&label, false, false, 0);
    if recommended.is_empty() {
        let empty = gtk::Label::new(Some(
            "No user skills yet. Use + Add Skill or create ~/.evobuddy/skills/<name>/SKILL.md.",
        ));
        empty.set_xalign(0.0);
        class(&empty, "eb-skill-meta");
        container.pack_start(&empty, false, false, 0);
    } else {
        container.pack_start(&skill_grid(&recommended, store, refresher), false, false, 0);
    }
}

fn skill_grid(skills: &[crate::catalog::Skill], store: &Store, refresher: &Refresher) -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(12);
    grid.set_column_homogeneous(true);
    for (index, skill) in skills.iter().enumerate() {
        grid.attach(&skill_card(skill, store, refresher), (index % 3) as i32, (index / 3) as i32, 1, 1);
    }
    grid
}

fn skill_card(skill: &crate::catalog::Skill, store: &Store, refresher: &Refresher) -> gtk::Box {
    let card = vbox(4);
    class(&card, "eb-skill-card");
    let badge = gtk::Label::new(Some(if skill.source == "builtin" { "BUILT-IN" } else { "USER" }));
    badge.set_xalign(0.0);
    class(&badge, "eb-skill-badge");
    card.pack_start(&badge, false, false, 0);
    let head = hbox(8);
    let icon = gtk::Label::new(Some(if skill.source == "builtin" { "◆" } else { "✦" }));
    class(&icon, "eb-skill-icon");
    head.pack_start(&icon, false, false, 0);
    let title = gtk::Label::new(Some(&skill.name));
    title.set_xalign(0.0);
    class(&title, "eb-skill-title");
    head.pack_start(&title, true, true, 0);
    let add = gtk::Button::with_label(if skill.enabled { "✓" } else { "+" });
    class(&add, "eb-skill-add");
    let name = skill.name.clone();
    let store_for_toggle = store.clone();
    let refresher_for_toggle = refresher.clone();
    add.connect_clicked(move |_| {
        crate::catalog::toggle(&name);
        store_for_toggle.borrow_mut().catalog_tab = 1;
        refresher_for_toggle.request();
    });
    head.pack_end(&add, false, false, 0);
    card.pack_start(&head, false, false, 0);
    let description = gtk::Label::new(Some(&skill.description));
    description.set_xalign(0.0);
    description.set_line_wrap(true);
    description.set_max_width_chars(44);
    class(&description, "eb-skill-meta");
    card.pack_start(&description, false, false, 0);
    let meta = format!(
        "{} · {} · {} trigger{}",
        skill.source,
        if skill.enabled { "installed" } else { "available" },
        skill.triggers.len(),
        if skill.triggers.len() == 1 { "" } else { "s" }
    );
    let meta = gtk::Label::new(Some(&meta));
    meta.set_xalign(0.0);
    class(&meta, "eb-skill-stats");
    card.pack_start(&meta, false, false, 0);
    if let Some(error) = &skill.error {
        let error = gtk::Label::new(Some(error));
        error.set_xalign(0.0);
        class(&error, "eb-skill-error");
        card.pack_start(&error, false, false, 0);
    }
    let view = gtk::Button::with_label("Details");
    view.set_halign(gtk::Align::Start);
    let skill_for_view = skill.clone();
    let store_for_view = store.clone();
    let refresher_for_view = refresher.clone();
    view.connect_clicked(move |_| {
        open_skill_detail(skill_for_view.clone(), &store_for_view, &refresher_for_view)
    });
    card.pack_start(&view, false, false, 0);
    card
}

fn fill_connectors(container: &gtk::Box, store: &Store, refresher: &Refresher, query: &str) {
    let entries = crate::catalog::configured_connectors();
    let last_test = store.borrow().connector_test.clone();
    let query = query.to_ascii_lowercase();
    let mut cards: Vec<(String, String, String)> = crate::catalog::connector_cards()
        .iter()
        .filter(|card| {
            query.is_empty()
                || card.name.to_ascii_lowercase().contains(&query)
                || card.description.to_ascii_lowercase().contains(&query)
        })
        .map(|card| (card.name.into(), card.description.into(), card.icon.into()))
        .collect();
    for entry in &entries {
        if !cards.iter().any(|(existing, _, _)| existing == &entry.name)
            && (query.is_empty() || entry.name.to_ascii_lowercase().contains(&query))
        {
            cards.push((
                entry.name.clone(),
                entry
                    .error
                    .clone()
                    .unwrap_or_else(|| format!("{} connector", entry.transport)),
                "⚙".into(),
            ));
        }
    }
    let intro = gtk::Label::new(Some(
        "Connect tools and data sources. Connectors are validated and passed to ACP as mcpServers.",
    ));
    intro.set_xalign(0.0);
    class(&intro, "eb-catalog-intro");
    container.pack_start(&intro, false, false, 0);
    let grid = gtk::Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(12);
    grid.set_column_homogeneous(true);
    for (index, (card_name, card_description, card_icon)) in cards.iter().enumerate() {
        let item = vbox(4);
        class(&item, "eb-skill-card");
        let head = hbox(8);
        let icon = gtk::Label::new(Some(card_icon));
        class(&icon, "eb-skill-icon");
        head.pack_start(&icon, false, false, 0);
        let name = gtk::Label::new(Some(card_name));
        name.set_xalign(0.0);
        class(&name, "eb-skill-title");
        head.pack_start(&name, true, true, 0);
        if let Some(entry) = entries.iter().find(|entry| &entry.name == card_name) {
            let status = if entry.error.is_some() { "Invalid" } else { "Configured" };
            let done = gtk::Label::new(Some(status));
            class(&done, if entry.error.is_some() { "eb-skill-error" } else { "eb-skill-source" });
            head.pack_end(&done, false, false, 0);
            let test = gtk::Button::with_label("Test");
            let store = store.clone();
            let refresher = refresher.clone();
            let name = card_name.clone();
            test.connect_clicked(move |_| {
                // A connector handshake can take up to ten seconds (stdio) or
                // four (http). Running it here would stop the window painting
                // for that whole time, so the probe runs on a worker thread and
                // reports back through this panel's short poll.
                store.borrow_mut().connector_test =
                    Some((name.clone(), "testing…".to_string()));
                refresher.request();
                let outcome: Arc<std::sync::Mutex<Option<(String, String)>>> =
                    Arc::new(std::sync::Mutex::new(None));
                {
                    let outcome = outcome.clone();
                    let probed = name.clone();
                    std::thread::Builder::new()
                        .name("evobuddy-connector-test".into())
                        .spawn(move || {
                            let result = crate::catalog::configured_connectors()
                                .into_iter()
                                .find(|entry| entry.name == probed)
                                .map(|entry| match entry.test() {
                                    Ok(message) => (probed.clone(), message),
                                    Err(message) => {
                                        (probed.clone(), format!("failed: {message}"))
                                    }
                                })
                                .unwrap_or_else(|| {
                                    (probed.clone(), "failed: connector is not configured".into())
                                });
                            *outcome.lock().unwrap() = Some(result);
                        })
                        .expect("spawn connector test");
                }
                let started = std::time::Instant::now();
                let poll_outcome = outcome.clone();
                let poll_store = store.clone();
                let poll_refresher = refresher.clone();
                let poll_name = name.clone();
                gtk::glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    let mut app = poll_store.borrow_mut();
                    if let Some(result) = poll_outcome.lock().unwrap().take() {
                        app.connector_test = Some(result);
                        drop(app);
                        poll_refresher.request();
                        return gtk::glib::ControlFlow::Break;
                    }
                    if started.elapsed() > std::time::Duration::from_secs(30) {
                        app.connector_test =
                            Some((poll_name.clone(), "failed: test is still running".to_string()));
                        drop(app);
                        poll_refresher.request();
                        return gtk::glib::ControlFlow::Break;
                    }
                    gtk::glib::ControlFlow::Continue
                });
            });
            head.pack_end(&test, false, false, 0);
        } else {
            let add = gtk::Button::with_label("+");
            class(&add, "eb-skill-add");
            let store = store.clone();
            let refresher = refresher.clone();
            let name = card_name.clone();
            add.connect_clicked(move |_| open_add_connector(&store, &refresher, &name));
            head.pack_end(&add, false, false, 0);
        }
        item.pack_start(&head, false, false, 0);
        let description = gtk::Label::new(Some(card_description));
        description.set_xalign(0.0);
        description.set_line_wrap(true);
        class(&description, "eb-skill-meta");
        item.pack_start(&description, false, false, 0);
        if let Some((name, message)) = &last_test {
            if name == card_name {
                let result = gtk::Label::new(Some(message));
                result.set_xalign(0.0);
                result.set_line_wrap(true);
                class(&result, if message.starts_with("failed") { "eb-skill-error" } else { "eb-skill-stats" });
                item.pack_start(&result, false, false, 0);
            }
        }
        grid.attach(&item, (index % 3) as i32, (index / 3) as i32, 1, 1);
    }
    container.pack_start(&grid, false, false, 0);
}

fn open_skill_view(title: &str, body: &str) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title(title);
    window.set_default_size(620, 480);
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_wrap_mode(gtk::WrapMode::Word);
    if let Some(buffer) = view.buffer() {
        buffer.set_text(body);
    }
    let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scroll.add(&view);
    window.add(&scroll);
    window.show_all();
}

fn open_skill_detail(skill: crate::catalog::Skill, store: &Store, refresher: &Refresher) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title(&skill.name);
    window.set_default_size(720, 640);
    window.set_modal(true);
    window.set_decorated(false);
    window.set_position(gtk::WindowPosition::Center);
    window.set_keep_above(true);
    let root = vbox(10);
    root.set_margin_top(20);
    root.set_margin_bottom(20);
    root.set_margin_start(24);
    root.set_margin_end(24);

    let header = hbox(14);
    let icon = gtk::Label::new(Some("◆"));
    class(&icon, "eb-skill-detail-icon");
    header.pack_start(&icon, false, false, 0);
    let title_box = vbox(6);
    let title = gtk::Label::new(Some(&skill.name));
    title.set_xalign(0.0);
    class(&title, "eb-skill-detail-title");
    title_box.pack_start(&title, false, false, 0);
    let install = gtk::Button::with_label(if skill.enabled { "Installed" } else { "Install" });
    install.set_halign(gtk::Align::Start);
    class(&install, "eb-add-skill");
    let store_for_install = store.clone();
    let refresher_for_install = refresher.clone();
    let name = skill.name.clone();
    let install_for_click = install.clone();
    let was_enabled = skill.enabled;
    install.connect_clicked(move |_| {
        crate::catalog::toggle(&name);
        install_for_click.set_label(if was_enabled { "Install" } else { "Installed" });
        store_for_install.borrow_mut().catalog_tab = 1;
        refresher_for_install.request();
    });
    title_box.pack_start(&install, false, false, 0);
    header.pack_start(&title_box, true, true, 0);
    let close = gtk::Button::with_label("×");
    class(&close, "eb-skill-detail-close");
    let window_for_close = window.clone();
    close.connect_clicked(move |_| window_for_close.close());
    header.pack_end(&close, false, false, 0);
    root.pack_start(&header, false, false, 0);

    let description = gtk::Label::new(Some(&skill.description));
    description.set_xalign(0.0);
    description.set_line_wrap(true);
    class(&description, "eb-skill-detail-description");
    root.pack_start(&description, false, false, 0);
    root.pack_start(&gtk::Separator::new(gtk::Orientation::Horizontal), false, false, 0);

    let stats = hbox(12);
    for (value, label) in [
        (skill.version.clone(), "Version"),
        ("No data".to_string(), "Usage"),
        ("No data".to_string(), "Installs"),
    ] {
        let column = vbox(2);
        column.set_hexpand(true);
        let value = gtk::Label::new(Some(&value));
        class(&value, "eb-skill-detail-stat");
        column.pack_start(&value, false, false, 0);
        let label = gtk::Label::new(Some(label));
        class(&label, "eb-skill-stats");
        column.pack_start(&label, false, false, 0);
        stats.pack_start(&column, true, true, 0);
    }
    root.pack_start(&stats, false, false, 0);
    root.pack_start(&gtk::Separator::new(gtk::Orientation::Horizontal), false, false, 0);

    let try_header = gtk::Label::new(Some("Try it like this"));
    try_header.set_xalign(0.0);
    class(&try_header, "eb-catalog-section");
    root.pack_start(&try_header, false, false, 0);
    let examples = if skill.examples.is_empty() {
        vec![
            format!("Use {} on the current task", skill.name),
            format!("Review this change with {}", skill.name),
        ]
    } else {
        skill.examples.clone()
    };
    let status = gtk::Label::new(None);
    status.set_xalign(0.0);
    class(&status, "eb-skill-stats");
    for example in examples {
        let row = hbox(8);
        let label = gtk::Label::new(Some(&example));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_line_wrap(true);
        row.pack_start(&label, true, true, 0);
        let copy = gtk::Button::with_label("Copy");
        let example_for_copy = example.clone();
        let status_for_copy = status.clone();
        copy.connect_clicked(move |_| {
            let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
            clipboard.set_text(&example_for_copy);
            clipboard.store();
            status_for_copy.set_text("Copied to clipboard");
        });
        row.pack_end(&copy, false, false, 0);
        root.pack_start(&row, false, false, 0);
    }
    root.pack_start(&status, false, false, 0);

    let more = gtk::Label::new(Some("More"));
    more.set_xalign(0.0);
    class(&more, "eb-catalog-section");
    root.pack_start(&more, false, false, 0);
    let id = gtk::Label::new(Some(&format!("ID: {}", skill.id)));
    id.set_xalign(0.0);
    class(&id, "eb-skill-meta");
    root.pack_start(&id, false, false, 0);
    let instructions = gtk::Button::with_label("View instructions");
    let body = skill.body.clone();
    let title_text = skill.name.clone();
    instructions.connect_clicked(move |_| open_skill_view(&title_text, &body));
    root.pack_start(&instructions, false, false, 0);

    window.add(&root);
    window.show_all();
}

fn open_add_connector(store: &Store, refresher: &Refresher, preset: &str) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Add Connector");
    window.set_default_size(480, 280);
    window.set_modal(true);
    let root = vbox(8);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Connector name"));
    name.set_text(preset);
    root.pack_start(&name, false, false, 0);
    let command = gtk::Entry::new();
    command.set_placeholder_text(Some("Command, e.g. npx"));
    root.pack_start(&command, false, false, 0);
    let args = gtk::Entry::new();
    args.set_placeholder_text(Some("Arguments, space separated"));
    root.pack_start(&args, false, false, 0);
    let error = gtk::Label::new(None);
    error.set_xalign(0.0);
    class(&error, "eb-skill-error");
    root.pack_start(&error, false, false, 0);
    let actions = hbox(8);
    let cancel = gtk::Button::with_label("Cancel");
    let create = gtk::Button::with_label("Add");
    actions.pack_end(&create, false, false, 0);
    actions.pack_end(&cancel, false, false, 0);
    root.pack_start(&actions, false, false, 0);
    window.add(&root);
    let window_for_cancel = window.clone();
    cancel.connect_clicked(move |_| window_for_cancel.close());
    let window_for_create = window.clone();
    let store = store.clone();
    let refresher = refresher.clone();
    create.connect_clicked(move |_| {
        match crate::catalog::create_connector(
            &name.text().to_string(),
            &command.text().to_string(),
            &args.text().to_string(),
        ) {
            Ok(_) => {
                store.borrow_mut().catalog_tab = 2;
                refresher.request();
                window_for_create.close();
            }
            Err(message) => error.set_text(&message),
        }
    });
    window.show_all();
}

fn open_add_skill(store: &Store, refresher: &Refresher) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Add Skill");
    window.set_default_size(460, 220);
    window.set_modal(true);
    let root = vbox(8);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Skill name"));
    root.pack_start(&name, false, false, 0);
    let description = gtk::Entry::new();
    description.set_placeholder_text(Some("Description"));
    root.pack_start(&description, false, false, 0);
    let actions = hbox(8);
    let cancel = gtk::Button::with_label("Cancel");
    let create = gtk::Button::with_label("Create");
    actions.pack_end(&create, false, false, 0);
    actions.pack_end(&cancel, false, false, 0);
    root.pack_start(&actions, false, false, 0);
    window.add(&root);
    let window_for_cancel = window.clone();
    cancel.connect_clicked(move |_| window_for_cancel.close());
    let window_for_create = window.clone();
    let store = store.clone();
    let refresher = refresher.clone();
    create.connect_clicked(move |_| {
        let name = name.text().to_string();
        let description = description.text().to_string();
        if crate::catalog::create_skill(&name, &description).is_ok() {
            store.borrow_mut().catalog_tab = 1;
            refresher.request();
            window_for_create.close();
        }
    });
    window.show_all();
}

fn fill_scheduled(container: &gtk::Box, refresher: &Refresher) {
    let header = hbox(8);
    let title = gtk::Label::new(Some("Scheduled Tasks"));
    title.set_xalign(0.0);
    class(&title, "eb-page-title");
    header.pack_start(&title, false, false, 0);
    let spacer = gtk::Label::new(None);
    spacer.set_hexpand(true);
    header.pack_start(&spacer, true, true, 0);
    let new = gtk::Button::with_label("+ New");
    class(&new, "eb-add-skill");
    let refresher_for_new = refresher.clone();
    new.connect_clicked(move |_| open_add_scheduled(&refresher_for_new));
    header.pack_end(&new, false, false, 0);
    container.pack_start(&header, false, false, 0);

    let jobs = crate::catalog::scheduled();
    if jobs.is_empty() {
        let empty = vbox(4);
        empty.set_valign(gtk::Align::Center);
        let icon = gtk::Label::new(Some("⏱"));
        class(&icon, "eb-empty-icon");
        empty.pack_start(&icon, false, false, 0);
        let text = gtk::Label::new(Some("No scheduled tasks found."));
        class(&text, "eb-skill-meta");
        empty.pack_start(&text, false, false, 0);
        container.pack_start(&empty, false, false, 0);
    } else {
        for (index, (title, prompt, due)) in jobs.into_iter().enumerate() {
            let card = vbox(3);
            class(&card, "eb-skill-card");
            let head = hbox(8);
            let name = gtk::Label::new(Some(&title));
            name.set_xalign(0.0);
            class(&name, "eb-skill-title");
            head.pack_start(&name, false, false, 0);
            let when = gtk::Label::new(Some(&due_label(&due)));
            class(&when, "eb-skill-source");
            head.pack_start(&when, false, false, 0);
            let run = gtk::Button::with_label("Run now");
            let refresher_run = refresher.clone();
            run.connect_clicked(move |_| {
                crate::catalog::run_scheduled(index);
                refresher_run.request();
            });
            head.pack_end(&run, false, false, 0);
            let delete = gtk::Button::with_label("Delete");
            let refresher_delete = refresher.clone();
            delete.connect_clicked(move |_| {
                crate::catalog::remove_scheduled(index);
                refresher_delete.request();
            });
            head.pack_end(&delete, false, false, 0);
            card.pack_start(&head, false, false, 0);
            let body = gtk::Label::new(Some(&prompt));
            body.set_xalign(0.0);
            body.set_line_wrap(true);
            class(&body, "eb-skill-meta");
            card.pack_start(&body, false, false, 0);
            container.pack_start(&card, false, false, 0);
        }
    }

    let popular = gtk::Label::new(Some("Try these popular tasks"));
    popular.set_xalign(0.0);
    class(&popular, "eb-catalog-section");
    container.pack_start(&popular, false, false, 0);
    let grid = gtk::Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(12);
    grid.set_column_homogeneous(true);
    let templates = [
        ("Daily AI News Briefing", "Track key AI updates with a short daily brief."),
        ("Daily 5 English Words", "Recommend five practical words each day."),
        ("Weekly Work Report", "Review weekly PR and issue activity."),
        ("On This Day", "Pick one interesting historical event."),
        ("Daily Why", "Ask one fun why-question, then explain."),
        ("Pre-meeting Preparation", "Prepare a short agenda before a meeting."),
        ("Health Checkup Reminder", "Remind me to take a break and stretch."),
        ("Daily Bedtime Story", "Write a gentle 3-minute bedtime story."),
        ("Classic Movie Pick", "Recommend one highly acclaimed classic movie."),
    ];
    for (index, (name, description)) in templates.iter().enumerate() {
        let name = (*name).to_string();
        let description = (*description).to_string();
        let card = vbox(3);
        class(&card, "eb-skill-card");
        let title = gtk::Label::new(Some(&name));
        title.set_xalign(0.0);
        class(&title, "eb-skill-title");
        card.pack_start(&title, false, false, 0);
        let body = gtk::Label::new(Some(&description));
        body.set_xalign(0.0);
        body.set_line_wrap(true);
        class(&body, "eb-skill-meta");
        card.pack_start(&body, false, false, 0);
        let add = gtk::Button::with_label("Schedule +1m");
        add.set_halign(gtk::Align::Start);
        let refresher = refresher.clone();
        add.connect_clicked(move |_| {
            crate::catalog::add_scheduled(&name, &description, 60);
            refresher.request();
        });
        card.pack_start(&add, false, false, 0);
        grid.attach(&card, (index % 3) as i32, (index / 3) as i32, 1, 1);
    }

    container.pack_start(&grid, false, false, 0);
}

fn open_add_scheduled(refresher: &Refresher) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("New Scheduled Task");
    window.set_default_size(480, 220);
    window.set_modal(true);
    let root = vbox(8);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);
    let prompt = gtk::Entry::new();
    prompt.set_placeholder_text(Some("Prompt to run in one minute"));
    root.pack_start(&prompt, false, false, 0);
    let actions = hbox(8);
    let cancel = gtk::Button::with_label("Cancel");
    let create = gtk::Button::with_label("Schedule +1m");
    actions.pack_end(&create, false, false, 0);
    actions.pack_end(&cancel, false, false, 0);
    root.pack_start(&actions, false, false, 0);
    window.add(&root);
    let window_for_cancel = window.clone();
    cancel.connect_clicked(move |_| window_for_cancel.close());
    let window_for_create = window.clone();
    let refresher = refresher.clone();
    create.connect_clicked(move |_| {
        let text = prompt.text().to_string();
        if !text.trim().is_empty() {
            crate::catalog::add_scheduled(&text, &text, 60);
            refresher.request();
            window_for_create.close();
        }
    });
    window.show_all();
}

fn due_label(due: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|time| time.as_secs())
        .unwrap_or(0);
    match due.parse::<u64>() {
        Ok(at) if at <= now => "due now".into(),
        Ok(at) => format!("in {}s", at - now),
        Err(_) => due.to_string(),
    }
}

fn fill_library(container: &gtk::Box, refresher: &Refresher, store: &Store) {
    let (section, query) = {
        let app = store.borrow();
        (app.library_section, app.library_query.clone())
    };
    let row = hbox(16);
    let sidebar = vbox(3);
    sidebar.set_size_request(210, -1);
    let search = gtk::Entry::new();
    search.set_placeholder_text(Some("Search library"));
    search.set_text(&query);
    let store_for_search = store.clone();
    let refresher_for_search = refresher.clone();
    search.connect_activate(move |entry| {
        store_for_search.borrow_mut().library_query = entry.text().to_string();
        refresher_for_search.request();
    });
    sidebar.pack_start(&search, false, false, 0);
    for (index, label) in ["Recent", "Local artifacts", "My Materials"].iter().enumerate() {
        let button = gtk::Button::with_label(label);
        class(&button, if index == section { "eb-library-nav-active" } else { "eb-library-nav" });
        let store = store.clone();
        let refresher = refresher.clone();
        button.connect_clicked(move |_| {
            store.borrow_mut().library_section = index;
            refresher.request();
        });
        sidebar.pack_start(&button, false, false, 0);
    }
    row.pack_start(&sidebar, false, false, 0);

    let main = vbox(6);
    let title = gtk::Label::new(Some(match section {
        1 => "Local artifacts",
        2 => "My Materials",
        _ => "Recent",
    }));
    title.set_xalign(0.0);
    class(&title, "eb-page-title");
    main.pack_start(&title, false, false, 0);
    let tabs = hbox(8);
    for label in ["Recent visit", "Shared by me", "Shared with me"] {
        let label = gtk::Label::new(Some(label));
        class(&label, "eb-skill-source");
        tabs.pack_start(&label, false, false, 0);
    }
    main.pack_start(&tabs, false, false, 0);
    let header = hbox(0);
    for (label, width) in [("Name", 360), ("Owner", 180), ("Location", 180), ("Recent visit", 140)] {
        let label = gtk::Label::new(Some(label));
        label.set_xalign(0.0);
        label.set_size_request(width, -1);
        class(&label, "eb-skill-stats");
        header.pack_start(&label, false, false, 0);
    }
    main.pack_start(&header, false, false, 0);
    main.pack_start(&gtk::Separator::new(gtk::Orientation::Horizontal), false, false, 0);

    let tasks = store.borrow();
    let mut added = 0;
    for task in &tasks.tasks {
        let workspace = task.workspace_id.as_deref().and_then(|id| {
            tasks.workspaces.iter().find(|workspace| workspace.id == id)
        });
        let (name, location) = if let Some(artifact) = &task.artifact {
            (artifact.name.clone(), "Local artifacts")
        } else if let Some(workspace) = workspace {
            (
                crate::workspace::display_name(workspace),
                "My Materials",
            )
        } else {
            continue;
        };
        if section == 1 && location != "Local artifacts" {
            continue;
        }
        if section == 2 && location != "My Materials" {
            continue;
        }
        let needle = query.to_ascii_lowercase();
        if !needle.is_empty()
            && !name.to_ascii_lowercase().contains(&needle)
            && !location.to_ascii_lowercase().contains(&needle)
        {
            continue;
        }
        let line = hbox(0);
        for (value, width) in [
            (name, 360),
            ("you@evobuddy.local".to_string(), 180),
            (location.to_string(), 180),
            ("just now".to_string(), 140),
        ] {
            let label = gtk::Label::new(Some(&value));
            label.set_xalign(0.0);
            label.set_size_request(width, -1);
            class(&label, "eb-skill-meta");
            line.pack_start(&label, false, false, 0);
        }
        main.pack_start(&line, false, false, 0);
        main.pack_start(&gtk::Separator::new(gtk::Orientation::Horizontal), false, false, 0);
        added += 1;
    }
    if added == 0 {
        let empty = gtk::Label::new(Some("Nothing here yet."));
        empty.set_xalign(0.0);
        class(&empty, "eb-skill-meta");
        main.pack_start(&empty, false, false, 0);
    }
    row.pack_start(&main, true, true, 0);
    container.pack_start(&row, false, false, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 1440 baseline is a captured number (ui-spec §4.4) and must not move;
    /// the narrow-window floors are the invented rule recorded in §10-31.
    /// `panel_floor` is the panel's live content minimum, 332 with the two-file
    /// tab strip the `EB_ARTIFACT` seed produces.
    #[test]
    fn the_outer_sash_holds_the_captured_share_then_the_two_floors() {
        let sidebar = spec::SIDEBAR_WIDTH;
        let panel = 332;
        let sash = |width: i32| outer_sash(width, sidebar, true, false, panel);
        assert_eq!(sash(1920), 1340, "the panel keeps 580 while there is room");
        assert_eq!(sash(1440), 860, "captured baseline: sash 860, panel 580");
        assert_eq!(sash(1136), 804, "the conversation keeps its 500+24 floor");
        assert_eq!(sash(1100), 768, "then the panel keeps its content minimum");
        assert_eq!(sash(1082), 750, "the number GtkPaned was clamping to anyway");
        assert_eq!(sash(900), 568);
        assert_eq!(
            sash(800),
            sidebar + spec::CONVERSATION_MIN_WIDTH,
            "the conversation floor is the one that wins last"
        );
        assert_eq!(outer_sash(1440, sidebar, false, false, panel), 1440);
        assert_eq!(outer_sash(1440, sidebar, true, true, panel), sidebar);
        for width in (640..=1920).step_by(20) {
            let sash = sash(width);
            assert!(
                (sidebar..=width).contains(&sash),
                "width {width} put the sash at {sash}"
            );
        }
    }

    /// Local noon, `days` away from 2026-10-28, as a Unix second. Built through
    /// GLib so the ladder is tested against the machine's own calendar, which is
    /// the thing `Yesterday` depends on.
    fn local_noon(days: i32) -> u64 {
        let base = gtk::glib::DateTime::from_local(2026, 10, 28, 12, 0, 0.0)
            .expect("local noon on 2026-10-28");
        let moved = base.add_days(days).expect("shifting by whole days");
        moved.to_unix().max(0) as u64
    }

    #[test]
    fn the_sidebar_clock_walks_the_product_ladder() {
        let now = local_noon(0);
        let at = |secs: u64| sidebar_time(now - secs, now);
        assert_eq!(at(0), "Just now");
        assert_eq!(at(59), "Just now");
        assert_eq!(at(60), "1m");
        assert_eq!(at(3_599), "59m");
        assert_eq!(at(3_600), "1h");
        assert_eq!(at(86_399), "23h");
        // One calendar day back, whatever the clock says.
        assert_eq!(sidebar_time(local_noon(-1), now), "Yesterday");
        assert_eq!(sidebar_time(local_noon(-2), now), "2d");
        assert_eq!(sidebar_time(local_noon(-6), now), "6d");
        // Past a week the row shows a date instead of a count.
        assert_eq!(sidebar_time(local_noon(-8), now), "Oct 20");
        assert_eq!(sidebar_time(local_noon(-365), now), "25/10/28");
    }

    #[test]
    fn the_day_numbers_span_midnights_leaps_and_year_ends() {
        let day = |y, m, d| days_from_civil(y, m, d);
        assert_eq!(day(1970, 1, 1), 0, "the epoch is day zero");
        assert_eq!(day(2026, 10, 28) - day(2026, 10, 21), 7);
        assert_eq!(day(2024, 3, 1) - day(2024, 2, 28), 2, "2024 is a leap year");
        assert_eq!(day(2023, 3, 1) - day(2023, 2, 28), 1);
        assert_eq!(day(2026, 1, 1) - day(2025, 12, 31), 1, "no gap at a year end");
    }
}
