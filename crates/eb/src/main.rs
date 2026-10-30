// On Windows, build as a GUI (rather than console) application so launching the
// binary does not open an extra console window. Ignored on other platforms.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

//! EvoBuddy — the desktop client's main workspace.
//!
//! Rebuilt against a live CDP capture of the product rather than guessed from
//! class names. The capture, the derived numbers and the list of known
//! deviations live in `docs/ui-spec/ui-spec.md`.
//!
//! Split of responsibility:
//!
//! | module                | holds                                              |
//! |-----------------------|----------------------------------------------------|
//! | `spec`                | every measured number and the shell's chrome copy  |
//! | `ui/gtk.css`          | colours, type, radii, shadows (+ generated tokens) |
//! | the view modules      | structure, in `GtkBox` spacing and margins         |
//! | `model`               | re-exports `eb-api`'s application state            |
//! | `host`                | the rutis mount: plugins in, contributions out     |
//! | `shell`               | assembly, signal wiring, state -> widget refresh   |
//! | `selftest`            | the `EB_SELFTEST` smoke test and the PNG renderer  |
//!
//! The content itself — nav rows, dropdown menus, notices, scenario chips, the
//! placeholder answers — is contributed by the plugin crates under
//! `crates/plugins/`, against the surface in `crates/eb-api`. The crate layout
//! enforces the direction: plugins never depend on the host.
//!
//! Geometry lives in Rust rather than CSS because GTK CSS has no flexbox and no
//! `gap`; colours live in CSS because they are what the token pipeline produces.

mod agent;
mod bridge;
mod catalog;
mod plugin_panel;
mod preview_server;
mod composer;
mod conversation;
mod detail;
mod dsh_agent;
mod host;
mod markdown;
mod icons;
mod agents;
mod custom_model;
mod model;
mod router;
mod models;
mod selftest;
mod settings;
#[cfg(target_os = "macos")]
mod theme_view;
mod shell;
mod sidebar;
mod spec;
mod state;
mod workspace;
mod workspace_ui;
mod welcome;
mod widgets;

use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;

/// Generated colour set. See `docs/ui-spec/build_gtk_tokens.js`.
///
/// Resolved via `CARGO_MANIFEST_DIR` rather than a relative path so the includes
/// keep working if this file moves. The crate lives at `crates/eb`, so the
/// repository root — where `docs/` is — is two levels up.
const TOKENS_CSS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/ui-spec/gtk/tokens-light.css"
));
/// Hand-written rules for the parts the token set cannot express.
const SHELL_CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/gtk.css"));

/// Generated numeric utility classes (spacing/radius/type). New components use
/// these classes instead of repeating literal font sizes and radii.
const METRICS_CSS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/ui-spec/gtk/tokens-metrics.css"
));

/// The message submitted by `EB_DEMO=1`, so the conversation state can be
/// photographed without a human typing into it.
const DEMO_PROMPT: &str = "Show me what a task's conversation looks like.";

fn main() {
    if let Ok(requested) = std::env::var("EB_ACP_PROBE") {
        std::process::exit(acp_probe(&requested));
    }

    ensure_gtk_im_modules();

    let application = gtk::Application::new(Some(spec::APP_ID), Default::default());
    application.connect_startup(|_| {
        load_css();
        load_app_icon();
    });
    application.connect_activate(build_ui);
    application.connect_shutdown(|_| bridge::stop());
    preview_server::ensure();
    application.run();
    bridge::stop();
}

/// `EB_ACP_PROBE=1` probes the selected ACP slot; `EB_ACP_PROBE=jcode` probes a
/// named one. Handshake only — `initialize` and `session/new`, never
/// `session/prompt` — so it answers "does this agent speak ACP here?" without
/// spending a token or needing GTK. Exit 0 on a completed handshake, 1 when the
/// agent refuses, 2 on an unknown slot.
fn acp_probe(requested: &str) -> i32 {
    let choice = match requested {
        "" | "1" => agents::selected(),
        other => match agents::AcpAgent::parse(other) {
            Some(agent) => agent,
            None => {
                let ids: Vec<&str> = agents::all().iter().map(|agent| agent.id()).collect();
                eprintln!("acp-probe: unknown agent '{other}' (try: {})", ids.join(" "));
                return 2;
            }
        },
    };
    let (program, args) = choice.launch();
    println!(
        "acp-probe: {} -> {} {}",
        choice.id(),
        program.display(),
        args.join(" ")
    );
    if !program.is_file() {
        eprintln!("acp-probe: {} failed: {} is not an executable file", choice.id(), program.display());
        return 1;
    }
    let workspace = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let agent = eb_plugin_acp_agent::AcpAgent::new(router::acp_config(choice));
    match agent.probe(&workspace) {
        Ok(probe) => {
            println!("acp-probe: {} ok name={}", choice.id(), probe.agent_name);
            println!("acp-probe: {} capabilities={}", choice.id(), probe.capabilities);
            0
        }
        Err(error) => {
            eprintln!("acp-probe: {} failed: {error}", choice.id());
            1
        }
    }
}

/// Point GTK at its input-method module cache, so IME input works.
///
/// GTK derives its installation prefix from the path of the running executable.
/// A cargo binary such as `target/debug/evobuddy` lives outside the GTK install
/// tree, so GTK cannot find `immodules.cache`: its input-method table is empty,
/// it silently falls back to the "simple" context, and typing with an IME
/// (Chinese, Japanese, Korean, ...) does nothing.
///
/// We locate the *loaded* libgtk with `dladdr`, derive its `lib/` directory and
/// set `GTK_IM_MODULE_FILE` to the matching cache. Any user-supplied setting
/// (`GTK_IM_MODULE_FILE` or `GTK_EXE_PREFIX`) takes precedence.
///
/// Duplicated in `src/bin/notebook/main.rs`; the two binaries should be unified
/// behind a shared module before the demo is dropped.
#[cfg(target_os = "macos")]
fn ensure_gtk_im_modules() {
    use std::ffi::{c_char, CStr};
    use std::path::Path;

    if std::env::var_os("GTK_IM_MODULE_FILE").is_some()
        || std::env::var_os("GTK_EXE_PREFIX").is_some()
    {
        return;
    }

    // SAFETY: `dlsym`/`dladdr` only resolve addresses and write into `info`;
    // `dli_fname` points at loader-owned storage that stays valid afterwards.
    let cache = unsafe {
        let symbol = libc::dlsym(
            libc::RTLD_DEFAULT,
            c"gtk_init_check".as_ptr() as *const c_char,
        );
        if symbol.is_null() {
            return;
        }

        let mut info: libc::Dl_info = std::mem::zeroed();
        if libc::dladdr(symbol, &mut info) == 0 || info.dli_fname.is_null() {
            return;
        }

        let libgtk = CStr::from_ptr(info.dli_fname)
            .to_string_lossy()
            .into_owned();
        let Some(libdir) = Path::new(&libgtk).parent() else {
            return;
        };
        libdir.join("gtk-3.0").join("3.0.0").join("immodules.cache")
    };

    if cache.is_file() {
        std::env::set_var("GTK_IM_MODULE_FILE", cache);
    }
}

/// On other platforms GTK locates its input-method modules on its own.
#[cfg(not(target_os = "macos"))]
fn ensure_gtk_im_modules() {}

/// The application mark (`assets/logo.png`, resampled into this file). Window
/// managers and docks take a running window's icon from the app, not from the
/// desktop entry, so an AppImage or an unpacked bundle (Kylin's dock among
/// them) would otherwise show a generic gear.
const APP_ICON_PNG: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/appicon.png"));

/// Installs the embedded mark as the default icon for every window.
fn load_app_icon() {
    // Manual loader rather than `Pixbuf::from_read`: the upstream
    // gdk_pixbuf_new_from_stream path never closes its internal loader, and
    // GdkPixbuf warns on finalize.
    let icon = (|| -> Option<gtk::gdk::gdk_pixbuf::Pixbuf> {
        let loader = gtk::gdk::gdk_pixbuf::PixbufLoader::new();
        loader.write(APP_ICON_PNG).ok()?;
        loader.close().ok()?;
        loader.pixbuf()
    })();
    match icon {
        Some(pixbuf) => gtk::Window::set_default_icon(&pixbuf),
        None => eprintln!("[evobuddy] app icon decode failed"),
    }
    // Wayland compositors match the desktop entry by app id instead; keep the
    // icon-theme name as the fallback for installed bundles.
    gtk::Window::set_default_icon_name("evobuddy");
    #[cfg(target_os = "macos")]
    apply_macos_dock_icon();
}

/// macOS: the Dock takes its icon from NSApplication, not from the GTK window,
/// so a bare `cargo run` binary shows the generic executable icon. Feed the
/// embedded mark straight to NSApp.
#[cfg(target_os = "macos")]
fn apply_macos_dock_icon() {
    use objc2::AnyThread;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::{NSData, MainThreadMarker};

    let Some(marker) = MainThreadMarker::new() else {
        return;
    };
    let data = unsafe {
        NSData::dataWithBytes_length(
            APP_ICON_PNG.as_ptr().cast::<std::ffi::c_void>(),
            APP_ICON_PNG.len(),
        )
    };
    let decoded = NSImage::initWithData(NSImage::alloc(), &data);
    let Some(image) = decoded else {
        eprintln!("[evobuddy] app icon: NSImage decode failed");
        return;
    };
    unsafe { NSApplication::sharedApplication(marker).setApplicationIconImage(Some(&image)) };
}

fn load_css() {
    let provider = gtk::CssProvider::new();
    let css = format!("{TOKENS_CSS}\n{METRICS_CSS}\n{SHELL_CSS}");

    // Never swallow this. A rejected stylesheet disables *all* styling, GTK
    // prints nothing of its own accord, and the app falls back to the default
    // theme — which looks plausible enough to pass a visual check.
    match provider.load_from_data(css.as_bytes()) {
        Ok(()) => {
            if std::env::var_os("EB_SELFTEST").is_some() {
                println!("css: parsed {} bytes", css.len());
            }
        }
        Err(error) => {
            eprintln!("css: LOAD FAILED, styling disabled: {error}");
            // A broken stylesheet is exactly the silent failure the harness
            // exists to catch; do not let an assertion run pass on it.
            if std::env::var_os("EB_ASSERT").is_some()
                || std::env::var_os("EB_CLICK_ASSERT").is_some()
            {
                std::process::exit(1);
            }
        }
    }

    match gtk::gdk::Screen::default() {
        Some(screen) => gtk::StyleContext::add_provider_for_screen(
            &screen,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        ),
        None => eprintln!("css: no default screen, styling will not apply"),
    }
}

fn build_ui(application: &gtk::Application) {
    // Saved model settings must be in the environment before the dsh bridge
    // is spawned and before the composer reads its model menu.
    crate::models::apply_env(&crate::models::load());
    // Mount the plugins first: the window is rendered from their contributions.
    let host = host::Host::mount();

    let store = model::store(model::App::new());
    if !selftest::requested() {
        let loaded = state::load();
        let mut app = store.borrow_mut();
        // router.json was read when the host mounted. Keep the composer
        // picker on that ◇ mode. Self-tests stay on Default so screenshots
        // do not follow a developer's own setting.
        app.permission = crate::router::permission_mode();
        app.workspaces = loaded.workspaces;
        app.tasks = loaded
            .tasks
            .into_iter()
            .map(|mut task| {
                for turn in &mut task.turns {
                    if let model::Turn::Agent { streaming, .. } = turn {
                        *streaming = false;
                    }
                }
                task
            })
            .collect();
        if !app.tasks.is_empty() {
            app.route = model::Route::Task(0);
            app.nav = None;
            app.model = crate::router::menu_index(
                app.tasks.first().and_then(|task| task.agent_route.as_deref()),
            );
            app.workspace = app
                .tasks
                .first()
                .and_then(|task| task.workspace_id.as_deref())
                .and_then(|id| app.workspaces.iter().position(|workspace| workspace.id == id))
                .map(|position| position + 2)
                .unwrap_or(0);
        }
        drop(app);
        let cleaned = {
            let mut app = store.borrow_mut();
            let model::App { workspaces, tasks, .. } = &mut *app;
            let removed = workspace::cleanup_temporary(workspaces, tasks);
            if removed > 0 {
                app.workspace = 0;
            }
            removed
        };
        drop(cleaned);
        if cleaned > 0 || !store.borrow().tasks.is_empty() {
            // Persist the workspace migration immediately.
            state::save(&store);
        }
    }
    let (views, probes) = shell::build(
        application,
        &store,
        &host.contribs,
        host.answer(),
        host.agent_runtime.clone(),
    );
    let views = Rc::new(views);

    // Attach before the first `show_all`, so the harness still sees the very
    // first allocation.
    if selftest::requested() {
        selftest::attach(&views, &probes, application);
    }

    shell::finish(&views, &store);
    let restored_session = {
        store
            .borrow()
            .tasks
            .iter()
            .find_map(|task| task.session_id.clone())
    };
    if let Some(session_id) = restored_session {
        bridge::ensure();
        match bridge::resume_session(&session_id) {
            Ok(()) => {
                eprintln!("phase1: resumed {session_id}");
                views.refresh(&store);
            }
            Err(error) => eprintln!("phase1: resume failed: {error}"),
        }
    }

    if std::env::var_os("EB_ARTIFACT").is_some() {
        {
            let mut app = store.borrow_mut();
            app.tasks.push(model::Task {
                title: "Artifact test".into(),
                meta: String::new(),
                turns: vec![
                    model::Turn::User {
                        text: "Explain the selected artifact.".into(),
                        time: "now".into(),
                    },
                    model::Turn::Agent {
                        name: "EvoBuddy".into(),
                        status: Some("Finished".into()),
                        blocks: vec![model::Block::Text(
                            "This is the real EvoBuddy answer for the bound session.".into(),
                        )],
                        streaming: false,
                        time: Some(eb_api::model::unix_now()),
                    },
                ],
                agent_route: None,
                last_route: None,
                pending_permission: None,
                started_at: None,
                updated_at: None,
                pinned: false,
                archived: false,
                failure_unseen: false,
                unread: false,
                artifact: Some(model::Artifact {
                    name: "demo.py".into(),
                    code: "print('hello')\n".into(),
                }),
                // Two files so the tab strip's multi-file path is exercised
                // by the audit and the screenshots; the newest is selected.
                artifacts: vec![
                    model::Artifact {
                        name: "notes.md".into(),
                        code: "# Notes\n\nThe demo writes two files.\n".into(),
                    },
                    model::Artifact {
                        name: "demo.py".into(),
                        code: "print('hello')\n".into(),
                    },
                ],
                session_id: None,
                workspace_id: None,
                usage: None,
            });
            app.route = model::Route::Task(0);
            app.nav = None;
            app.detail_visible = true;
        }
        views.refresh(&store);
    }

    if let Ok(plugin) = std::env::var("EB_PLUGIN") {
        store.borrow_mut().detail_visible = true;
        views.refresh(&store);
        if let Some(url) = plugin_panel::url(&plugin) {
            preview_server::ensure();
            #[cfg(target_os = "macos")]
            theme_view::open_embedded(url, views.detail.body.upcast_ref());
        }
    }

    if std::env::var_os("EB_PANEL").is_some() {
        store.borrow_mut().detail_visible = true;
        views.refresh(&store);
        #[cfg(target_os = "macos")]
        {
            views.detail.root.set_size_request(520, -1);
            views.outer.set_position(views.window.size().0 - 520);
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            crate::bridge::ensure();
            crate::preview_server::ensure();
            if std::env::var_os("EB_EMBED_PANEL").is_some() {
                crate::theme_view::open_embedded(
                    crate::preview_server::url(),
                    views.detail.body.upcast_ref(),
                );
            } else {
                crate::theme_view::open_panel(
                    crate::preview_server::url(),
                    views.detail.body.upcast_ref(),
                );
            }
        }
    }

    if std::env::var_os("EB_DEMO").is_some()
        && std::env::var_os("EB_DEMO_BLOCKS").is_none()
        && std::env::var_os("EB_FAKE_DEMO").is_none()
        && std::env::var_os("EB_CLICK").is_none()
        && std::env::var_os("EB_CLICK_ASSERT").is_none()
        && std::env::var_os("EB_ENTER").is_none()
    {
        views.agent.submit(DEMO_PROMPT.to_string());
        // The demo agent emits synchronously, so the first frame should show the
        // finished conversation (the same pixels the old one-shot path drew).
        views.refresh(&store);
    }

    // Harness: exercise the real send-button path (not Enter, not the direct
    // Runner API). This is the path a mouse click takes.
    if std::env::var_os("EB_SEND_TEST").is_some() {
        views.welcome_composer.fill("hi");
        views.welcome_composer.click_send();
    }

    // Render the second demo answer (diff/exec/table) without going through the
    // streaming agent's manual send path. The first submit opens a task; the
    // second is the new-content variant.
    if std::env::var_os("EB_DEMO_BLOCKS").is_some() {
        views.agent.submit(DEMO_PROMPT.to_string());
        // Explicitly a second task: normal chat sends now continue the current
        // conversation, which is what the product does.
        views.agent.submit_as_new(DEMO_PROMPT.to_string());
        views.refresh(&store);
    }

    // Exercise the post-build_ui path: a real user click happens after this
    // function has returned, when only the app-lifetime keepalive holds Views.
    if let Some(ms) = std::env::var("EB_SEND_AFTER_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        let views = Rc::downgrade(&views);
        gtk::glib::timeout_add_local_once(Duration::from_millis(ms), move || {
            if let Some(views) = views.upgrade() {
                views.welcome_composer.fill("hi");
                views.welcome_composer.click_send();
            }
        });
    }

    // Harness: send a second message from the in-conversation composer, which
    // must continue the current task rather than create a sidebar row.
    if let Some(ms) = std::env::var("EB_CHAT_SEND_AFTER_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    {
        let views = Rc::downgrade(&views);
        gtk::glib::timeout_add_local_once(Duration::from_millis(ms), move || {
            if let Some(views) = views.upgrade() {
                views.chat_composer.fill("second message");
                views.chat_composer.click_send();
            }
        });
    }

    // ACP harness: submit and leave the window open so a delayed screenshot
    // (`EB_SHOT_AFTER_MS`) can catch the real stream after it settles.
    if std::env::var_os("EB_ACP_DEMO").is_some() {
        let prompt = std::env::var("EB_ACP_PROMPT")
            .unwrap_or_else(|_| DEMO_PROMPT.to_string());
        views.agent.submit(prompt);
        views.refresh(&store);
    }

    // A harness mode for the fake streaming agent: submit, let the events drain,
    // then quit. `EB_CANCEL_AFTER_MS` exercises the cancellation path.
    if std::env::var_os("EB_FAKE_DEMO").is_some() {
        views.agent.submit(DEMO_PROMPT.to_string());
        let poll_views = views.clone();
        let application = application.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(50), move || {
            if poll_views.agent.is_busy() {
                gtk::glib::ControlFlow::Continue
            } else {
                println!("fake-demo: finished");
                application.quit();
                gtk::glib::ControlFlow::Break
            }
        });

        if let Some(ms) = std::env::var("EB_FAKE_STOP_AFTER_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
        {
            let composer = views.chat_composer.clone();
            gtk::glib::timeout_add_local_once(Duration::from_millis(ms), move || {
                composer.click_send();
            });
        }
    }

    // Background-streaming harness: task A starts, and task B is submitted
    // while A is still streaming. A must keep running to its own terminal
    // event — the second submit may not cancel it. Exits 1 on any regression;
    // pair with EB_FAKE_AGENT so the streams overlap.
    if std::env::var_os("EB_TWO_TASK_TEST").is_some() {
        views.agent.submit(DEMO_PROMPT.to_string());
        let second_views = views.clone();
        gtk::glib::timeout_add_local_once(Duration::from_millis(250), move || {
            second_views.agent.submit_as_new("second task prompt".to_string());
        });
        let poll_views = views.clone();
        let poll_store = store.clone();
        let application = application.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(50), move || {
            if poll_views.agent.is_busy() {
                return gtk::glib::ControlFlow::Continue;
            }
            let app = poll_store.borrow();
            let mut problems: Vec<String> = Vec::new();
            if app.tasks.len() != 2 {
                problems.push(format!("expected 2 tasks, got {}", app.tasks.len()));
            }
            for (index, task) in app.tasks.iter().enumerate() {
                match task.turns.last() {
                    Some(model::Turn::Agent {
                        streaming,
                        status,
                        blocks,
                        ..
                    }) => {
                        if *streaming {
                            problems.push(format!("task {index} is still streaming"));
                        }
                        if let Some(status) = status {
                            if status.contains("Cancelled") || status.starts_with("Failed") {
                                problems.push(format!("task {index} terminal status {status:?}"));
                            }
                        }
                        if blocks.is_empty() {
                            problems.push(format!("task {index} has no answer blocks"));
                        }
                    }
                    _ => problems.push(format!("task {index} has no agent turn")),
                }
            }
            drop(app);
            if !problems.is_empty() {
                for problem in &problems {
                    eprintln!("two-task: FAIL {problem}");
                }
                std::process::exit(1);
            }
            println!("two-task: finished");
            application.quit();
            gtk::glib::ControlFlow::Break
        });
    }

    // Scroll-follow harness. Reading history during a long run used to be
    // impossible: every streamed delta re-jumped the message list to the
    // bottom. This flicks the view up mid-stream, requires the deltas to leave
    // it alone, then checks that scrolling back down resumes following. Pair
    // with EB_FAKE_AGENT and a slow, long stream.
    if std::env::var_os("EB_FOLLOW_TEST").is_some() {
        // The prompt, not the answer, is what makes the transcript taller than
        // the viewport: the flick below scrolls it, and with the short demo
        // prompt there is no scroll range to move. The harness brings its own
        // height so it does not depend on whatever the state file happens to
        // hold.
        let prompt = format!(
            "{DEMO_PROMPT}\n{}",
            "scroll-follow harness context line. ".repeat(120)
        );
        views.agent.submit(prompt);
        let scroll = views.conversation.scroll.clone();
        let pinned = views.conversation.pinned.clone();
        let agent = views.agent.clone();
        let store = store.clone();
        let application = application.clone();
        let painted = views.conversation.painted_task.clone();
        // How much answer text is on screen. Growing text is the event under
        // test; whether it happens to add a layout line is beside the point.
        let streamed = move || -> usize {
            let app = store.borrow();
            let task = match painted.get() {
                Some(index) => app.tasks.get(index),
                None => app.tasks.last(),
            };
            task.and_then(|task| task.turns.last())
                .map(|turn| match turn {
                    model::Turn::Agent { blocks, .. } => blocks
                        .iter()
                        .filter_map(|block| match block {
                            model::Block::Text(text) => Some(text.len()),
                            _ => None,
                        })
                        .sum(),
                    _ => 0,
                })
                .unwrap_or(0)
        };
        let phase = std::rc::Rc::new(std::cell::Cell::new(0u8));
        let since = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let held = std::rc::Rc::new(std::cell::Cell::new((0.0f64, 0usize)));
        gtk::glib::timeout_add_local(Duration::from_millis(200), move || {
            since.set(since.get() + 200);
            let adjustment = scroll.vadjustment();
            match phase.get() {
                0 if since.get() >= 600 => {
                    if !agent.is_busy() {
                        eprintln!("follow-test: FAIL the stream ended before the scroll");
                        std::process::exit(1);
                    }
                    adjustment.set_value((adjustment.upper() - adjustment.page_size() - 300.0).max(0.0));
                    let below = adjustment.upper() - adjustment.value() - adjustment.page_size();
                    if pinned.get() || below < 100.0 {
                        eprintln!(
                            "follow-test: FAIL scrolling up left pinned={} at {below:.0}px from the \
                             bottom (scroll range {:.0}px, page {:.0}px)",
                            pinned.get(),
                            adjustment.upper() - adjustment.page_size(),
                            adjustment.page_size()
                        );
                        std::process::exit(1);
                    }
                    held.set((adjustment.value(), streamed()));
                    phase.set(1);
                    gtk::glib::ControlFlow::Continue
                }
                0 | 1 => {
                    if phase.get() == 1 && since.get() >= 1400 {
                        let (value_before, text_before) = held.get();
                        if !agent.is_busy() {
                            eprintln!("follow-test: FAIL the stream ended before the hold check");
                            std::process::exit(1);
                        }
                        let text_now = streamed();
                        if text_now <= text_before {
                            eprintln!("follow-test: FAIL nothing streamed to interrupt the reader");
                            std::process::exit(1);
                        }
                        let moved = adjustment.value() - value_before;
                        if moved.abs() > 1.0 {
                            eprintln!(
                                "follow-test: FAIL a delta yanked the view {moved:+.0}px while \
                                 {text_now} chars streamed in"
                            );
                            std::process::exit(1);
                        }
                        adjustment.set_value(adjustment.upper() - adjustment.page_size());
                        phase.set(2);
                    }
                    gtk::glib::ControlFlow::Continue
                }
                2 if since.get() >= 2200 => {
                    let below = adjustment.upper() - adjustment.value() - adjustment.page_size();
                    if agent.is_busy() && below > 24.0 {
                        eprintln!(
                            "follow-test: FAIL following never resumed, {below:.0}px below the bottom"
                        );
                        std::process::exit(1);
                    }
                    println!("follow-test: finished");
                    application.quit();
                    gtk::glib::ControlFlow::Break
                }
                _ => gtk::glib::ControlFlow::Continue,
            }
        });
    }

    // Lazy-window harness. A long conversation renders only its newest turns.
    // This reaches the top of that window twice and checks, per flick, that an
    // earlier slice arrives (exactly `ATTACH_STEP` turns, so one flick is one
    // slice and cannot walk the transcript), that the render state and the
    // container agree, and that the reader is left on the line they were
    // looking at. That line is measured as the anchor frame's offset in the
    // message list minus the scroll value — the content pixel under the top edge
    // of the viewport — so the pane's own position and the frame's margins cancel
    // between the two samples.
    //
    // Measured against the frames themselves, not the height of the content: the
    // reveal negotiates with provisional wrapped-text heights and the transcript
    // above the reader grows again a pass or two later, which is exactly the push
    // the anchor has to absorb. Both flicks are therefore required to land within
    // 4px of the line the reader had.
    //
    // Needs EB_STATE=<tasks.json> from docs/make_heavy_state.py with a
    // conversation longer than two slices; EB_LOAD_SHOT=<path> captures it.
    if std::env::var_os("EB_LOAD_TEST").is_some() {
        let step = conversation::ATTACH_STEP;
        let needed = conversation::ATTACH_TURNS + 2 * step;
        let index = match store
            .borrow()
            .tasks
            .iter()
            .position(|task| task.turns.len() > needed)
        {
            Some(index) => index,
            None => {
                eprintln!(
                    "load-test: FAIL needs a saved task with more than {needed} turns — \
                     python3 docs/make_heavy_state.py /tmp/eb-heavy/tasks.orig.json 3 24 (each \
                     round is two turns), then EB_STATE a scratch copy of it"
                );
                std::process::exit(1);
            }
        };
        let open_task = views.open_task.clone();
        open_task(index);
        let messages = views.conversation.messages.clone();
        let scroll = views.conversation.scroll.clone();
        let application = application.clone();
        let shot = std::env::var("EB_LOAD_SHOT").ok();
        let shot_views = views.clone();
        let probe = {
            let views = views.clone();
            move || views.conversation.window()
        };
        // The frame the reader has under the top edge of the viewport. The
        // oldest mounted frame is that one once the view is at the top;
        // `Container::children()` walks a GtkBox in reverse, so it is the last
        // child here.
        let anchor: std::rc::Rc<std::cell::RefCell<Option<gtk::Widget>>> =
            std::rc::Rc::new(std::cell::RefCell::new(None));
        let before = std::rc::Rc::new(std::cell::Cell::new((0usize, 0usize, 0i32)));
        let drifts = std::rc::Rc::new(std::cell::Cell::new((0i32, 0i32)));
        let phase = std::rc::Rc::new(std::cell::Cell::new(0u8));
        let since = std::rc::Rc::new(std::cell::Cell::new(0u32));
        gtk::glib::timeout_add_local(Duration::from_millis(200), move || {
            since.set(since.get() + 200);
            let adjustment = scroll.vadjustment();
            let line = |widget: &gtk::Widget| -> i32 {
                widget.allocation().y() - adjustment.value() as i32
            };
            let anchor_widget = || -> gtk::Widget {
                anchor
                    .borrow()
                    .as_ref()
                    .cloned()
                    .expect("the flick picked an anchor frame")
            };
            // The gesture under test: `value_changed` raises the load request and
            // asks the shell to repaint; the repaint attaches the slice and the
            // idle after it moves the view back down by exactly its height.
            let flick = |first: usize, mounted: usize| -> (usize, usize, i32) {
                *anchor.borrow_mut() = Some(
                    messages
                        .children()
                        .last()
                        .cloned()
                        .expect("the message list holds its window"),
                );
                adjustment.set_value(if adjustment.value() > 1.0 { 0.0 } else { 2.0 });
                (first, mounted, line(&anchor_widget()))
            };
            let fail = |message: String| -> ! {
                eprintln!("load-test: FAIL {message}");
                std::process::exit(1)
            };
            match phase.get() {
                0 if since.get() >= 700 => {
                    let (first, mounted) = probe();
                    let children = messages.children();
                    if mounted != children.len() || first < 2 * step {
                        eprintln!(
                            "load-test: FAIL task {index} paints window {first}+{mounted} over {} \
                             children: a window within two slices of turn 0 cannot show two loads",
                            children.len()
                        );
                        std::process::exit(1);
                    }
                    if adjustment.upper() - adjustment.page_size() < 200.0 {
                        eprintln!(
                            "load-test: FAIL the rendered window does not overflow the viewport \
                             (range {:.0}px), so there is no top edge to reach",
                            adjustment.upper() - adjustment.page_size()
                        );
                        std::process::exit(1);
                    }
                    before.set(flick(first, mounted));
                    phase.set(1);
                }
                1 if since.get() >= 1300 => {
                    let (first, mounted, landing) = before.get();
                    let loaded = probe();
                    let children = messages.children().len();
                    if let Some(message) = slice_imbalance((first, mounted), loaded, children, step)
                    {
                        fail(format!("first flick: {message}"));
                    }
                    let drift = line(&anchor_widget()) - landing;
                    // The cold load is the case the anchor exists for: the
                    // reveal's own measure is not final, so without the late
                    // corrections the reader is left a slice-height from their
                    // line. With them the line holds on the first flick too.
                    if drift.abs() > 4 {
                        fail(format!(
                            "the first flick left the reader {drift}px off their line \
                             ({first}+{mounted} -> {}+{}, the view is {:.0}px down the content \
                             it just grew)",
                            loaded.0, loaded.1, adjustment.value()
                        ));
                    }
                    drifts.set((drift, 0));
                    before.set(flick(loaded.0, loaded.1));
                    phase.set(2);
                }
                2 if since.get() >= 1900 => {
                    let (first, mounted, landing) = before.get();
                    let loaded = probe();
                    let children = messages.children().len();
                    if let Some(message) = slice_imbalance((first, mounted), loaded, children, step)
                    {
                        fail(format!("second flick: {message}"));
                    }
                    let drift = line(&anchor_widget()) - landing;
                    // The warm case: the transcript has been measured once
                    // already, so nothing grows under this slice at all.
                    if drift.abs() > 4 {
                        fail(format!(
                            "the second flick moved the reader {drift}px off their line \
                             ({first}+{mounted} -> {}+{}, first flick drifted {}px)",
                            loaded.0,
                            loaded.1,
                            drifts.get().0
                        ));
                    }
                    let (first_drift, _) = drifts.get();
                    match shot.clone() {
                        Some(path) => {
                            selftest::snapshot(&shot_views, &path);
                            println!(
                                "load-test: finished task={index} window={}+{}->{}+{} \
                                 drift={first_drift}px,{drift}px shot={path}",
                                first, mounted, loaded.0, loaded.1
                            );
                        }
                        None => println!(
                            "load-test: finished task={index} window={}+{}->{}+{} \
                             drift={first_drift}px,{drift}px",
                            first, mounted, loaded.0, loaded.1
                        ),
                    }
                    application.quit();
                    return gtk::glib::ControlFlow::Break;
                }
                _ => {}
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    // Window-resize harness. The window server resizes the frame under us and
    // GTK learns the new size only inside its own configure-event handler, so a
    // connected handler that returns `true` (stop emission) leaves the widget
    // tree allocated at the old width while the server stretches the stale
    // surface over the new frame — that is the report "switch to a task and the
    // sidebar fills the whole window, no conversation". Resizing the GdkWindow
    // directly reproduces the one-sided resize: GtkWindow is not told about it,
    // GDK answers with a configure event.
    // A resize is a request, and the layout trails it by more than one round:
    // the server answers with a configure event, GTK re-lays out the paned, and
    // the viewport re-allocates its child on a pass of its own. So each move is
    // asked for until the *frame* lands, then measured only once two consecutive
    // reads of the pane and the message column agree. Two things that buys: a
    // minimum-width hint that is one layout behind never fails the run (GTK
    // publishes the smaller hint after it has laid out at the new size, and a
    // user dragging keeps asking, so the lag is invisible), and the numbers
    // checked are the settled ones. Invariants on both the way down and the way
    // back up: GTK's cached size and the surface drawn into agree, the frame
    // lands on the requested width, the sidebar keeps its fixed 280px, the
    // content stack takes the slack, and the message column is re-centred for
    // the pane it was given (the one number that can still exceed the pane is
    // reported as a note, see `verify`).
    // EB_RESIZE_TEST=<width> (default 1000); EB_RESIZE_TASK=1 mounts a saved
    // conversation first, which is the situation in the report;
    // EB_RESIZE_SHOT=<prefix> writes <prefix>-narrow.png / -wide.png.
    if std::env::var_os("EB_RESIZE_TEST").is_some() {
        let width: i32 = std::env::var("EB_RESIZE_TEST")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1000);
        let wide = width + 470;
        let mount = std::env::var_os("EB_RESIZE_TASK").is_some();
        let views = views.clone();
        let store = store.clone();
        let application = application.clone();
        let phase = std::rc::Rc::new(std::cell::Cell::new(0u8));
        let since = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let settled = std::rc::Rc::new(std::cell::Cell::new((-1i32, -1i32)));
        gtk::glib::timeout_add_local(Duration::from_millis(200), move || {
            since.set(since.get() + 200);
            // A configure event only becomes a new allocation inside GTK's window
            // update handler, and that handler runs from the GDK frame clock - a
            // clock that does not tick for a process which is not the active app,
            // which a bare `cargo run` never is. The frame itself arrives
            // (`window.size()` moves) and the tree keeps its old allocation, so the
            // harness hands it over the way `gtk_window_process_updates` would:
            // allocate the toplevel at the frame GTK was told about.
            let (frame_w, frame_h) = views.window.size();
            let alloc = views.window.allocation();
            if alloc.width() != frame_w || alloc.height() != frame_h {
                views.window.size_allocate(&gtk::gdk::Rectangle::new(0, 0, frame_w, frame_h));
            }
            let fail = |message: String| -> ! {
                eprintln!("resize-test: FAIL {message}");
                std::process::exit(1)
            };
            let ask = |want: i32| {
                let Some(surface) = views.window.window() else {
                    fail("the window has no GdkWindow to resize".into());
                };
                surface.resize(want, views.window.size().1);
            };
            // (frame GTK believes, surface drawn into, content stack, message column)
            let measure = || -> (i32, i32, i32, i32) {
                let Some(surface) = views.window.window() else {
                    fail("the window has no GdkWindow to measure".into());
                };
                // The column's own allocation is the width the transcript is
                // wrapped at: it is centred by its scroller, so it has no margins
                // to add back, and it must never exceed the pane.
                let column = views.conversation.messages.allocation().width();
                (
                    views.window.size().0,
                    surface.width(),
                    views.stack.allocation().width(),
                    column,
                )
            };
            let verify = |label: &str, want: i32, state: (i32, i32, i32, i32)| {
                let (toplevel, drawn, stack, column) = state;
                if (toplevel - drawn).abs() > 2 {
                    fail(format!(
                        "{label}: the surface is {drawn}px wide but GTK lays out {toplevel}px; the \
                         configure event never reached GtkWindow, so the window server stretches a \
                         stale surface over the frame"
                    ));
                }
                if (toplevel - want).abs() > 2 {
                    fail(format!(
                        "{label}: asked for a {want}px window and GTK has {toplevel}px \
                         (surface {drawn}px) after {}ms",
                        since.get()
                    ));
                }
                if views.sidebar.root.allocation().width() != crate::spec::SIDEBAR_WIDTH {
                    fail(format!(
                        "{label}: the sidebar was allocated {}, not its fixed {}",
                        views.sidebar.root.allocation().width(),
                        crate::spec::SIDEBAR_WIDTH
                    ));
                }
                let want_stack = want - crate::spec::SIDEBAR_WIDTH - 20;
                if stack < want_stack {
                    fail(format!(
                        "{label}: the content stack got {stack}px of a {toplevel}px window \
                         (expected at least {want_stack}px)"
                    ));
                }
                if column > stack {
                    // Reported, not failed: `_gtk_text_layout_get_min_width` walks
                    // the *wrapped* display lines and only breaks at spaces, so a
                    // Chinese paragraph looks like one unbreakable word and the
                    // text view's minimum width is stuck at the last width it was
                    // laid out at (~752). Below a ~1030px window the column
                    // therefore overflows its pane and the scroller scrolls
                    // sideways. The alternatives were a window that could not
                    // shrink at all (the report) or a minimum width of
                    // sidebar + 752; re-wrapping needs fresh text views.
                    eprintln!(
                        "resize-test: note {label}: the message column is {column}px wide inside a \
                         {stack}px pane"
                    );
                }
                println!(
                    "resize-test: {label} width={want} toplevel={toplevel} surface={drawn} \
                     sidebar={} stack={stack} column={column} after={}ms",
                    views.sidebar.root.allocation().width(),
                    since.get()
                );
            };
            // True once the pane and the column stop moving between two reads
            // *and* the pane agrees with the frame. Two identical reads are not
            // enough on their own: the tree can hand back the same stale
            // allocation twice while the relayout that the configure event
            // queued is still pending, and measuring there reads a mid-flight
            // frame as the "sidebar fills the window" symptom.
            let steady = |state: (i32, i32, i32, i32)| {
                let pane = state.0 - crate::spec::SIDEBAR_WIDTH;
                settled.get() == (state.2, state.3) && (state.2 - pane).abs() <= 40
            };
            let note = |state: (i32, i32, i32, i32)| settled.set((state.2, state.3));
            let shot = |suffix: &str| {
                if let Some(path) = std::env::var("EB_RESIZE_SHOT").ok() {
                    selftest::snapshot(&views, &format!("{path}-{suffix}.png"));
                }
            };
            match phase.get() {
                0 if since.get() >= 600 => {
                    if mount && !store.borrow().tasks.is_empty() {
                        (*views.open_task)(0);
                    }
                    ask(width);
                    phase.set(1);
                }
                1 if since.get() >= 800 => {
                    if (views.window.size().0 - width).abs() <= 2 || since.get() >= 5000 {
                        phase.set(2);
                    } else {
                        ask(width);
                    }
                }
                // One tick after the frame landed the tree is still reallocating,
                // so wait for a pair of identical reads before measuring.
                2 if since.get() >= 1000 => {
                    let state = measure();
                    if steady(state) || since.get() >= 6000 {
                        verify("shrink", width, state);
                        shot("narrow");
                        ask(wide);
                        settled.set((-1, -1));
                        phase.set(3);
                    } else {
                        note(state);
                    }
                }
                3 if since.get() >= 1400 => {
                    if (views.window.size().0 - wide).abs() <= 2 || since.get() >= 8000 {
                        phase.set(4);
                    } else {
                        ask(wide);
                    }
                }
                4 if since.get() >= 1600 => {
                    let state = measure();
                    if steady(state) || since.get() >= 10000 {
                        verify("grow", wide, state);
                        shot("wide");
                        application.quit();
                        return gtk::glib::ControlFlow::Break;
                    }
                    note(state);
                }
                _ => {}
            }
            gtk::glib::ControlFlow::Continue
        });
    }
    // Sidebar row-states harness, `EB_ROWS=1` (ui-spec §10-40). One widget per
    // row state: the 16px status glyph replaces the timestamp the way the
    // product's `trailingStatus` hides its `timeContainer`, and the unread dot
    // rides beside the timestamp instead. Point EB_STATE at a fixture from
    // docs/make_row_state.py (one row per state, each timestamp landing on a
    // rung of the ladder) and check the tree row by row. Fails outright when
    // the fixture is missing, so it cannot pass by rendering nothing.
    if std::env::var_os("EB_ROWS").is_some() {
        let views = views.clone();
        let store = store.clone();
        let application = application.clone();
        let since = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let fired = std::rc::Rc::new(std::cell::Cell::new(false));
        gtk::glib::timeout_add_local(Duration::from_millis(200), move || {
            since.set(since.get() + 200);
            if fired.get() || since.get() < 600 {
                return gtk::glib::ControlFlow::Continue;
            }
            fired.set(true);
            let fail = |message: String| -> ! {
                eprintln!("rows: FAIL {message}");
                std::process::exit(1);
            };
            if store.borrow().tasks.len() < 11 {
                fail(format!(
                    "needs the 11-row fixture from docs/make_row_state.py (mounted {})",
                    store.borrow().tasks.len()
                ));
            }
            // Startup settles every restored turn - a process that died
            // mid-answer must not look live - so Working cannot come from the
            // archive. Drive it: stream that task's last turn and repaint.
            let driven = {
                let mut app = store.borrow_mut();
                app.tasks
                    .iter_mut()
                    .find(|task| task.title == "row: working")
                    .and_then(|task| task.turns.last_mut())
                    .and_then(|turn| match turn {
                        model::Turn::Agent { streaming, .. } => {
                            *streaming = true;
                            Some(true)
                        }
                        model::Turn::User { .. } => None,
                    })
                    .unwrap_or(false)
            };
            if !driven {
                fail("the fixture's `row: working` has no agent turn to stream".to_string());
            }
            views.refresh(&store);
            // What stands in one row's trailing region. The three marks are
            // independent widgets, read by class rather than position, because
            // the product lets the unread dot ride alongside the timestamp but
            // never the status glyph.
            #[derive(Default)]
            struct Slot {
                meta: Option<String>,
                alert: bool,
                spinner: bool,
                unread: bool,
            }
            fn children_of(
                widget: &impl gtk::glib::object::IsA<gtk::Widget>,
            ) -> Vec<gtk::Widget> {
                widget
                    .as_ref()
                    .downcast_ref::<gtk::Container>()
                    .map(|container| container.children())
                    .unwrap_or_default()
            }
            let has_class = |widget: &gtk::Widget, name: &str| {
                widget
                    .style_context()
                    .list_classes()
                    .iter()
                    .any(|class| class.as_str() == name)
            };
            let mark_classes = [
                "eb-task-meta",
                "eb-row-alert",
                "eb-row-spinner",
                "eb-row-unread",
            ];
            // One row wrapper -> the widgets in its trailing region.
            let row_marks = |shell: &gtk::Widget| -> Vec<gtk::Widget> {
                let Some(button) = children_of(shell).first().cloned() else {
                    return Vec::new();
                };
                let line = button
                    .downcast_ref::<gtk::Bin>()
                    .and_then(|bin| bin.child())
                    .unwrap_or_else(|| button.clone());
                children_of(&line)
                    .into_iter()
                    .filter(|child| {
                        mark_classes
                            .iter()
                            .any(|name| has_class(child, name))
                    })
                    .collect()
            };
            let mut rows: Vec<(String, Slot)> = Vec::new();
            let mut row_shells: Vec<gtk::Widget> = Vec::new();
            for section in children_of(&views.sidebar.content) {
                for body in children_of(&section) {
                    for row in children_of(&body) {
                        let Some(button) = children_of(&row).first().cloned() else {
                            continue;
                        };
                        let matched = has_class(&button, "eb-task");
                        // The row's line is the button's single child.
                        let line = button
                            .downcast_ref::<gtk::Bin>()
                            .and_then(|bin| bin.child())
                            .unwrap_or_else(|| button.clone());
                        let children = children_of(&line);
                        let label_text = |widget: &gtk::Widget| {
                            widget
                                .downcast_ref::<gtk::Label>()
                                .map(|label| label.text().to_string())
                        };
                        let Some(title) = children
                            .iter()
                            .find(|child| has_class(child, "eb-task-title"))
                            .and_then(|child| label_text(child))
                        else {
                            // Section headers walk the same shape; a row with
                            // no title label is one of them, not a task.
                            continue;
                        };
                        let mut slot = Slot::default();
                        for child in &children {
                            if has_class(child, "eb-task-meta") {
                                slot.meta = label_text(child);
                            } else if has_class(child, "eb-row-alert") {
                                slot.alert = true;
                            } else if has_class(child, "eb-row-spinner") {
                                slot.spinner = true;
                            } else if has_class(child, "eb-row-unread") {
                                slot.unread = true;
                            }
                        }
                        rows.push((title, slot));
                        if matched {
                            row_shells.push(row);
                        }
                    }
                }
            }
            if rows.len() < 11 {
                fail(format!(
                    "needs the 11-row fixture from docs/make_row_state.py (mounted {})",
                    rows.len()
                ));
            }
            // title -> (the rung the clock must show, the glyph it must show,
            // whether the unread dot is on). `None` for the rung is the point:
            // a glyph replaces the timestamp.
            let mut bad: Vec<String> = Vec::new();
            for (title, slot) in &rows {
                let (want_meta, want_glyph, want_dot): (Option<&str>, &str, bool) =
                    match title.as_str() {
                        "row: failed unseen" => (None, "alert", false),
                        "row: planning" => (None, "spinner", false),
                        "row: working" => (None, "spinner", false),
                        "row: unread" => (Some("5m"), "", true),
                        "row: just now" => (Some("Just now"), "", false),
                        "row: failed seen" => (Some("3d"), "", false),
                        "row: yesterday" => (Some("Yesterday"), "", false),
                        "row: 3d" => (Some("3d"), "", false),
                        "row: 6d" => (Some("6d"), "", false),
                        "row: 8d" => (Some("Oct"), "", false),
                        "row: 400d" => (Some("/"), "", false),
                        other => {
                            bad.push(format!("unexpected row {other:?}"));
                            continue;
                        }
                    };
                let glyph = match want_glyph {
                    "alert" => slot.alert && !slot.spinner,
                    "spinner" => slot.spinner && !slot.alert,
                    _ => !slot.alert && !slot.spinner,
                };
                let clock = match (want_meta, &slot.meta) {
                    (Some(rung), Some(shown)) => shown.contains(rung),
                    (None, None) => true,
                    (Some(rung), None) => {
                        bad.push(format!("{title:?} shows no timestamp, wants {rung:?}"));
                        false
                    }
                    (None, Some(shown)) => {
                        bad.push(format!("{title:?} keeps {shown:?} under its glyph"));
                        false
                    }
                };
                if !glyph || !clock || slot.unread != want_dot {
                    bad.push(format!(
                        "{title:?} has badge={} spinner={} dot={} meta={:?}, wants glyph={want_glyph:?} dot={want_dot} meta={want_meta:?}",
                        slot.alert, slot.spinner, slot.unread, slot.meta
                    ));
                }
            }
            for (title, slot) in &rows {
                let mut marks: Vec<&str> = Vec::new();
                if slot.alert {
                    marks.push("red badge");
                }
                if slot.spinner {
                    marks.push("spinner");
                }
                if slot.unread {
                    marks.push("unread dot");
                }
                eprintln!(
                    "rows: {title}: {}{}",
                    if marks.is_empty() {
                        String::new()
                    } else {
                        format!("{} ", marks.join(" + "))
                    },
                    match &slot.meta {
                        Some(meta) => format!("time {meta:?}"),
                        None => "no time".to_string(),
                    }
                );
            }
            if !bad.is_empty() {
                fail(bad.join("; "));
            }
            if let Some(path) = std::env::var_os("EB_ROWS_SHOT") {
                let path = path.to_string_lossy().into_owned();
                crate::selftest::snapshot(&views, &path);
                eprintln!("rows: wrote {path}");
            }
            // `EB_ROWS_FADE=<png>` checks the mechanism behind the product's
            // hover rule: the timestamp, the glyph and the dot all drop out
            // while the pointer is on the card, or while its menu is open. GTK
            // CSS cannot hide a windowless widget, so the marks fade with
            // GtkWidget opacity - this makes the same call the enter/leave
            // handlers make, requires the getter to read it back, and leaves a
            // picture of a sidebar with every trailing mark gone.
            if let Some(path) = std::env::var_os("EB_ROWS_FADE") {
                let path = path.to_string_lossy().into_owned();
                let mut faded = 0;
                for shell in &row_shells {
                    for mark in row_marks(shell) {
                        mark.set_opacity(0.0);
                        if mark.opacity() == 0.0 {
                            faded += 1;
                        }
                    }
                }
                if faded == 0 {
                    fail("no row mark accepted the opacity fade".to_string());
                }
                crate::selftest::snapshot(&views, &path);
                eprintln!("rows: faded {faded} marks, wrote {path}");
            }
            eprintln!("rows: ok {} sidebar rows render their state", rows.len());
            application.quit();
            gtk::glib::ControlFlow::Break
        });
    }
    // Padding-scroll harness, `EB_PAD_SCROLL=1` (ui-spec §10-38). The transcript
    // column is 752 wide inside a much wider pane, and the scroller's input
    // window ends with the column, so a wheel event over the padding used to
    // land on nothing: the pointer had to be on the text for the page to move.
    // Mount the saved conversation, put one smooth wheel event at the pane-wide
    // catcher under the column, and require the transcript's own adjustment to
    // answer in both directions. Pair with `EB_STATE=<long tasks.json>`; the
    // harness fails outright if the mounted conversation has nothing to scroll,
    // so it cannot quietly degrade into a passing no-op.
    if std::env::var_os("EB_PAD_SCROLL").is_some() {
        let views = views.clone();
        let store = store.clone();
        let application = application.clone();
        let phase = std::rc::Rc::new(std::cell::Cell::new(0u8));
        let since = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let at = std::rc::Rc::new(std::cell::Cell::new(0f64));
        gtk::glib::timeout_add_local(Duration::from_millis(200), move || {
            since.set(since.get() + 200);
            let fail = |message: String| -> ! {
                eprintln!("pad-scroll: FAIL {message}");
                std::process::exit(1);
            };
            let value = || views.conversation.scroll.vadjustment().value();
            match phase.get() {
                0 if since.get() >= 600 => {
                    if store.borrow().tasks.is_empty() {
                        fail("needs a saved conversation with something to scroll (set EB_STATE)".into());
                    }
                    (*views.open_task)(0);
                    phase.set(1);
                }
                1 if since.get() >= 1200 => {
                    let adj = views.conversation.scroll.vadjustment();
                    if adj.upper() <= adj.page_size() + 1.0 {
                        fail(format!(
                            "the mounted transcript has nothing to scroll ({:.0} of {:.0})",
                            adj.page_size(),
                            adj.upper()
                        ));
                    }
                    // A fresh mount is pinned to the newest turn, so the only
                    // direction with room is up; the second notch comes back
                    // down and proves the clamp is the reason, not the path.
                    at.set(adj.value());
                    if at.get() < 400.0 {
                        fail(format!("the view is only {:.0}px from the top", at.get()));
                    }
                    // The handler is only half of it. GDK drops an event at the
                    // GdkWindow when that window's event mask does not select the
                    // event type, so a catcher whose window never asked for the
                    // scroll masks has a perfectly good handler that no real
                    // pointer can reach. `propagate_event` - what this harness
                    // uses - delivers to the widget and skips that filter, which
                    // is how the padding test stayed green while the user's wheel
                    // did nothing. Assert the mask, not just the behaviour.
                    let want = gtk::gdk::EventMask::SCROLL_MASK
                        | gtk::gdk::EventMask::SMOOTH_SCROLL_MASK;
                    for (label, hits) in [
                        ("the pane catcher", &views.conversation.transcript_hits),
                        ("the page catcher", &views.conversation.page_hits),
                    ] {
                        let Some(window) = hits.window() else {
                            fail(format!("{label} has no GdkWindow to select events on"));
                        };
                        let have = window.events();
                        if !have.contains(want) {
                            fail(format!(
                                "{label}'s window selects {have:?}, which is missing \
                                 SCROLL_MASK/SMOOTH_SCROLL_MASK: a real wheel over it is \
                                 dropped by GDK before any widget sees it"
                            ));
                        }
                    }
                    selftest::scroll_wheel(&views.conversation.transcript_hits, -400.0);
                    phase.set(2);
                }
                2 if since.get() >= 1400 => {
                    let up = value();
                    if (up - (at.get() - 400.0)).abs() > 8.0 {
                        fail(format!(
                            "a wheel event over the padding moved the transcript {:.0}px, not -400px",
                            up - at.get()
                        ));
                    }
                    // The bar thickens while the transcript moves (spec §10-39), so
                    // the wheel that just moved it has to have left the class
                    // behind. Without this the dim rule could silently stop
                    // matching and the only evidence would be a user calling the
                    // bar invisible.
                    if !views
                        .conversation
                        .transcript_bar
                        .style_context()
                        .has_class("eb-bar-active")
                    {
                        fail("scrolling left the bar in its resting state".into());
                    }
                    selftest::scroll_wheel(&views.conversation.transcript_hits, 250.0);
                    let down = value();
                    if (down - (up + 250.0)).abs() > 8.0 {
                        fail(format!(
                            "scrolling back down over the padding moved the transcript {:.0}px, not 250px",
                            down - up
                        ));
                    }
                    // Third notch on the *page* catcher, which covers the band the
                    // transcript's own catcher stops above: the composer card is
                    // centred in the pane, so its left and right margins are blank
                    // pixels too, and a wheel there belongs to nothing else.
                    selftest::scroll_wheel(&views.conversation.page_hits, -300.0);
                    let page = value();
                    if (page - (down - 300.0)).abs() > 8.0 {
                        fail(format!(
                            "a wheel event over the composer band moved the transcript {:.0}px, \
                             not -300px",
                            page - down
                        ));
                    }
                    println!(
                        "pad-scroll: ok the pane scrolls {:.0} -> {:.0} -> {:.0} -> {:.0} of {:.0}",
                        at.get(),
                        up,
                        down,
                        page,
                        views.conversation.scroll.vadjustment().upper()
                    );
                    application.quit();
                    return gtk::glib::ControlFlow::Break;
                }
                _ => {}
            }
            gtk::glib::ControlFlow::Continue
        });
    }
    // Task-switch harness. Three tasks stream at once while the user clicks
    // between them, which is the shape of the report "switch a few times
    // between running tasks and the window hangs". Three invariants:
    //   * the message list holds exactly the turns the conversation says it has
    //     mounted (a switch that leaves frames behind grows the tree every
    //     time), and that window stays inside the painted task's turns,
    //   * the main loop keeps meeting its own 120ms deadline, i.e. one switch
    //     never costs more than a frame budget — the switch that rebuilds a long
    //     transcript inline is what starved it, and
    //   * the frames a switch re-attaches are drawn: visible, with a real
    //     allocation, and optionally captured with `EB_SWITCH_SHOT=<path>`.
    // Pair with EB_FAKE_AGENT; EB_SWITCH_TEST=<switches> defaults to 30.
    // The switch targets are the three heaviest conversations on screen. With
    // an empty state file the harness creates them itself (three streaming
    // tasks); point EB_STATE at a saved tasks.json with three long
    // conversations and the same clicks drive those instead, which is how the
    // report is reproduced at production transcript sizes. Note the state only
    // loads when no selftest flag is set, so use EB_SWITCH_SHOT rather than
    // EB_SHOT for the capture here.
    if std::env::var_os("EB_SWITCH_TEST").is_some() {
        let switches: usize = std::env::var("EB_SWITCH_TEST")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(30);
        let seeded = store.borrow().tasks.len() >= 3;
        if seeded {
            // A follow-up turn inside each long conversation: the transcript is
            // heavy *and* its tail is live, so every switch has a streaming
            // conversation to repaint — the reported situation.
            for (delay, index) in [(150u64, 0usize), (300, 1), (450, 2)] {
                let agent = views.agent.clone();
                let open_task = views.open_task.clone();
                gtk::glib::timeout_add_local_once(Duration::from_millis(delay), move || {
                    open_task(index);
                    agent.submit(format!("follow-up question {index}"));
                });
            }
        } else {
            views.agent.submit(DEMO_PROMPT.to_string());
            for (delay, prompt) in [
                (150u64, "second streaming task"),
                (300, "third streaming task"),
            ] {
                let agent = views.agent.clone();
                let text = prompt.to_string();
                gtk::glib::timeout_add_local_once(Duration::from_millis(delay), move || {
                    agent.submit_as_new(text);
                });
            }
        }
        let open_task = views.open_task.clone();
        let messages = views.conversation.messages.clone();
        let sidebar_rows = views.sidebar.content.clone();
        let detail = views.detail.root.clone();
        let painted_cell = views.conversation.painted_task.clone();
        let window_probe = {
            let views = views.clone();
            move || views.conversation.window()
        };
        let shot_views = views.clone();
        let store = store.clone();
        let application = application.clone();
        let started = std::time::Instant::now();
        let mut done = 0usize;
        let mut worst = 0u64;
        let mut worst_switch = 0u64;
        let mut first_tree = 0usize;
        let mut tree = 0usize;
        gtk::glib::timeout_add_local(Duration::from_millis(120), move || {
            done += 1;
            // Lateness of this tick is how long the main loop was busy: the
            // refresh a switch triggers runs inline on the same thread.
            let target = 450 + done as u64 * 120;
            let late = started
                .elapsed()
                .as_millis()
                .saturating_sub(target as u128) as u64;
            worst = worst.max(late);
            let count = store.borrow().tasks.len();
            let ready = count >= if seeded { 3 } else { 6 };
            let base = if seeded { 0 } else { count.saturating_sub(3) };
            let index = (base + done % 3).min(count.saturating_sub(1));
            let switch_started = std::time::Instant::now();
            open_task(index);
            let switch_ms = switch_started.elapsed().as_millis() as u64;
            if ready {
                worst_switch = worst_switch.max(switch_ms);
            }
            let app = store.borrow();
            let turns = app
                .tasks
                .get(index)
                .map(|task| task.turns.len())
                .unwrap_or(0);
            let children = messages.children().len();
            let (window_first, mounted) = window_probe();
            let painted = app
                .tasks
                .iter()
                .map(|task| {
                    task.turns
                        .iter()
                        .map(|turn| match turn {
                            model::Turn::User { .. } => 'U',
                            model::Turn::Agent { .. } => 'A',
                        })
                        .collect::<String>()
                })
                .collect::<Vec<String>>()
                .join("|");
            drop(app);
            tree = count_widgets(&sidebar_rows.clone().upcast())
                + count_widgets(&detail.clone().upcast())
                + count_widgets(&messages.clone().upcast());
            if first_tree == 0 {
                first_tree = tree;
            }
            if std::env::var_os("EB_DEBUG").is_some() {
                eprintln!(
                    "switch: {done} task={index} ready={ready} late={late}ms switch={switch_ms}ms \
                     children={children} turns={turns} window={window_first}..{mounted} \
                     tree={tree} painted_now={:?} painted={painted}",
                    painted_cell.get()
                );
            }
            // The repaint a switch triggers may land on the next tick, so the
            // invariant is stated against the task that says it is painted: the
            // container holds exactly the frames the render state claims, and
            // that window is still inside the transcript.
            if let Some(painted_index) = painted_cell.get() {
                if children != mounted {
                    eprintln!(
                        "switch-test: FAIL switch {done} left {children} message children while \
                         the conversation mounted {mounted} turns"
                    );
                    std::process::exit(1);
                }
                let painted_turns = store
                    .borrow()
                    .tasks
                    .get(painted_index)
                    .map(|task| task.turns.len())
                    .unwrap_or(usize::MAX);
                if window_first.saturating_add(mounted) > painted_turns {
                    eprintln!(
                        "switch-test: FAIL switch {done} renders turns {window_first}.. of task \
                         {painted_index}, which has {painted_turns} turns"
                    );
                    std::process::exit(1);
                }
            }
            if done >= switches {
                if worst_switch > 600 {
                    eprintln!(
                        "switch-test: FAIL one switch cost {worst_switch}ms of UI-thread time \
                         during {switches} switches"
                    );
                    std::process::exit(1);
                }
                if worst > 600 {
                    eprintln!(
                        "switch-test: FAIL the main loop starved by {worst}ms during {switches} switches"
                    );
                    std::process::exit(1);
                }
                let summary = (worst, worst_switch, first_tree, tree);
                // The last switch needs a frame of its own before its
                // re-attached frames can be judged: GTK lays out and draws
                // between main-loop iterations, so reading an allocation from
                // this callback would read the previous frame's tree.
                let messages = messages.clone();
                let views = shot_views.clone();
                let application = application.clone();
                let shot = std::env::var("EB_SWITCH_SHOT").ok();
                gtk::glib::timeout_add_local_once(Duration::from_millis(300), move || {
                    let (worst, worst_switch, first_tree, tree) = summary;
                    let frames = messages.children();
                    let drawn = frames
                        .iter()
                        .filter(|frame| {
                            frame.is_visible()
                                && frame.allocation().width() > 0
                                && frame.allocation().height() > 0
                        })
                        .count();
                    if drawn != frames.len() {
                        eprintln!(
                            "switch-test: FAIL {} of {} message frames are attached but not drawn",
                            frames.len() - drawn,
                            frames.len()
                        );
                        std::process::exit(1);
                    }
                    match shot {
                        Some(path) => {
                            selftest::snapshot(&views, &path);
                            println!(
                                "switch-test: finished switches={switches} worst_late={worst}ms \
                                 worst_switch={worst_switch}ms tree={first_tree}->{tree} \
                                 frames={drawn} window={:?} shot={path}",
                                views.conversation.window()
                            );
                        }
                        None => println!(
                            "switch-test: finished switches={switches} worst_late={worst}ms \
                             worst_switch={worst_switch}ms tree={first_tree}->{tree} \
                             frames={drawn} window={:?}",
                            views.conversation.window()
                        ),
                    }
                    application.quit();
                });
                return gtk::glib::ControlFlow::Break;
            }
            gtk::glib::ControlFlow::Continue
        });
    }

    // Hang harness. The dsh bridge is the one remote the GTK main thread talks
    // to inline (session create/begin/finish around every submit), so a bridge
    // that accepts a connection and then goes silent used to freeze the window
    // forever. This squats on the bridge address with a black-hole listener,
    // runs two turns, and asserts each one still reaches its terminal event
    // inside its socket budget. Pair with EB_FAKE_AGENT=1 and point
    // EVOBUDDY_BRIDGE_ADDR at a free port so a live bridge is not in the way.
    if std::env::var_os("EB_HANG_TEST").is_some() {
        let target = std::env::var("EVOBUDDY_BRIDGE_ADDR")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                eprintln!("hang-test: FAIL set EVOBUDDY_BRIDGE_ADDR=127.0.0.1:<free port>");
                std::process::exit(1);
            });
        let listener = match std::net::TcpListener::bind(&target) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("hang-test: FAIL cannot squat {target}: {error}");
                eprintln!("hang-test: choose a port the running dsh bridge does not hold");
                std::process::exit(1);
            }
        };
        let held: std::sync::Arc<std::sync::Mutex<Vec<std::net::TcpStream>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        {
            let held = held.clone();
            std::thread::spawn(move || {
                use std::io::Read;
                for stream in listener.incoming().flatten() {
                    let mut stream = stream;
                    let mut buffer = [0u8; 4096];
                    let _ = stream.read(&mut buffer);
                    // Keep the socket open and unanswered: closing it would hand
                    // the client an instant EOF instead of a read timeout.
                    held.lock().unwrap().push(stream);
                }
            });
        }
        let started = std::time::Instant::now();
        let turn_started = Rc::new(std::cell::RefCell::new(started));
        let samples = Rc::new(std::cell::RefCell::new(Vec::<Duration>::new()));
        let stage = Rc::new(std::cell::Cell::new(0usize));
        let probe_views = views.clone();
        let probe_store = store.clone();
        let probe_application = application.clone();
        let probe_turn = turn_started.clone();
        let probe_samples = samples.clone();
        let probe_stage = stage.clone();
        // Both submits block the main thread inside the bridge call, which is
        // exactly what is being measured; the first one therefore happens here
        // rather than from the poll, so the samples line up with turns.
        views.agent.submit(DEMO_PROMPT.to_string());
        gtk::glib::timeout_add_local(Duration::from_millis(50), move || {
            if started.elapsed() > Duration::from_secs(20) {
                eprintln!("hang-test: FAIL the app never recovered; a main-thread call is wedged");
                std::process::exit(1);
            }
            if probe_views.agent.is_busy() {
                return gtk::glib::ControlFlow::Continue;
            }
            let elapsed = probe_turn.borrow().elapsed();
            probe_samples.borrow_mut().push(elapsed);
            if probe_stage.get() == 0 {
                probe_stage.set(1);
                *probe_turn.borrow_mut() = std::time::Instant::now();
                probe_views.agent.submit(DEMO_PROMPT.to_string());
                return gtk::glib::ControlFlow::Continue;
            }
            let samples = probe_samples.borrow().clone();
            let (first, second) = (samples[0], samples[1]);
            let mut problems: Vec<String> = Vec::new();
            if first > Duration::from_secs(5) {
                problems.push(format!("the first turn took {first:?} against a silent bridge"));
            }
            if second > Duration::from_secs(3) || second > first {
                problems.push(format!(
                    "the second turn took {second:?} against the same silent bridge (first took {first:?}); the degraded cooldown should skip it"
                ));
            }
            let app = probe_store.borrow();
            match app.tasks.last().and_then(|task| task.turns.last()) {
                Some(model::Turn::Agent {
                    streaming, blocks, ..
                }) if !*streaming && !blocks.is_empty() => {}
                _ => problems.push("the second turn never reached a finished answer".to_string()),
            }
            if !problems.is_empty() {
                for problem in &problems {
                    eprintln!("hang-test: FAIL {problem}");
                }
                std::process::exit(1);
            }
            println!("hang-test: recovered first={first:?} second={second:?}");
            probe_application.quit();
            gtk::glib::ControlFlow::Break
        });
    }

    // GTK owns the widgets, but `Views` is the Rust-side tree that the refresh
    // callbacks upgrade from a Weak. If this Rc is dropped when `build_ui`
    // returns, every later agent/UI update silently finds no view to repaint.
    // Keep the app-lifetime model alive explicitly; the process exit is the
    // natural drop point.
    if std::env::var_os("EB_SETTINGS").is_some() {
        settings::open();
    }
    #[cfg(target_os = "macos")]
    if let Ok(index) = std::env::var("EB_THEME_HOST") {
        theme_view::open(&index);
    }

    std::mem::forget(views);
}

/// One flick's window transition: `before` and `after` are (first, mounted), and
/// `children` is what the message list actually holds. Returns the reason the
/// transition was not exactly one clean slice.
fn slice_imbalance(
    before: (usize, usize),
    after: (usize, usize),
    children: usize,
    step: usize,
) -> Option<String> {
    let (was_first, was_mounted) = before;
    let (now_first, now_mounted) = after;
    let earlier = was_first.saturating_sub(now_first);
    let added = now_mounted.saturating_sub(was_mounted);
    if earlier != step {
        return Some(format!(
            "{was_first}+{was_mounted} -> {now_first}+{now_mounted}: {earlier} turns came down \
             from above the window, expected {step}"
        ));
    }
    if added != earlier {
        return Some(format!(
            "{was_first}+{was_mounted} -> {now_first}+{now_mounted}: {earlier} turns unrendered \
             above, but {added} turns mounted"
        ));
    }
    if children != now_mounted {
        return Some(format!(
            "the container holds {children} children for {now_mounted} mounted turns"
        ));
    }
    None
}

/// Every widget below `root`, including itself. The switch harness uses it to
/// catch a repaint that leaves a pane's tree bigger than the state it renders.
fn count_widgets(root: &gtk::Widget) -> usize {
    let mut count = 1;
    if let Ok(container) = root.clone().downcast::<gtk::Container>() {
        for child in container.children() {
            count += count_widgets(&child);
        }
    }
    count
}
