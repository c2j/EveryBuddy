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
mod composer;
mod conversation;
mod detail;
mod host;
mod model;
mod selftest;
mod shell;
mod sidebar;
mod spec;
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
    ensure_gtk_im_modules();

    let application = gtk::Application::new(Some(spec::APP_ID), Default::default());
    application.connect_startup(|_| load_css());
    application.connect_activate(build_ui);
    application.run();
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
    // Mount the plugins first: the window is rendered from their contributions.
    let host = host::Host::mount();

    let store = model::store(model::App::new());
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
        views.agent.submit(DEMO_PROMPT.to_string());
        views.refresh(&store);
    }

    // ACP harness: submit and leave the window open so a delayed screenshot
    // (`EB_SHOT_AFTER_MS`) can catch the real stream after it settles.
    if std::env::var_os("EB_ACP_DEMO").is_some() {
        views.agent.submit(DEMO_PROMPT.to_string());
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
}
