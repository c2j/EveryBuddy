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
/// Handlers therefore capture a `Refresher` and call `request()`; the real
/// closure is installed in one step once the tree exists.
#[derive(Clone)]
pub struct Refresher {
    deferred: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
    immediate: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
}

impl Refresher {
    pub fn new() -> Refresher {
        Refresher {
            deferred: Rc::new(RefCell::new(None)),
            immediate: Rc::new(RefCell::new(None)),
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
        if let Some(refresh) = self.immediate.borrow().as_ref() {
            refresh();
        } else {
            self.request();
        }
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
    pub welcome_composer: Rc<Composer>,
    pub chat_composer: Rc<Composer>,
    /// Every "show sidebar" button: one per route's topbar, since only the
    /// visible route's button can be clicked.
    pub reveals: Vec<gtk::Button>,
    /// Owns the in-flight agent turn and its event drain loop.
    pub agent: Rc<Runner>,
    refresher: Refresher,
    open_task: Rc<dyn Fn(usize)>,
    /// A sidebar section header was clicked.
    section_toggle: Rc<dyn Fn(usize)>,
    /// The last `(sidebar_visible, artifact_open, artifact_maximised)` triple
    /// that positioned the panes. Tracking it keeps a repaint from undoing a
    /// sash the user dragged.
    laid_out: Cell<Option<(bool, bool, bool)>>,
}

/// A pane for a nav row whose screen has not been built.
struct Unbuilt {
    root: gtk::Box,
    title: gtk::Label,
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
    window.set_default_size(spec::WINDOW_WIDTH, spec::WINDOW_HEIGHT);
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
    let open_task = act1(store, &refresher, |app, index: usize| app.open_task(index));
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
    let pick = act2(store, &refresher, |app, picker: Picker, index: usize| {
        match picker {
            Picker::Model => app.model = index,
            Picker::Workspace => app.workspace = index,
            Picker::Permission => app.permission = index,
            // The add-context menu performs an action; there is nothing to
            // remember between openings.
            Picker::Add => {}
        }
    });
    let collapse = act0(store, &refresher, |app| app.sidebar_visible = false);
    let reveal = act0(store, &refresher, |app| app.sidebar_visible = true);
    let maximise = act0(store, &refresher, |app| {
        app.detail_maximised = !app.detail_maximised
    });
    let hide_detail = act0(store, &refresher, |app| app.detail_visible = false);
    let filter = act1(store, &refresher, |_app, _: usize| {});
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
    let composer_wiring = composer::Wiring {
        submit: submit.clone(),
        cancel,
        pick: pick.clone(),
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
    let sidebar = sidebar::build(
        &sidebar::Wiring {
            nav: nav.clone(),
            collapse: collapse.clone(),
            filter,
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

    let conversation = conversation::build(
        &conversation::Wiring {
            reveal: reveal.clone(),
        },
        &chat_composer,
    );
    note(&mut probes, "eb-chat-topbar", &conversation.topbar);
    note(&mut probes, "eb-chat-actions", &conversation.actions);
    note(&mut probes, "eb-messages", &conversation.messages);
    note(&mut probes, "eb-input-area", &chat_composer.root);
    note(&mut probes, "eb-chat-card", &chat_composer.card);

    let detail = detail::build(
        &detail::Wiring {
            close: hide_detail.clone(),
            expand: maximise,
        },
        contribs,
    );
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
    class(&outer, "eb-split");
    outer.pack1(&inner, true, false);
    outer.pack2(&detail.root, false, false);

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
        welcome_composer,
        chat_composer,
        reveals,
        agent,
        refresher,
        open_task,
        section_toggle: section,
        laid_out: Cell::new(None),
    };

    (views, probes)
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
        if std::env::var_os("EB_DEBUG").is_some() {
            eprintln!("refresh: enter {:?}", std::thread::current().id());
        }
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

        let usage = match &app.route {
            Route::Task(index) => app.tasks.get(*index).and_then(|task| task.usage),
            _ => None,
        };
        self.welcome_composer.set_usage(None);
        self.chat_composer.set_usage(usage);

        // ---- sidebar list ------------------------------------------------
        let rows: Vec<TaskRow> = app
            .tasks
            .iter()
            .enumerate()
            .rev()
            .map(|(index, task)| TaskRow {
                title: task.title.clone(),
                meta: task.meta.clone(),
                active: matches!(app.route, Route::Task(active) if active == index),
                index,
            })
            .collect();
        sidebar::fill_content(
            &self.sidebar.content,
            &rows,
            app.collapsed,
            &self.open_task,
            &self.section_toggle,
        );

        // ---- content pane ------------------------------------------------
        // The product caps the message column at 752 with the artifact panel
        // closed and 500 with it open; the pane is 1160 / 580 respectively.
        // Set the margins before filling messages so the first layout pass
        // already sees the right width.
        let artifact_open = match &app.route {
            Route::Task(index) => {
                app.detail_visible
                    && app
                        .tasks
                        .get(*index)
                        .and_then(|task| task.artifact.as_ref())
                        .is_some()
            }
            _ => false,
        };
        let sidebar = if app.sidebar_visible {
            spec::SIDEBAR_WIDTH
        } else {
            0
        };
        let pane_width = spec::WINDOW_WIDTH
            - sidebar
            - if artifact_open {
                spec::CONVERSATION_WIDTH
            } else {
                0
            };
        let message_width = if artifact_open {
            spec::MESSAGE_WIDTH
        } else {
            spec::MESSAGE_WIDTH_CLOSED
        };
        let margin = ((pane_width - message_width) / 2).max(0);
        self.conversation.messages.set_margin_start(margin);
        self.conversation.messages.set_margin_end(margin);

        // The input area is capped too: 832 with the panel closed (a 784 card),
        // and the full 580 pane with it open.
        let input_width = if artifact_open {
            spec::CONVERSATION_WIDTH
        } else {
            spec::CHAT_INPUT_AREA_WIDTH_CLOSED
        };
        let input_margin = ((pane_width - input_width) / 2).max(0);
        self.chat_composer.root.set_margin_start(input_margin);
        self.chat_composer.root.set_margin_end(input_margin);

        let mut artifact = None;
        let mut shown = "welcome";
        match &app.route {
            Route::Unbuilt(name) => {
                self.unbuilt_title.set_text(name);
                shown = "unbuilt";
            }
            Route::Task(index) => match app.tasks.get(*index) {
                Some(task) => {
                    self.conversation.title.set_text(&task.title);
                    conversation::fill_messages(&self.conversation, task);
                    artifact = task.artifact.as_ref();
                    shown = "conversation";
                }
                // A stale index falls back to the landing page rather than
                // panicking on a refresh racing a deletion.
                None => {}
            },
            Route::Welcome => {}
        }

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
        let open = app.detail_visible && artifact.is_some();
        if let Some(artifact) = artifact {
            if open {
                detail::fill(&self.detail, artifact);
            }
        }
        self.detail.root.set_visible(open);

        // ---- panes --------------------------------------------------------
        self.sidebar.root.set_visible(app.sidebar_visible);
        for button in &self.reveals {
            button.set_visible(!app.sidebar_visible);
        }

        // Only reposition when the *layout* changed, so a sash the user dragged
        // survives the next repaint.
        let layout = (app.sidebar_visible, open, app.detail_maximised);
        if self.laid_out.get() != Some(layout) {
            let sidebar = if app.sidebar_visible {
                spec::SIDEBAR_WIDTH
            } else {
                0
            };
            self.inner.set_position(sidebar);
            self.outer.set_position(if open {
                // Maximised, the artifact panel keeps what the sidebar does not
                // use and the conversation column is squeezed to nothing.
                let conversation = if app.detail_maximised {
                    0
                } else {
                    spec::CONVERSATION_WIDTH
                };
                sidebar + conversation
            } else {
                spec::WINDOW_WIDTH
            });
            self.laid_out.set(Some(layout));
        }
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

    root.pack_start(&body, true, true, 0);

    Unbuilt {
        root,
        title,
        reveal: button,
    }
}
