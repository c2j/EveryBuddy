//! Smoke test, run with `EB_SELFTEST=1`.
//!
//! GTK reports neither a CSS parse failure nor a mistyped `@define-color` name:
//! a stylesheet with one bad line is dropped wholesale and the app silently
//! falls back to the default theme, which looks plausible enough to pass a
//! visual check. And a colour can resolve correctly while being applied to the
//! wrong node. So the test does two separate things:
//!
//!   * resolve the colour names the shell depends on, and
//!   * print the geometry each probed widget actually ended up with, to compare
//!     against the captured numbers in `docs/ui-spec/ui-spec.md`.
//!
//! Set `EB_SHOT=<path>` to also render the window to a PNG, which is the only
//! check that catches a colour on the wrong node or text clipped by a too-small
//! allocation. `EB_DEMO=1` submits a message first, so the conversation state can
//! be photographed too.
//!
//! `EB_ASSERT=1` turns the printed geometry report into an exit-code check (the
//! CI verify scripts use it), and `EB_CLICK_ASSERT=1` does the same for the
//! click audit's stable observations. Both skip values that move with the window
//! server, fonts or layout timing.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gtk::glib::translate::{ToGlibPtr, ToGlibPtrMut};
use gtk::prelude::*;

use crate::shell::{Probes, Views};
use crate::spec;
/// Colour names the shell relies on.
///
/// The first five come from the generated token set; the last three are the
/// component-level alpha values measured off the live renderer, which are not in
/// `tokens.css` and so are defined locally in `ui/gtk.css`.
const COLOURS: [&str; 8] = [
    "bg_secondary",
    "bg_tertiary",
    "bg_hover_light",
    "bg_card",
    "border_strong",
    "eb_text_primary",
    "eb_text_secondary",
    "eb_text_tertiary",
];

/// Whether any harness flag is set.
///
/// The harness drives everything from the first allocation, so *every* `EB_*`
/// flag needs it attached — the flags used to require `EB_SELFTEST=1` alongside
/// them, which silently did nothing when it was left out.
pub fn requested() -> bool {
    [
        "EB_SELFTEST",
        "EB_SHOT",
        "EB_ENTER",
        "EB_CLICK",
        "EB_CLICK_ASSERT",
        "EB_ASSERT",
        "EB_DEMO",
        "EB_DEMO_BLOCKS",
        "EB_ACP_DEMO",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some())
}

pub fn attach(views: &Rc<Views>, probes: &Probes, application: &gtk::Application) {
    let window = views.window.clone();
    let probes: Vec<(String, gtk::Widget)> = probes
        .iter()
        .map(|(name, widget)| ((*name).to_string(), widget.clone()))
        .collect();
    let shot = std::env::var_os("EB_SHOT").map(|path| path.to_string_lossy().into_owned());
    let shot_delay = std::env::var("EB_SHOT_AFTER_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok());

    // ---- the geometry report, once the first frame has been allocated ----
    let reported = Rc::new(Cell::new(false));
    {
        let views = views.clone();
        let probes = probes.clone();
        let application = application.clone();
        window.connect_size_allocate(move |window, _| {
            // Fires again on every resize; only the first allocation matters.
            if reported.replace(true) {
                return;
            }
            // A widget receives ::size-allocate *before* its children are
            // allocated, so acting here reads a half-built tree: child
            // allocations are still stale. Defer to an idle callback, by which
            // point the frame has settled.
            let window = window.clone();
            let views = views.clone();
            let probes = probes.clone();
            let application = application.clone();
            gtk::glib::idle_add_local_once(move || {
                report(&window, &views, &probes);
                if enter_requested() {
                    // The composer defers its repaint through an idle, so the
                    // route only changes after this callback returns. Report
                    // from a further idle rather than blocking here —
                    // `test_widget_wait_for_draw` would wait for a frame that
                    // cannot be produced while the main loop is blocked.
                    let editor = enter_test(&views);
                    let views = views.clone();
                    let application = application.clone();
                    gtk::glib::idle_add_local_once(move || {
                        let detail = match &editor {
                            Some(editor) => format!("buffer={:?}", text_of(editor)),
                            None => "no eb-editor text view found".into(),
                        };
                        println!(
                            "enter: after={} (want conversation), {detail}",
                            views
                                .stack
                                .visible_child_name()
                                .map(|name| name.to_string())
                                .unwrap_or_else(|| "<none>".into())
                        );
                        application.quit();
                    });
                    return;
                }
                if click_requested() {
                    // The audit is a chain of click/observe pairs; each
                    // observation has to wait for the deferred repaint, so it
                    // drives itself through idles and quits at the end.
                    start_click_audit(&views, &application);
                    return;
                }
                if shot_requested() {
                    return;
                }
                application.quit();
            });
        });
    }

    // ---- the screenshot, on the first real paint ----
    //
    // `gtk_widget_draw` checks that the widget is *drawable*, which is only true
    // once the compositor has mapped and painted the toplevel. Drawing earlier
    // writes an empty — but perfectly well-formed — surface, so a timer is not
    // enough to make this deterministic. A paint of the window proves the
    // condition, so the first one schedules the capture.
    let Some(path) = shot else {
        return;
    };
    let captured = Rc::new(Cell::new(false));
    let application = application.clone();
    let views = views.clone();
    window.connect_draw(move |_, _| {
        if !captured.replace(true) {
            let views = views.clone();
            let path = path.clone();
            let application = application.clone();
            if let Some(delay) = shot_delay {
                gtk::glib::timeout_add_local_once(
                    std::time::Duration::from_millis(delay),
                    move || {
                        snapshot(&views, &path);
                        application.quit();
                    },
                );
            } else {
                gtk::glib::idle_add_local_once(move || {
                    snapshot(&views, &path);
                    application.quit();
                });
            }
        }
        gtk::glib::Propagation::Proceed
    });
}

fn shot_requested() -> bool {
    std::env::var_os("EB_SHOT").is_some()
}

fn enter_requested() -> bool {
    std::env::var_os("EB_ENTER").is_some()
}

fn click_requested() -> bool {
    std::env::var_os("EB_CLICK").is_some() || click_assert_requested()
}

/// `EB_ASSERT=1` turns the geometry/colour report into an exit-code assertion.
fn assert_requested() -> bool {
    std::env::var_os("EB_ASSERT").is_some()
}

/// `EB_CLICK_ASSERT=1` asserts the audit's stable observations, not just prints.
fn click_assert_requested() -> bool {
    std::env::var_os("EB_CLICK_ASSERT").is_some()
}

/// `EB_CLICK_SIGNAL=1` activates controls by emitting `clicked` instead of
/// synthesising pointer events, to tell "the handler is missing" apart from
/// "the event never arrived".
fn signal_only() -> bool {
    std::env::var_os("EB_CLICK_SIGNAL").is_some()
}

// ------------------------------------------------------------- the audit
//
// Every control the UI draws is activated in turn and the state it is supposed
// to move is read back afterwards. A control that reports "no change" is either
// unwired or unreachable, which is exactly the question this answers — the
// duplicate cue in the output is deliberate, since only an observable change
// proves a click did anything.

struct Step {
    label: String,
    activate: Rc<dyn Fn(&Views)>,
    observe: Rc<dyn Fn(&Views) -> String>,
}

/// Counted across the audit; `EB_CLICK_ASSERT=1` turns a non-zero count into a
/// failing exit status at the end.
static CLICK_FAILURES: AtomicUsize = AtomicUsize::new(0);

/// The observations that must hold for each step. Deliberately *not* the whole
/// line: toplevel counts, popover coordinates and the code pane's measured
/// scroll width move with the window server, fonts and layout timing.
fn expected_click(label: &str) -> &'static [&'static str] {
    match label {
        "chip[0]" => &["Help me plan and build a small desktop utility."],
        "segmented 'Code'" => &["active segments: [1]"],
        "nav[1] (second row)" => &["route=unbuilt"],
        "nav[0] (New Task)" => &["route=welcome"],
        "sidebar collapse" => &["sidebar visible=false", "reveal visible=true"],
        "sidebar reveal" => &["sidebar visible=true", "reveal visible=false"],
        "sidebar search" => &["menu open: visible=true", "notice=\"Global search"],
        "sidebar mini programs" => &["notice=\"Mini programs"],
        "sidebar filter menu" => &["3 of 3 menu rows visible"],
        "composer '+' menu" => &["4 of 4 menu rows visible"],
        "model selector menu" => &["5 of 5 menu rows visible"],
        "Enter (submit via chip text)" => &["route=conversation", "Ask anything"],
        "chat search" => &["notice=\"Searching inside a task"],
        "chat share" => &["notice=\"Sharing a task"],
        "artifact expand" => &["detail visible=true"],
        "artifact expand (back)" => &["detail visible=true"],
        "artifact tab" => &["detail visible=true"],
        "artifact new tab" => &["notice=\"Only the one artifact"],
        "artifact tab close (x)" => &["detail visible=false"],
        "artifact panel close (header)" => &["detail visible=false"],
        "task row (reopens panel)" => &["detail visible=true"],
        "copy message" => &["Copied"],
        "section label (Tasks)" => &["GtkButton:hidden"],
        "section label (expand again)" => &["GtkButton:shown"],
        "user row menu" => &["4 of 4 menu rows visible"],
        "send button (empty input)" => &["route=conversation"],
        _ => &[],
    }
}

fn assert_click_observation(label: &str, observed: &str) {
    if !click_assert_requested() {
        return;
    }
    let expected = expected_click(label);
    if expected.is_empty() {
        eprintln!("click: ASSERT FAILED: no expectation for step {label:?}");
        CLICK_FAILURES.fetch_add(1, Ordering::SeqCst);
        return;
    }
    let missing: Vec<&str> = expected
        .iter()
        .copied()
        .filter(|needle| !observed.contains(needle))
        .collect();
    if missing.is_empty() {
        return;
    }
    eprintln!("click: ASSERT FAILED: {label}: missing {missing:?} in {observed:?}");
    CLICK_FAILURES.fetch_add(1, Ordering::SeqCst);
}

fn start_click_audit(views: &Rc<Views>, application: &gtk::Application) {
    let steps: Vec<Step> = build_steps();
    println!("click: --- audit ({} steps) ---", steps.len());
    if click_assert_requested() && steps.len() != 26 {
        eprintln!(
            "click: ASSERT FAILED: expected 26 steps, the audit now has {}",
            steps.len()
        );
        CLICK_FAILURES.fetch_add(1, Ordering::SeqCst);
    }
    // A user's window is active; a bare `cargo run` has nothing to activate it,
    // and several GTK paths behave differently for an inactive window, so ask
    // for activation explicitly before judging what does not respond.
    views.window.present();
    if let Some(window) = views.window.window() {
        window.focus(gtk::gdk::ffi::GDK_CURRENT_TIME as u32);
    }
    println!(
        "click: at rest: {} window active={} mapped={}",
        popover_state(views),
        views.window.is_active(),
        views.window.is_mapped(),
    );
    let views = views.clone();
    let application = application.clone();
    gtk::glib::idle_add_local_once(move || {
        println!("click: after present(): window active={}", views.window.is_active());
        drive(views, Rc::new(steps), Rc::new(Cell::new(0)), application);
    });
}

fn drive(
    views: Rc<Views>,
    steps: Rc<Vec<Step>>,
    index: Rc<Cell<usize>>,
    application: gtk::Application,
) {
    let position = index.get();
    if position >= steps.len() {
        println!("click: --- end ---");
        let failures = CLICK_FAILURES.load(Ordering::SeqCst);
        if click_assert_requested() {
            if failures == 0 {
                println!("click: assertions passed ({} steps)", steps.len());
            } else {
                eprintln!("click: {failures} assertion(s) failed");
                application.quit();
                std::process::exit(1);
            }
        }
        application.quit();
        return;
    }
    let step = &steps[position];
    let (label, activate, observe) = (step.label.clone(), step.activate.clone(), step.observe.clone());
    activate(&views);

    let views = views.clone();
    let steps = steps.clone();
    let index = index.clone();
    let application = application.clone();
    gtk::glib::idle_add_local_once(move || {
        let observed = observe(&views);
        println!("click: {label:<34} {observed}");
        assert_click_observation(&label, &observed);
        index.set(position + 1);
        drive(views, steps, index, application);
    });
}

fn observe_route(views: &Views) -> String {
    format!(
        "route={}",
        views
            .stack
            .visible_child_name()
            .map(|name| name.to_string())
            .unwrap_or_else(|| "<none>".into())
    )
}

fn observe_text(views: &Views) -> String {
    let text = text_of(&views.welcome_composer.view);
    format!("composer={text:?}")
}

fn observe_sidebar(views: &Views) -> String {
    // The reveal button's glyph is the check that `hide_from_show_all` worked:
    // a `no_show_all` button whose glyph was never shown renders as an empty
    // square, which is exactly what "partially broken UI" looks like.
    let glyph = views
        .welcome
        .reveal
        .child()
        .and_then(|child| child.downcast::<gtk::Label>().ok())
        .map(|label| {
            format!(
                "glyph {:?} visible={}",
                label.text().to_string(),
                label.is_visible()
            )
        })
        .unwrap_or_else(|| "no glyph child".into());
    format!(
        "sidebar visible={} reveal visible={} ({glyph})",
        views.sidebar.root.is_visible(),
        views.welcome.reveal.is_visible()
    )
}

fn observe_popover(views: &Views) -> String {
    popover_state(views)
}

/// What the dropdowns are doing.
///
/// A `GtkPopover` is not a separate toplevel window — GTK parents it to the
/// window of the widget it is anchored to and draws it there — so the widget
/// walk cannot see one either (it is only added to the tree when it pops up).
/// The input grab and the popover's own state are what can be observed.
fn popover_state(views: &Views) -> String {
    // Two fields of this line move with the environment, not with the code:
    // the toplevel *count* — macOS pre-creates a hidden GtkTooltipWindow in
    // some sessions and not others — and the popover's *absolute* position,
    // which includes the window origin the window server chooses. Compare
    // steps and notice copy across runs; do not diff this line byte for byte.
    //
    // A popover is allocated by the window's own layout pass, which the stalled
    // frame clock of a headless run never delivers — the same reason the
    // screenshot path re-runs allocation before drawing. Do that first, so the
    // size below is the size a real frame would give it.
    views.window.size_allocate(&views.window.allocation());

    let toplevels = gtk::Window::list_toplevels();
    let visible = toplevels.iter().filter(|window| window.is_visible()).count();
    // A hidden toplevel is exactly the kind of thing that only shows up as a
    // count otherwise; name every window so the audit line explains itself.
    let names = toplevels
        .iter()
        .map(|window| {
            format!(
                "{}{}",
                window.type_().name(),
                if window.is_visible() { "" } else { "(hidden)" }
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let state = match gtk::grab_get_current() {
        Some(widget) => match widget.clone().downcast::<gtk::Popover>() {
            Ok(popover) => {
                let position = popover
                    .translate_coordinates(&views.window, 0, 0)
                    .map(|(x, y)| format!("at {x},{y}"))
                    .unwrap_or_else(|| "at ?".into());
                let mut inside = Vec::new();
                walk(&popover.clone().upcast::<gtk::Widget>(), &mut inside);
                let label = inside
                    .iter()
                    .filter(|child| child.style_context().has_class("eb-missing-label"))
                    .filter_map(|child| child.clone().downcast::<gtk::Label>().ok())
                    .map(|label| label.text().to_string())
                    .next();
                let body = match label {
                    Some(text) => format!("notice={text:?}"),
                    None => {
                        let rows = inside
                            .iter()
                            .filter(|child| child.style_context().has_class("eb-menu-item"))
                            .collect::<Vec<_>>();
                        let shown = rows.iter().filter(|child| child.is_visible()).count();
                        format!("{shown} of {} menu rows visible", rows.len())
                    }
                };
                format!(
                    "menu open: visible={} mapped={} alloc={}x{} {position}, {body}",
                    popover.is_visible(),
                    popover.is_mapped(),
                    popover.allocation().width(),
                    popover.allocation().height(),
                )
            }
            Err(_) => format!("grab held by {}", widget.type_().name()),
        },
        None => "no menu open, no grab".to_string(),
    };
    format!("toplevels={visible}/{} [{names}]; {state}", toplevels.len())
}

/// Which of the segmented control's two items carries the "selected" class.
fn observe_segment(views: &Views) -> String {
    let marked: Vec<usize> = widgets(views)
        .into_iter()
        .filter(|widget| widget.style_context().has_class("eb-segment"))
        .enumerate()
        .filter(|(_, widget)| widget.style_context().has_class("eb-segment-active"))
        .map(|(index, _)| index)
        .collect();
    format!("active segments: {marked:?}")
}

fn observe_detail(views: &Views) -> String {
    // The code view is a non-wrapping `GtkTextView` inside a scroller whose
    // natural width is deliberately not propagated, so the only way to tell
    // "scrolls sideways" from "clips" is the horizontal adjustment: `upper`
    // above `page_size` means there is content off to the right to scroll to.
    let code = widgets(views)
        .into_iter()
        .find(|widget| widget.style_context().has_class("eb-code-scroll"))
        .and_then(|widget| widget.downcast::<gtk::ScrolledWindow>().ok())
        .map(|scroll| {
            let horizontal = scroll.hadjustment();
            format!(
                "code scroll {:.0} of {:.0} wide",
                horizontal.page_size(),
                horizontal.upper()
            )
        })
        .unwrap_or_else(|| "no code view".into());
    format!(
        "detail visible={} maximised-pos={} tabs={} ({code})",
        views.detail.root.is_visible(),
        views.outer.position(),
        widgets(views)
            .into_iter()
            .filter(|widget| widget.style_context().has_class("eb-tab"))
            .count()
    )
}

/// The visibility chain from a task row up to the window.
///
/// `is_visible` is true only when every ancestor is visible too, so a row that
/// reports invisible has to say *where* the chain stops.
fn task_row_state(views: &Views) -> String {
    let rows: Vec<String> = widgets(views)
        .into_iter()
        .filter(|widget| widget.style_context().has_class("eb-task"))
        .map(|widget| {
            let mut chain = Vec::new();
            let mut node = Some(widget);
            while let Some(current) = node {
                chain.push(format!(
                    "{}:{}",
                    current.type_().name(),
                    if current.is_visible() { "shown" } else { "hidden" }
                ));
                node = current.parent();
            }
            chain.join(" < ")
        })
        .collect();
    if rows.is_empty() {
        "no task row at all".into()
    } else {
        rows.join(" | ")
    }
}

fn build_steps() -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();

    // ---- landing page -----------------------------------------------------
    steps.push(Step {
        label: "chip[0]".into(),
        activate: Rc::new(|views| {
            if let Some(chip) = nth_class(views, "eb-chip", 0) {
                click(&chip, &views.window);
            }
        }),
        observe: Rc::new(observe_text),
    });
    steps.push(Step {
        label: "segmented 'Code'".into(),
        activate: Rc::new(|views| {
            if let Some(button) = nth_class(views, "eb-segment", 1) {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_segment),
    });
    steps.push(Step {
        label: "nav[1] (second row)".into(),
        activate: Rc::new(|views| {
            if let Some(row) = nth_class(views, "eb-nav", 1) {
                click(&row, &views.window);
            }
        }),
        observe: Rc::new(observe_route),
    });
    steps.push(Step {
        label: "nav[0] (New Task)".into(),
        activate: Rc::new(|views| {
            if let Some(row) = nth_class(views, "eb-nav", 0) {
                click(&row, &views.window);
            }
        }),
        observe: Rc::new(observe_route),
    });
    steps.push(Step {
        label: "sidebar collapse".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_tooltip(views, "Hide sidebar") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_sidebar),
    });
    steps.push(Step {
        label: "sidebar reveal".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_tooltip(views, "Show sidebar") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_sidebar),
    });
    steps.push(Step {
        label: "sidebar search".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_tooltip(views, "Search") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_popover),
    });
    steps.push(Step {
        label: "sidebar mini programs".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_tooltip(views, "Mini programs") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_popover),
    });
    steps.push(Step {
        label: "sidebar filter menu".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_tooltip(views, "Filter tasks") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_popover),
    });
    steps.push(Step {
        label: "composer '+' menu".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_tooltip(views, "Add context") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_popover),
    });
    steps.push(Step {
        label: "model selector menu".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_tooltip(views, "Choose model") {
                click(&button, &views.window);
                if let Some(path) = std::env::var_os("EB_MENU_SHOT") {
                    snapshot(views, &path.to_string_lossy());
                }
            }
        }),
        observe: Rc::new(observe_popover),
    });

    // ---- into a task ------------------------------------------------------
    // Chip[0] left text in the composer, so Return now submits it and the
    // conversation route is reached the same way a user reaches it. Focus is
    // faked first, because a user can only press Return at all with the window
    // active and the editor focused.
    steps.push(Step {
        label: "Enter (submit via chip text)".into(),
        activate: Rc::new(|views| {
            focus_editor(&views.welcome_composer.view);
            send_return(&views.welcome_composer.view, &views.window);
        }),
        observe: Rc::new(|views| {
            format!(
                "{} composer={:?}",
                observe_route(views),
                text_of(&views.welcome_composer.view)
            )
        }),
    });
    steps.push(Step {
        label: "chat search".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_tooltip(views, "Search in task") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_popover),
    });
    steps.push(Step {
        label: "chat share".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_tooltip(views, "Share task") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_popover),
    });
    steps.push(Step {
        label: "artifact expand".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_tooltip(views, "Give the panel the whole window") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_detail),
    });
    steps.push(Step {
        label: "artifact expand (back)".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_tooltip(views, "Give the panel the whole window") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_detail),
    });
    steps.push(Step {
        label: "artifact tab".into(),
        activate: Rc::new(|views| {
            if let Some(tab) = by_class(views, "eb-tab") {
                click(&tab, &views.window);
            }
        }),
        observe: Rc::new(observe_detail),
    });
    steps.push(Step {
        label: "artifact new tab".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_tooltip(views, "New tab") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_popover),
    });
    steps.push(Step {
        label: "artifact tab close (x)".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_tooltip(views, "Close artifact") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_detail),
    });
    steps.push(Step {
        label: "artifact panel close (header)".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_tooltip(views, "Hide artifact panel") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_detail),
    });
    steps.push(Step {
        label: "task row (reopens panel)".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_class(views, "eb-task") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_detail),
    });
    steps.push(Step {
        label: "copy message".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_tooltip(views, "Copy message") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(|views| {
            let tooltips: Vec<String> = widgets(views)
                .into_iter()
                .filter(|widget| widget.style_context().has_class("eb-icon-mini"))
                .filter_map(|widget| widget.tooltip_text().map(|text| text.to_string()))
                .collect();
            format!("mini-button tooltips: {tooltips:?}")
        }),
    });
    steps.push(Step {
        label: "section label (Tasks)".into(),
        activate: Rc::new(|views| {
            println!("click:   before: {}", task_row_state(views));
            if let Some(button) = by_class(views, "eb-section-label") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(task_row_state),
    });
    steps.push(Step {
        label: "section label (expand again)".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_class(views, "eb-section-label") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(task_row_state),
    });
    steps.push(Step {
        label: "user row menu".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_class(views, "eb-user") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_popover),
    });
    steps.push(Step {
        label: "send button (empty input)".into(),
        activate: Rc::new(|views| {
            if let Some(button) = by_class(views, "eb-send") {
                click(&button, &views.window);
            }
        }),
        observe: Rc::new(observe_route),
    });

    steps
}

fn widgets(views: &Views) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    walk(&views.window.clone().upcast(), &mut found);
    found
}

fn walk(root: &gtk::Widget, found: &mut Vec<gtk::Widget>) {
    found.push(root.clone());
    if let Ok(container) = root.clone().downcast::<gtk::Container>() {
        for child in container.children() {
            walk(&child, found);
        }
    }
}

fn by_class(views: &Views, class_name: &str) -> Option<gtk::Widget> {
    widgets(views)
        .into_iter()
        .find(|widget| widget.style_context().has_class(class_name))
}

fn nth_class(views: &Views, class_name: &str, index: usize) -> Option<gtk::Widget> {
    widgets(views)
        .into_iter()
        .filter(|widget| widget.style_context().has_class(class_name))
        .nth(index)
}

fn by_tooltip(views: &Views, tooltip: &str) -> Option<gtk::Widget> {
    widgets(views).into_iter().find(|widget| {
        widget
            .tooltip_text()
            .map(|text| text == tooltip)
            .unwrap_or(false)
    })
}

/// Activate a widget the way a mouse would.
///
/// Both halves of the click are delivered to the widget itself. GTK's own
/// dispatch already picked the widget out of the hierarchy before it called the
/// handler, and `gtk_propagate_event` on a widget runs the same handlers in the
/// same order for every ancestor — so this reaches the same code a real press
/// does, sensitivity check included.
///
/// The event still needs a *viewable* window, because `gtk_widget_event` drops
/// any event whose window is missing or unmapped — see `send_return`.
fn click(widget: &gtk::Widget, toplevel: &impl IsA<gtk::Widget>) {
    if signal_only() {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
            button.clicked();
        }
        return;
    }
    let Some(window) = toplevel.window() else {
        eprintln!("click: the toplevel has no window, cannot synthesise a click");
        return;
    };
    // `gtk_widget_event` refuses an event for an unrealized widget, and a widget
    // that became visible this frame is only realized when the frame clock next
    // lays the window out. A real click can never arrive that early; a
    // synthesised one can, so realize first — exactly what that frame would do.
    if !widget.is_realized() {
        widget.realize();
    }
    let device = gtk::gdk::Display::default()
        .and_then(|display| display.default_seat())
        .and_then(|seat| seat.pointer());

    for kind in [
        gtk::gdk::EventType::ButtonPress,
        gtk::gdk::EventType::ButtonRelease,
    ] {
        let mut event = gtk::gdk::Event::new(kind);
        unsafe {
            let ptr: *mut gtk::gdk::ffi::GdkEvent = event.to_glib_none_mut().0;
            (*ptr).button.window = window.to_glib_none().0;
            // `gdk_event_free` releases the event's reference to the window.
            gtk::glib::gobject_ffi::g_object_ref((*ptr).button.window as *mut _);
            (*ptr).button.time = gtk::gdk::ffi::GDK_CURRENT_TIME as u32;
            (*ptr).button.send_event = 1;
            (*ptr).button.x = 1.0;
            (*ptr).button.y = 1.0;
            (*ptr).button.button = 1;
            (*ptr).button.state = 0;
            (*ptr).button.device = device
                .as_ref()
                .map(|device| device.to_glib_none().0)
                .unwrap_or(std::ptr::null_mut());
        }
        gtk::propagate_event(widget, &mut event);
    }
}

/// Drive Enter-to-send end to end.
///
/// Run with `EB_ENTER=1`. Two things have to happen before a synthesised Return
/// means anything:
///
/// * the composer has to stop showing its hint, which it does on focus-in — and
///   GTK only delivers focus-in to a widget whose toplevel is *active*
///   (`window_update_has_focus` gates on `is_active`). A run under `cargo run`
///   has no window manager to activate the window, so
///   `gtk_widget_send_focus_change` is called directly, exactly as
///   `gtk_window_real_set_focus` would.
/// * the key has to arrive at the editor as a real `GdkEventKey` whose window is
///   mapped — see `send_return` for why neither `gtk_test_widget_send_key` nor a
///   bare event works.
///
/// A route change proves the whole chain: the key reached the composer's
/// handler, the input-method filter it now delegates to declined the key, and
/// `submit()` ran. It cannot test the *composing* case — a real IME has to be
/// driving the composition for the input method to claim the key (ui-spec §10).
fn enter_test(views: &Views) -> Option<gtk::TextView> {
    let editor = find_editor(&views.welcome.root.clone().upcast())?;
    focus_editor(&editor);
    if let Some(buffer) = editor.buffer() {
        buffer.set_text("EB_ENTER probe");
    }
    println!(
        "enter: before={} hint-cleared={}",
        views
            .stack
            .visible_child_name()
            .map(|name| name.to_string())
            .unwrap_or_else(|| "<none>".into()),
        !editor.style_context().has_class("eb-placeholder"),
    );
    send_return(&editor, &views.window);
    Some(editor)
}

/// Hand a Return key press to the editor.
///
/// Built by hand rather than with `gtk::test_widget_send_key`, which resolves
/// the keymap through `gtk_widget_get_window` — and a text view is a windowless
/// widget, so sending to it bails out with `gdk_window_get_display: assertion
/// 'GDK_IS_WINDOW (window)' failed` and nothing is delivered.
///
/// The event still has to carry a *viewable* window: `gtk_widget_event` drops
/// key events whose window is missing or unmapped
/// (`event_window_is_still_viewable`), silently — the handler never runs. The
/// toplevel's window is used for that, while the event itself is delivered to
/// the editor, which is where the composer's handler lives.
fn send_return(editor: &gtk::TextView, toplevel: &impl IsA<gtk::Widget>) -> bool {
    let Some(window) = toplevel.window() else {
        eprintln!("enter: the toplevel has no window, cannot synthesise a key");
        return false;
    };
    let mut event = gtk::gdk::Event::new(gtk::gdk::EventType::KeyPress);
    unsafe {
        let ptr: *mut gtk::gdk::ffi::GdkEvent = event.to_glib_none_mut().0;
        (*ptr).key.window = window.to_glib_none().0;
        // `gdk_event_free` releases the event's reference to the window.
        gtk::glib::gobject_ffi::g_object_ref((*ptr).key.window as *mut _);
        (*ptr).key.keyval = (*gtk::gdk::keys::constants::Return) as u32;
        (*ptr).key.state = 0;
        (*ptr).key.time = gtk::gdk::ffi::GDK_CURRENT_TIME as u32;
    }
    gtk::propagate_event(editor, &mut event);
    true
}

/// Put focus on a widget the way a user's click does.
///
/// GTK only delivers focus-in to a widget of an *active* toplevel
/// (`window_update_has_focus` gates on it) and a bare `cargo run` has no window
/// manager to activate the window, so `gtk_widget_send_focus_change` stands in
/// for what `gtk_window_real_set_focus` would send.
fn focus_editor(editor: &gtk::TextView) {
    editor.grab_focus();
    unsafe {
        let widget: gtk::Widget = editor.clone().upcast();
        let event = gtk::gdk::ffi::gdk_event_new(gtk::gdk::ffi::GDK_FOCUS_CHANGE);
        (*event).focus_change.in_ = 1;
        gtk::ffi::gtk_widget_send_focus_change(widget.to_glib_none().0, event);
        gtk::gdk::ffi::gdk_event_free(event);
    }
}

fn text_of(view: &gtk::TextView) -> String {
    view.buffer()
        .and_then(|buffer| {
            buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .map(|text| text.to_string())
        })
        .unwrap_or_default()
}

/// Depth-first search for the composer's editor, which is the text view the
/// input method is attached to.
fn find_editor(root: &gtk::Widget) -> Option<gtk::TextView> {
    if let Ok(view) = root.clone().downcast::<gtk::TextView>() {
        return view
            .style_context()
            .has_class("eb-editor")
            .then_some(view);
    }
    let container = root.clone().downcast::<gtk::Container>().ok()?;
    container.children().into_iter().find_map(|child| find_editor(&child))
}

fn check(ok: bool, message: String, failures: &mut usize) {
    if ok {
        return;
    }
    eprintln!("selftest: ASSERT FAILED: {message}");
    *failures += 1;
}

/// Turn the printed report into an exit-code check with `EB_ASSERT=1`.
///
/// The fixed blocks are measured values, so they are asserted on every screen
/// that has room for them. A window clamped to a small CI workarea is expected
/// (the OS is right); only the screen-independent invariants are asserted there.
fn assert_geometry(
    window: &gtk::ApplicationWindow,
    views: &Views,
    probes: &[(String, gtk::Widget)],
) {
    if !assert_requested() {
        return;
    }

    let mut failures = 0usize;
    let allocation = window.allocation();
    let roomy = allocation.width() >= spec::WINDOW_WIDTH;
    let small_screen = gtk::gdk::Screen::default()
        .and_then(|screen| screen.root_window())
        .map(|screen| {
            screen.width() < spec::WINDOW_WIDTH || screen.height() < spec::WINDOW_HEIGHT + 32
        })
        .unwrap_or(false);

    check(
        !small_screen
            || (allocation.width() == spec::WINDOW_WIDTH
                && allocation.height() == spec::WINDOW_HEIGHT),
        format!(
            "window is {}x{}, want {}x{} on a screen with room",
            allocation.width(),
            allocation.height(),
            spec::WINDOW_WIDTH,
            spec::WINDOW_HEIGHT
        ),
        &mut failures,
    );
    check(
        views.inner.position() == spec::SIDEBAR_WIDTH,
        format!(
            "sash sidebar x={}, want {}",
            views.inner.position(),
            spec::SIDEBAR_WIDTH
        ),
        &mut failures,
    );

    // Welcome-only fixed blocks: asserted whenever they are visible.
    for (name, want_width, want_height) in [
        (
            "eb-composer-card",
            spec::WELCOME_CARD_WIDTH,
            spec::WELCOME_CARD_HEIGHT,
        ),
        ("eb-chips", spec::CHIPS_WIDTH, spec::CHIPS_HEIGHT),
    ] {
        let Some((_, widget)) = probes.iter().find(|(probe, _)| probe == name) else {
            check(false, format!("probe {name} is missing"), &mut failures);
            continue;
        };
        if !widget.is_visible() {
            continue;
        }
        let actual = widget.allocation();
        check(
            actual.width() == want_width && actual.height() == want_height,
            format!(
                "{name} is {}x{}, want {want_width}x{want_height}",
                actual.width(),
                actual.height()
            ),
            &mut failures,
        );
    }

    // Conversation-only blocks: the three-pane layout does not fit when the
    // window is clamped to a small screen, so only assert with room.
    if roomy {
        let detail_open = probes
            .iter()
            .find(|(probe, _)| probe == "eb-detail")
            .map(|(_, widget)| widget.is_visible())
            .unwrap_or(false);
        let chat_width = if detail_open {
            spec::CHAT_CARD_WIDTH
        } else {
            spec::CHAT_CARD_WIDTH_CLOSED
        };
        let messages_width = if detail_open {
            spec::MESSAGE_WIDTH - 1
        } else {
            spec::MESSAGE_WIDTH_CLOSED - 1
        };
        if let Some((_, widget)) = probes.iter().find(|(probe, _)| probe == "eb-messages") {
            if widget.is_visible() {
                check(
                    widget.allocation().width() == messages_width,
                    format!(
                        "eb-messages is {} wide, want {messages_width} (detail open={detail_open})",
                        widget.allocation().width()
                    ),
                    &mut failures,
                );
            }
        }
        for (name, want_width, want_height) in [
            ("eb-chat-card", chat_width, spec::CHAT_CARD_HEIGHT),
            ("eb-detail", spec::DETAIL_WIDTH, spec::WINDOW_HEIGHT),
        ] {
            let Some((_, widget)) = probes.iter().find(|(probe, _)| probe == name) else {
                check(false, format!("probe {name} is missing"), &mut failures);
                continue;
            };
            if !widget.is_visible() {
                continue;
            }
            let actual = widget.allocation();
            check(
                actual.width() == want_width && actual.height() == want_height,
                format!(
                    "{name} is {}x{}, want {want_width}x{want_height}",
                    actual.width(),
                    actual.height()
                ),
                &mut failures,
            );
        }
    }

    let context = window.style_context();
    for name in COLOURS {
        check(
            context.lookup_color(name).is_some(),
            format!("@{name} did not resolve (malformed/incomplete stylesheet?)"),
            &mut failures,
        );
    }

    if failures > 0 {
        eprintln!("selftest: {failures} assertion(s) failed");
        std::process::exit(1);
    }
    println!("selftest: assertions passed");
}

/// Print the resolved palette and the geometry of every probed widget.
fn report(window: &gtk::ApplicationWindow, views: &Views, probes: &[(String, gtk::Widget)]) {
    let allocation = window.allocation();
    println!(
        "selftest: window {}x{} (want {}x{})",
        allocation.width(),
        allocation.height(),
        spec::WINDOW_WIDTH,
        spec::WINDOW_HEIGHT
    );
    println!(
        "selftest: sash sidebar x={} (want {})",
        views.inner.position(),
        spec::SIDEBAR_WIDTH
    );
    println!("selftest: sash artifact x={}", views.outer.position());

    let context = window.style_context();
    for name in COLOURS {
        match context.lookup_color(name) {
            Some(colour) => println!(
                "selftest: @{name} = ({:.3}, {:.3}, {:.3}, {:.3})",
                colour.red(),
                colour.green(),
                colour.blue(),
                colour.alpha()
            ),
            None => println!("selftest: @{name} = <MISSING>"),
        }
    }

    if std::env::var_os("EB_DEBUG").is_some() {
        println!(
            "selftest: stack child = {:?}",
            views.stack.visible_child_name().map(|n| n.to_string())
        );
        for (name, widget) in [
            ("outer", views.outer.clone().upcast::<gtk::Widget>()),
            ("inner", views.inner.clone().upcast::<gtk::Widget>()),
            ("stack", views.stack.clone().upcast::<gtk::Widget>()),
            ("sidebar", views.sidebar.root.clone().upcast::<gtk::Widget>()),
            (
                "welcome",
                views.welcome.root.clone().upcast::<gtk::Widget>(),
            ),
            (
                "conversation",
                views.conversation.root.clone().upcast::<gtk::Widget>(),
            ),
            ("detail", views.detail.root.clone().upcast::<gtk::Widget>()),
        ] {
            println!(
                "selftest:   {name:<13} visible={} mapped={} drawable={} alloc={}x{}",
                widget.is_visible(),
                widget.is_mapped(),
                widget.is_drawable(),
                widget.allocation().width(),
                widget.allocation().height()
            );
        }
    }

    println!("selftest: --- layout, absolute x,y and size ---");
    for (name, widget) in probes {
        if !widget.is_visible() {
            println!("selftest: {name:<22} <hidden>");
            continue;
        }
        let allocation = widget.allocation();
        let position = widget
            .translate_coordinates(window, 0, 0)
            .map(|(x, y)| format!("{x:>5},{y:>4}"))
            .unwrap_or_else(|| "    ?,   ?".to_string());
        println!(
            "selftest: {name:<22} {position}  {}x{}",
            allocation.width(),
            allocation.height()
        );
    }

    assert_geometry(window, views, probes);
}

/// Render the widget tree into a PNG with Cairo.
///
/// GTK has no DOM-snapshot equivalent, but `gtk_widget_draw` onto an offscreen
/// surface is close enough for a visual diff against the capture.
///
/// `EB_SHOT_PANED=1` draws the paned instead of the toplevel, and `EB_DEBUG=1`
/// fills the surface with magenta first. Both exist to tell "nothing was drawn"
/// apart from "the wrong thing was drawn" when a capture comes back empty.
fn snapshot(views: &Views, path: &str) {
    let window = &views.window;

    // A repaint that adds widgets (`detail::fill` does) leaves widgets flagged as
    // "allocation pending". `gtk_widget_draw` refuses to draw such a widget — it
    // draws *nothing at all*, which reads like an empty window — and GTK only
    // clears the flag from its own layout pass, which may never run because the
    // window's size never changes. Re-running allocation with the size the
    // widget already has clears the flag for the whole tree. GTK's own paint
    // path never calls `gtk_widget_draw`, so this is a capture concern only.
    window.size_allocate(&window.allocation());

    let allocation = window.allocation();
    let (width, height) = (allocation.width(), allocation.height());
    if width <= 0 || height <= 0 {
        eprintln!("shot: window has no size yet, skipping");
        return;
    }

    let surface = match gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, width, height) {
        Ok(surface) => surface,
        Err(error) => {
            eprintln!("shot: cannot create surface: {error}");
            return;
        }
    };
    let context = match gtk::cairo::Context::new(&surface) {
        Ok(context) => context,
        Err(error) => {
            eprintln!("shot: cannot create cairo context: {error}");
            return;
        }
    };

    let debug = std::env::var_os("EB_DEBUG").is_some();
    if debug {
        context.set_source_rgb(0.8, 0.2, 0.8);
        let _ = context.paint();
    }

    let target: gtk::Widget = window.clone().upcast();
    if debug {
        println!(
            "shot: target={} visible={} mapped={} drawable={} alloc={}x{}",
            target.type_().name(),
            target.is_visible(),
            target.is_mapped(),
            target.is_drawable(),
            target.allocation().width(),
            target.allocation().height()
        );
    }
    target.draw(&context);
    if debug {
        println!("shot: cairo status = {:?}", context.status());
    }

    let mut file = match std::fs::File::create(path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("shot: cannot create {path}: {error}");
            return;
        }
    };
    match surface.write_to_png(&mut file) {
        Ok(()) => println!("shot: wrote {path} ({width}x{height})"),
        Err(error) => eprintln!("shot: write failed: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The assertion map is keyed by label; a new step without an expectation
    /// would silently pass the audit. This keeps the two lists in sync.
    #[test]
    fn every_click_step_has_an_expectation() {
        let steps = build_steps();
        assert_eq!(steps.len(), 26, "the audit's step count changed");
        for step in &steps {
            assert!(
                !expected_click(&step.label).is_empty(),
                "no assertion for step {:?}",
                step.label
            );
        }
    }
}

#[cfg(test)]
mod assertion_tests {
    use super::*;

    #[test]
    fn click_assertion_detects_a_missing_observation() {
        std::env::set_var("EB_CLICK_ASSERT", "1");
        let before = CLICK_FAILURES.load(Ordering::SeqCst);
        assert_click_observation("chip[0]", "composer=\"definitely not the chip text\"");
        assert_eq!(CLICK_FAILURES.load(Ordering::SeqCst), before + 1);
        std::env::remove_var("EB_CLICK_ASSERT");
    }
}
