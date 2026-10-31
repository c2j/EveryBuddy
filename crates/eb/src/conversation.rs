//! The conversation pane — the product's `conversation-page-chrome`.
//!
//! Measured against the live capture (ui-spec §4.4), 580 wide:
//!
//! ```text
//! y=0    topbar        580x64, padding 0 16px, 8px gap
//!        options       540x32, 12px gap
//!         title        16px/600
//!         actions      108x32 = three 32px buttons, 4px gap (search, share,
//!                      right-panel toggle; search expands into a 300x32 pill)
//! y=64   message list  580x636, `padding: 0 40px` -> a 500 wide column
//! y=700  input area    580x168 = a 134px card inset 24px + a 34px AI notice
//! ```
//!
//! Every message sits in a `cr-frame`. Agent frames keep 16px above and below.
//! A user frame is 16px above and 28px below, and that 28px is the toolbar, so
//! the frame class drops the extra bottom padding. The user's turn is a
//! right-aligned bubble on #f2f2f2 whose width follows the text up to the
//! column. The agent's turn is a left-aligned card with a logo/name header.
//! Tool, diff, and table blocks keep 14px of vertical margin. Text blocks do
//! not: their 26px line box and 12px paragraph gap live in the text view.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;

use crate::composer::Composer;
use eb_api::PermissionDecision;
use crate::model::{Artifact, Block, DiffLine, PlanEntry, Task, Turn};
use crate::spec;
use crate::widgets::*;

/// What a scroll catcher's GdkWindow has to select before a real wheel reaches
/// its handler. GDK filters events at the window (`gdk_window_get_events`), so
/// without these two bits the handler is dead code as far as the pointer is
/// concerned - `gtk::propagate_event`, which the harnesses use, skips that
/// filter and hides the mistake.
const SCROLL_EVENTS: gtk::gdk::EventMask = gtk::gdk::EventMask::SCROLL_MASK
    .union(gtk::gdk::EventMask::SMOOTH_SCROLL_MASK);

#[derive(Clone)]
pub struct Wiring {
    /// The "show sidebar" affordance shown while the sidebar is collapsed.
    pub reveal: Rc<dyn Fn()>,
    /// The right-panel toggle in the chat topbar: opens the panel when it is
    /// closed, closes it when it is open.
    pub toggle_artifact: Rc<dyn Fn()>,
    /// An artifact card (or "View all artifacts") was clicked in a message:
    /// open the panel with that file's tab selected.
    pub open_artifact_at: Rc<dyn Fn(usize)>,
    /// "Edit and resend" under a user bubble: prefills the composer.
    pub edit: Rc<dyn Fn(String)>,
    /// "Regenerate" under a settled answer: resubmits the prompt.
    pub submit: Rc<dyn Fn(String)>,
    /// The reader reached the top of what is rendered: ask the shell for a
    /// repaint so `fill_messages` can attach an earlier slice of the
    /// transcript. Scroll events do not repaint on their own, and a long
    /// conversation is only rendered a window at a time.
    pub load_more: Rc<dyn Fn()>,
    /// The user answered an inline tool-permission card: deliver the decision
    /// to the waiting worker and schedule a repaint.
    pub resolve_permission: Rc<dyn Fn(PermissionDecision)>,
}

/// The quick actions one painted turn can reach. Built per turn by
/// `pack_turn`, because regenerate needs the prompt that started the answer.
#[derive(Clone)]
struct TurnActions {
    edit: Rc<dyn Fn(String)>,
    submit: Rc<dyn Fn(String)>,
    /// Opens the artifact panel on a given file tab.
    artifact: Rc<dyn Fn(usize)>,
    /// The user text before an agent turn; `None` on the first turn.
    prompt: Option<String>,
}

impl TurnActions {
    /// For callers without a shell wired up yet: the buttons paint, they just
    /// have nowhere to go.
    #[cfg(test)]
    fn detached() -> Self {
        let noop: Rc<dyn Fn(String)> = Rc::new(|_| {});
        Self {
            edit: noop.clone(),
            submit: noop,
            artifact: Rc::new(|_| {}),
            prompt: None,
        }
    }
}

pub struct Conversation {
    pub root: gtk::Box,
    pub topbar: gtk::Box,
    /// The action group in the topbar: search, share, right-panel toggle.
    pub actions: gtk::Box,
    /// The panel toggle; always present, its tooltip says which way it goes.
    pub artifact_button: gtk::Button,
    /// The expanded search pill (capture 3): it takes the magnifier's slot
    /// while active; the title stays visible.
    pub search_button: gtk::Button,
    pub search_bar: gtk::Box,
    pub search_entry: gtk::Entry,
    pub search_state: Rc<std::cell::RefCell<SearchState>>,
    /// Floating "back to the newest message" button; visible while the reader
    /// is scrolled up from the bottom edge.
    /// Answers an inline tool-permission card into the waiting worker.
    permission_resolver: std::cell::RefCell<Option<Rc<dyn Fn(PermissionDecision)>>>,
    /// The currently rendered card, keyed by its title so a repaint leaves it
    /// alone instead of rebuilding (and dropping button press state).
    permission_card: std::cell::RefCell<Option<(String, gtk::Box)>>,
    /// The topbar title, repainted with the active task's title.
    pub title: gtk::Label,
    /// The scroll region holding the messages.
    pub scroll: gtk::ScrolledWindow,
    /// The transcript's vertical bar, drawn outside the message column at the
    /// edge of the pane.
    pub transcript_bar: gtk::Scrollbar,
    /// The pane-wide scroll catcher under the message column. The scroller's own
    /// input window ends at the column edge, so without this the wheel does
    /// nothing over the padding left and right of the transcript.
    pub transcript_hits: gtk::EventBox,
    /// The page-wide scroll catcher around the conversation body: the composer
    /// card is centred with margins of its own, so the band under the transcript
    /// has blank pixels too, and a wheel there belongs to nothing unless this
    /// box claims it.
    pub page_hits: gtk::EventBox,
    /// The message list: the rendered window of `painted_task`'s frames, and
    /// only those.
    pub messages: gtk::Box,
    /// Render state per packed turn, oldest attached first. A streaming
    /// refresh updates the last turn's blocks in place; tearing the painted tree
    /// down per token is what makes the message area flicker.
    pub turns: std::rc::Rc<std::cell::RefCell<Vec<TurnRender>>>,
    /// Whether the message list follows the newest output. Scrolling up unpins
    /// it, so a streaming reply cannot yank the reader back to the bottom; a
    /// new turn or another task re-pins it.
    pub pinned: Rc<Cell<bool>>,
    /// Set while the view scrolls itself, so its own jump is not read as the
    /// user leaving the bottom.
    auto_scroll: Rc<Cell<bool>>,
    /// The task currently painted; a change means the user opened another
    /// conversation, where the newest message is the point.
    pub painted_task: Rc<Cell<Option<usize>>>,
    /// Off-screen conversations with their frame handles, keyed by task index,
    /// each with the turn its first frame renders. A switch re-attaches these
    /// widgets instead of rebuilding every block of the transcript: the rebuild
    /// cost grew with the conversation, which is what stacked up when the user
    /// clicked between running tasks.
    detached: Rc<RefCell<HashMap<usize, (usize, Vec<TurnRender>)>>>,
    /// Least-recently-off-screen first; bounds what a switch can hold alive.
    detached_order: Rc<RefCell<Vec<usize>>>,
    /// Index into the painted task of the turn `turns[0]` renders. A long
    /// conversation is rendered from here down, not from its first turn.
    first: Rc<Cell<usize>>,
    /// Set when the reader scrolls to the top edge; the next repaint answers it
    /// by attaching an earlier slice.
    pending_load: Rc<Cell<bool>>,
    /// The line a load must keep under the top edge of the viewport: the frame
    /// that was there when the slice was attached, and how far below the edge it
    /// sat. The view is corrected to it after every layout pass until it stops
    /// moving — see `ANCHOR_SETTLE_PASSES`.
    anchor: Rc<std::cell::RefCell<Option<(gtk::Widget, f64)>>>,
    /// Corrections spent on the current anchor; bounds how long a load can hold
    /// the scroll and therefore the load gate.
    anchor_passes: Rc<Cell<u8>>,
    pub reveal: gtk::Button,
    /// Callbacks the per-turn toolbars bind to.
    on_edit: Rc<dyn Fn(String)>,
    on_submit: Rc<dyn Fn(String)>,
    /// Artifact cards in an agent turn bind to this: open the panel with that
    /// file's tab selected.
    open_artifact_at: Rc<dyn Fn(usize)>,
}

pub fn build(wiring: &Wiring, composer: &Composer) -> Conversation {
    let root = vbox(0);
    class(&root, "eb-content");

    // ---- topbar ---------------------------------------------------------
    let reveal = icon_button("▶", "eb-icon", "Show sidebar");
    hide_from_show_all(&reveal);
    reveal.connect_clicked({
        let reveal = wiring.reveal.clone();
        move |_| reveal()
    });

    let topbar = hbox(8);
    class(&topbar, "eb-content-topbar");
    topbar.pack_start(&reveal, false, false, 0);

    let options = hbox(12);
    options.set_vexpand(false);
    options.set_valign(gtk::Align::Center);

    let title = text("", "eb-chat-title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    options.pack_start(&title, true, true, 0);

    // Measured (2026-10-24 captures): three 32px ghost buttons, 4px apart —
    // search, share, and the right-panel toggle — always on the topbar. The
    // search button swaps places with its expanded pill; the panel toggle
    // only changes its tooltip as the panel opens and closes.
    let actions = hbox(4);
    actions.set_valign(gtk::Align::Center);
    let search = icon_button("◉", "eb-icon", "Search in conversation ⌘F");
    actions.pack_start(&search, false, false, 0);

    // The expanded search sits in the magnifier's slot (built below, packed
    // here while still hidden).
    let search_bar = hbox(0);
    class(&search_bar, "eb-search-pill");
    search_bar.set_size_request(300, 32);
    search_bar.set_valign(gtk::Align::Center);
    actions.pack_start(&search_bar, false, false, 0);

    let share = icon_button("⤴", "eb-icon", "Share");
    attach_notice(
        &share,
        "Sharing a task is not part of this UI-only build.",
    );
    actions.pack_start(&share, false, false, 0);

    // Opens and closes the artifact panel; the shell keeps the tooltip in
    // step with the panel state on every refresh.
    let artifact_button = icon_button("◫", "eb-icon", "Open right panel ⇧⌘B");
    artifact_button.connect_clicked({
        let toggle_artifact = wiring.toggle_artifact.clone();
        move |_| toggle_artifact()
    });
    actions.pack_start(&artifact_button, false, false, 0);

    options.pack_start(&actions, false, false, 0);
    topbar.pack_start(&options, true, true, 0);
    root.pack_start(&topbar, false, false, 0);

    // ---- body -----------------------------------------------------------
    let body = vbox(0);
    body.set_vexpand(true);

    let messages = vbox(0);
    class(&messages, "eb-messages");
    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    // The column is centred by the *scroller*: `halign` plus a content width, so
    // the message list is allocated exactly the design width and its margins are
    // never part of a size request. Centring the list itself with margins looked
    // equivalent and is not: GtkViewport allocates its child
    // `max(minimum + margins, view width)`, and the minimum of a wrapped Chinese
    // paragraph is the width it last wrapped to (`_gtk_text_layout_get_min_width`
    // only breaks at spaces). GTK reallocates with the margins of the *previous*
    // frame, so a grow left the column at 1220px inside a 1389px pane and it
    // could never come back — the transcript drawn past the edge of its pane.
    scroll.set_halign(gtk::Align::Center);
    // `Automatic`, not `Never`: with `Never` the scroller forwards the message
    // list's minimum width into the window's own minimum, so the frame stops
    // shrinking and GTK's idea of the layout stops agreeing with the surface it
    // is drawn into. Scrolling sideways inside the column is the lesser evil.
    //
    // `External` for the vertical bar: inside the column it lands on the
    // right-hand edge of the 752px design width, which is where the quoted blocks
    // *inside* the messages draw their own bars. The two overlap and neither can
    // be grabbed. The product puts the transcript bar at the edge of the window,
    // so the bar lives outside the column (see the overlay below) and only the
    // adjustment is shared.
    scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::External);
    // Long model output must scroll inside the message column, never widen the
    // conversation pane (and with it the whole window). `max_content_width` only
    // truncates when the natural width propagates, so this pair is what makes the
    // scroller ask for exactly the design column: 752 of natural, 1 of minimum.
    scroll.set_propagate_natural_width(true);
    scroll.set_min_content_width(1);
    scroll.set_max_content_width(spec::MESSAGE_WIDTH_CLOSED);
    scroll.add(&messages);

    // Chat logs are expected to follow the newest line, but only while the
    // reader is *at* the newest line: reading history during a long run must
    // not be interrupted by every streaming delta.
    let pinned = Rc::new(Cell::new(true));
    let auto_scroll = Rc::new(Cell::new(false));
    let first = Rc::new(Cell::new(0usize));
    let pending_load = Rc::new(Cell::new(false));
    let anchor: Rc<std::cell::RefCell<Option<(gtk::Widget, f64)>>> =
        Rc::new(std::cell::RefCell::new(None));
    let anchor_passes = Rc::new(Cell::new(0u8));
    // "Back to the newest message" is the centred overlay attached further
    // down (attach_jump_latest); its sync closure drives visibility.
    {
        let pinned = pinned.clone();
        let auto_scroll = auto_scroll.clone();
        let first = first.clone();
        let pending_load = pending_load.clone();
        let anchor = anchor.clone();
        let load_more = wiring.load_more.clone();
        scroll.vadjustment().connect_value_changed(move |adjustment| {
            if auto_scroll.get() {
                return;
            }
            // The reader moved the view themselves, so the line the last load
            // promised to hold is no longer theirs to keep.
            anchor.borrow_mut().take();
            let below = adjustment.upper() - adjustment.value() - adjustment.page_size();
            pinned.set(below <= FOLLOW_SLACK);
            // The top of the rendered window is in view and earlier turns exist:
            // raise the flag and ask for a repaint, which attaches the slice.
            // The anchor is set from the moment a slice is planned until the view
            // has been held back on the reader's line, so the jump it causes
            // cannot queue the next one and walk the whole transcript in a burst.
            if adjustment.value() <= LOAD_EDGE
                && first.get() > 0
                && anchor.borrow().is_none()
            {
                pending_load.set(true);
                load_more();
            }
        });
    }

    // The slice a load attaches is measured in more than one pass: the reveal
    // negotiates with the heights Pango reports at that moment, then re-measures
    // the wrapped text it has just laid out and the content *above* the reader
    // grows again. That push is what the reader feels as a jump, and it cannot be
    // compensated by the one jump above — measured 1580px of late growth on a
    // 9490px slice. So while a load holds a line, every change of the content
    // height puts the view back on it, until the layout settles or the budget of
    // passes runs out (an unsettled layout must not close the load gate forever).
    {
        let anchor = anchor.clone();
        let auto_scroll = auto_scroll.clone();
        let settles = anchor_passes.clone();
        scroll.vadjustment().connect_changed(move |adjustment| {
            if auto_scroll.get() {
                return;
            }
            let Some((frame, line)) = anchor.borrow().clone() else {
                return;
            };
            let pass = settles.get();
            if pass >= ANCHOR_SETTLE_PASSES {
                anchor.borrow_mut().take();
                return;
            }
            settles.set(pass + 1);
            // The frame's allocation is in content coordinates, so this is the
            // scroll value that keeps it exactly `line` under the top edge.
            let want = frame.allocation().y() as f64 - line;
            if (want - adjustment.value()).abs() < 1.0 {
                return;
            }
            let was = adjustment.value();
            let guard = auto_scroll.clone();
            guard.set(true);
            adjustment.set_value(want);
            guard.set(false);
            if std::env::var_os("EB_DEBUG").is_some() {
                eprintln!(
                    "reanchor: pass {pass} {was:.0}px -> {want:.0}px (line {line:.0}px, \
                     content {:.0}px)",
                    adjustment.upper()
                );
            }
        });
    }

    // The product paints a 72px transparent-to-surface fade on the message
    // viewport, above the composer. It is pass-through so the last lines stay
    // selectable.
    let viewport = gtk::Overlay::new();
    viewport.set_vexpand(true);
    // The bar belongs to the pane, not the column: it floats over the padding at
    // the right edge, so the 752px column stays centred on the same axis as the
    // composer. It cannot be an overlay child of the scroller itself, because
    // GtkOverlay sizes its main child to the child's request when that child does
    // not expand, and then aligns the overlays against that 752px box — which
    // parked the bar at the column's edge, on top of the bars the quoted blocks
    // inside the messages draw. The row below expands, so its overlay children are
    // measured against the whole pane.
    let scroller_row = hbox(0);
    scroller_row.pack_start(&scroll, true, true, 0);
    // The scroller only owns an input window over the 752px column it is sized
    // to, so the padding on either side of the transcript swallowed the wheel:
    // the pointer had to be *on* the text for the page to move. This catcher
    // covers the whole pane and hands the event to the scroller with
    // `gtk_widget_event`, which runs GTK's own scroll path - the device's scroll
    // unit and smooth deltas, the clamp, the kinetic finish, overshoot - instead
    // of a copy of that maths here. Widening the scroller to the pane instead
    // would stretch the column, since `GtkViewport` sizes its child to the view
    // (§10-36): the column's width is what `max_content_width` pins.
    let transcript_hits = gtk::EventBox::new();
    // `GtkBox` derives its expand hint from its children; `GtkEventBox` has no
    // `compute_expand` of its own, so the hint stops at this box and the pane
    // above (GtkOverlay's main child, then the stack and the paned) reads the
    // transcript as a 752px non-expanding strip. Say it out loud instead.
    transcript_hits.set_hexpand(true);
    transcript_hits.set_vexpand(true);
    // `GtkEventBox` realises a window that selects the button and crossing masks
    // only, so a scroll handler on it would never see a real wheel: GDK filters
    // at the window, before the widget chain is involved. Ask for what the
    // handler handles - `GtkScrolledWindow` selects the same two bits.
    transcript_hits.add_events(SCROLL_EVENTS);
    transcript_hits.add(&scroller_row);
    {
        let scroll = scroll.clone();
        transcript_hits.connect_scroll_event(move |_, event| scroll.event(&**event).into());
    }
    viewport.add(&transcript_hits);
    let mask = gtk::Box::new(gtk::Orientation::Vertical, 0);
    class(&mask, "eb-message-mask");
    mask.set_size_request(-1, 72);
    mask.set_valign(gtk::Align::End);
    mask.set_halign(gtk::Align::Fill);
    mask.set_hexpand(true);
    mask.set_sensitive(false);
    mask.set_can_focus(false);
    viewport.add_overlay(&mask);
    viewport.set_overlay_pass_through(&mask, true);

    // The bar shares the scroller's own vertical adjustment, so wheel, keyboard,
    // drag and the follow-the-newest-line logic all still drive one position.
    let transcript_bar = gtk::Scrollbar::new(gtk::Orientation::Vertical, Some(&scroll.vadjustment()));
    class(&transcript_bar, "eb-transcript-bar");
    transcript_bar.set_halign(gtk::Align::End);
    transcript_bar.set_valign(gtk::Align::Fill);
    viewport.add_overlay(&transcript_bar);
    // Painted after the fade so the circle sits on top of it. The fade stays
    // pass-through; this button does not, so the click reaches it.
    attach_jump_latest(
        &viewport,
        &scroll,
        &pinned,
        &auto_scroll,
        &anchor,
        &pending_load,
    );
    {
        // Read and write the position, then get out of the way: at rest the bar is
        // a hairline, and it thickens while the transcript moves and for a beat
        // after. Each delta re-arms the one-shot (remove + add is microseconds),
        // but the class is only touched when it is absent, because adding a class
        // the node already has invalidates its style and queues a redraw - on a
        // streaming answer that fires per token, and §10-34/§10-35 is exactly
        // about not paying for repaints the reader cannot see.
        let bar = transcript_bar.clone();
        let settle = Rc::new(Cell::new(None::<gtk::glib::SourceId>));
        scroll.vadjustment().connect_value_changed(move |_| {
            if let Some(id) = settle.take() {
                id.remove();
            }
            let context = bar.style_context();
            if !context.has_class("eb-bar-active") {
                context.add_class("eb-bar-active");
            }
            let bar = bar.clone();
            let dim = settle.clone();
            let id = gtk::glib::timeout_add_local(
                std::time::Duration::from_millis(900),
                move || {
                    bar.style_context().remove_class("eb-bar-active");
                    dim.set(None);
                    gtk::glib::ControlFlow::Break
                },
            );
            settle.set(Some(id));
        });
    }
    {
        // Nothing to scroll, nothing to show: a short transcript must not leave a
        // dead bar pinned to the window edge.
        let bar = transcript_bar.clone();
        scroll.vadjustment().connect_changed(move |adjustment| {
            bar.set_visible(adjustment.upper() > adjustment.page_size() + 1.0);
        });
    }
    body.pack_start(&viewport, true, true, 0);

    // Measured 580x168: the card inset 24px plus the AI notice. Built by the
    // composer module because it shares the editor and toolbar with the landing
    // page; packed here because it is part of this pane's column.
    body.pack_start(&composer.root, false, false, 0);

    // The transcript band's padding is claimed by `transcript_hits`; the composer
    // band below it has blank margins of its own (the card is centred inside a
    // much wider pane) and nothing there owns a wheel event. One catcher over the
    // whole body covers both, so "scroll from the empty space" does not depend on
    // which band the pointer happens to be in. Widgets with their own windows -
    // the column's scroller, the composer's text view - still consume their
    // events first, and the catcher only sees what nobody claimed.
    let page_hits = gtk::EventBox::new();
    page_hits.add_events(SCROLL_EVENTS);
    page_hits.add(&body);
    {
        let scroll = scroll.clone();
        page_hits.connect_scroll_event(move |_, event| scroll.event(&**event).into());
    }

    root.pack_start(&page_hits, true, true, 0);

    // ---- the expanded search pill (capture 3) -----------------------------
    // 300x32 #f2f2f2 rounded card in the magnifier's slot: dimmed magnifier,
    // placeholder entry, then 24px previous/next/close steps and a × to
    // collapse. The title stays visible the whole time.
    let search_entry = gtk::Entry::new();
    search_entry.set_placeholder_text(Some("Search in conversation"));
    search_entry.set_width_chars(10);
    class(&search_entry, "eb-search-entry");
    let magnifier = crate::icons::image("search-dim", 14);
    if let Some(magnifier) = magnifier {
        magnifier.set_margin_end(8);
        search_bar.pack_start(&magnifier, false, false, 0);
    }
    search_bar.pack_start(&search_entry, true, true, 0);
    let pill_button = |icon: &str, tip: &str| {
        let button = flat_button("eb-icon-mini");
        button.set_tooltip_text(Some(tip));
        if let Some(image) = crate::icons::image(icon, 12) {
            button.add(&image);
        }
        button
    };
    let search_prev = pill_button("chevron-up", "Previous match");
    let search_next = pill_button("chevron-down", "Next match");
    let search_close = pill_button("close", "Close search");
    search_bar.pack_start(&search_prev, false, false, 0);
    search_bar.pack_start(&search_next, false, false, 0);
    search_bar.pack_start(&search_close, false, false, 0);

    let search_state: Rc<std::cell::RefCell<SearchState>> =
        Rc::new(std::cell::RefCell::new(SearchState::default()));
    let turns_state: Rc<std::cell::RefCell<Vec<TurnRender>>> =
        Rc::new(std::cell::RefCell::new(Vec::new()));

    {
        let state = search_state.clone();
        let scroll = scroll.clone();
        let turns = turns_state.clone();
        search_entry.connect_changed(move |entry| {
            let query = entry.text().to_string();
            search_set_query(&state, &turns, &scroll, query);
        });
    }
    {
        let state = search_state.clone();
        let scroll = scroll.clone();
        let turns = turns_state.clone();
        search_prev.connect_clicked(move |_| search_step(&state, &turns, &scroll, -1));
    }
    {
        let state = search_state.clone();
        let scroll = scroll.clone();
        let turns = turns_state.clone();
        search_next.connect_clicked(move |_| search_step(&state, &turns, &scroll, 1));
    }

    // Expanding swaps the pill into the magnifier's slot; collapsing clears
    // the query (which also clears any selection highlight on next refresh).
    let show_pill: Rc<dyn Fn()> = {
        let bar = search_bar.clone();
        let button = search.clone();
        let entry = search_entry.clone();
        Rc::new(move || {
            button.set_visible(false);
            bar.set_visible(true);
            entry.grab_focus();
        })
    };
    let hide_pill: Rc<dyn Fn()> = {
        let bar = search_bar.clone();
        let button = search.clone();
        let entry = search_entry.clone();
        Rc::new(move || {
            entry.set_text("");
            bar.set_visible(false);
            button.set_visible(true);
        })
    };
    search_close.connect_clicked({
        let hide = hide_pill.clone();
        move |_| hide()
    });
    search_entry.connect_key_press_event({
        let hide = hide_pill.clone();
        move |_, event| {
            if event.keyval() == gtk::gdk::keys::constants::Escape {
                hide();
                gtk::glib::Propagation::Stop
            } else {
                gtk::glib::Propagation::Proceed
            }
        }
    });
    // The magnifier expands the pill, WorkBuddy-style.
    search.connect_clicked({
        let show = show_pill.clone();
        move |_| show()
    });
    // `show_all` would otherwise re-expand the hidden pill on every pass.
    hide_from_show_all(&search_bar);

    Conversation {
        root,
        topbar,
        actions,
        artifact_button,
        title,
        scroll,
        transcript_bar,
        transcript_hits,
        page_hits,
        messages,
        reveal,
        turns: turns_state,
        pinned,
        auto_scroll,
        painted_task: Rc::new(Cell::new(None)),
        detached: Rc::new(RefCell::new(HashMap::new())),
        detached_order: Rc::new(RefCell::new(Vec::new())),
        first,
        pending_load,
        anchor,
        anchor_passes,
        search_button: search,
        search_bar,
        search_entry,
        search_state,
        on_edit: wiring.edit.clone(),
        on_submit: wiring.submit.clone(),
        open_artifact_at: wiring.open_artifact_at.clone(),
        permission_resolver: RefCell::new(Some(wiring.resolve_permission.clone())),
        permission_card: RefCell::new(None),
    }
}

/// Conversation search state: the live query and the turns it matches.
#[derive(Default)]
pub struct SearchState {
    pub query: String,
    /// Packed-turn indexes (in pack order) whose text contains the query.
    pub hits: Vec<usize>,
    /// Which hit next/prev steps point at.
    pub cursor: usize,
}

/// What is currently packed for one turn, so a refresh can update widgets in
/// place instead of rebuilding them.
#[derive(Default)]
struct TurnRender {
    /// Header identity: user text/time, or agent name/status/streaming. A
    /// change here rebuilds the turn frame.
    sig: u64,
    /// Per-block content signatures (agent turns only).
    blocks: Vec<u64>,
    /// The agent block container, kept for in-place block updates.
    body: Option<gtk::Box>,
    /// The packed turn widget. `Container::children()` walks a GtkBox in
    /// reverse, so removals must use these recorded handles, not children().
    frame: Option<gtk::Widget>,
    /// Packed block widgets, parallel to `blocks`.
    widgets: Vec<gtk::Widget>,
}

/// Rebuild the message list for one task.
///
/// Rebuilding rather than appending keeps a single source of truth: the widgets
/// always describe the current state of `App`, never an accumulation of deltas.
///
/// `opened` is the task the pane shows. Switching tasks detaches the outgoing
/// conversation's frames into `detached` and re-attaches the incoming one's, so
/// a switch costs a container update per turn rather than a full re-render of
/// both transcripts.
///
/// Only a window of the transcript is mounted: `conversation.turns` covers
/// `task.turns[first..]`, and everything indexed below is *window-relative* —
/// add `first` to reach the task. The reader gets earlier turns by scrolling to
/// the top, which lowers `first` and prepends a slice.
/// Render (or withdraw) the inline tool-permission card under the newest
/// turn. Keyed by title: a repaint leaves an existing card alone, so button
/// press state survives.
fn update_permission_card(conversation: &Conversation, task: &Task) {
    let pending = match &task.pending_permission {
        Some(pending) => pending,
        None => {
            if let Some((_, card)) = conversation.permission_card.take() {
                conversation.messages.remove(&card);
            }
            return;
        }
    };
    // Snapshot the current key first: holding the borrow across `take()`
    // below would double-borrow the RefCell and panic.
    let existing = conversation
        .permission_card
        .borrow()
        .as_ref()
        .map(|(title, _)| title.clone());
    match existing {
        Some(title) if title == pending.title => return,
        Some(_) => {
            if let Some((_, card)) = conversation.permission_card.take() {
                conversation.messages.remove(&card);
            }
        }
        None => {}
    }

    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    class(&card, "eb-permission-card");
    let title_label = gtk::Label::new(Some(&pending.title));
    title_label.set_line_wrap(true);
    title_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    title_label.set_width_chars(1);
    title_label.set_max_width_chars(60);
    title_label.set_xalign(0.0);
    class(&title_label, "eb-permission-card-title");
    card.pack_start(&title_label, false, false, 0);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::Start);
    for option in &pending.options {
        let button = gtk::Button::with_label(&option.label);
        class(&button, "eb-chip");
        let responder = pending.responder.clone();
        let resolver = conversation.permission_resolver.borrow().clone();
        let decision = PermissionDecision::Selected(option.id.clone());
        button.connect_clicked(move |_| {
            let _ = responder.respond(decision.clone());
            if let Some(resolve) = &resolver {
                resolve(decision.clone());
            }
        });
        buttons.pack_start(&button, false, false, 0);
    }
    card.pack_start(&buttons, false, false, 0);

    conversation.messages.pack_start(&card, false, false, 8);
    card.show_all();
    *conversation.permission_card.borrow_mut() =
        Some((pending.title.clone(), card));
}

pub fn fill_messages(conversation: &Conversation, task: &Task, opened: Option<usize>) {
    if conversation.painted_task.get() != opened {
        if let Some(target) = opened {
            conversation.switch_conversation(target, task.turns.len());
        }
        conversation.painted_task.set(opened);
        // A load request raised against the conversation that just left the
        // screen is not a request about this one, and neither is an anchor: its
        // frame is off-screen now and would move the new view to where the old
        // reader was sitting.
        conversation.pending_load.set(false);
        conversation.anchor.borrow_mut().take();
    }
    if conversation.pending_load.take() {
        conversation.load_earlier(task);
    }
    let first = conversation.first.get().min(task.turns.len());
    conversation.first.set(first);
    let signed_started = std::time::Instant::now();
    let mut fresh: Vec<TurnRender> = task.turns[first..].iter().map(turn_state).collect();
    let signed_ms = signed_started.elapsed().as_secs_f64() * 1000.0;

    // The card row lives on the last settled agent turn, so artifact changes
    // must dirty that turn's header signature — otherwise a new file written
    // after the turn completed would never repaint the cards.
    let owner = artifact_owner(task);
    if let Some(index) = owner {
        // The card row can sit above the window, where nothing is painted.
        if let Some(slot) = index.checked_sub(first).and_then(|i| fresh.get_mut(i)) {
            slot.sig = slot.sig.wrapping_add(artifact_sig(&task.artifacts));
        }
    }
    let turn_artifacts = |index: usize| -> &[Artifact] {
        if owner == Some(index) {
            &task.artifacts
        } else {
            &[]
        }
    };

    // Everything already painted and unchanged: leave the widget tree alone.
    // Rebuilding per streaming token is what makes the area flicker.
    {
        let packed = conversation.turns.borrow();
        if packed.len() == fresh.len()
            && packed
                .iter()
                .zip(fresh.iter())
                .all(|(old, new)| old.sig == new.sig && old.blocks == new.blocks)
        {
            update_permission_card(conversation, task);
            return;
        }
    }

    // A turn appearing is something the user started (a submission, a
    // regenerate), so follow it even from scrolled-up history. Deltas inside an
    // existing turn deliberately do not re-pin.
    if task.turns.len() > first + conversation.turns.borrow().len() {
        conversation.pinned.set(true);
    }

    // Turns whose full state matches are reused as-is; the mismatch index is
    // where the update starts.
    let reuse = {
        let packed = conversation.turns.borrow();
        packed
            .iter()
            .zip(fresh.iter())
            .take_while(|(old, new)| old.sig == new.sig && old.blocks == new.blocks)
            .count()
    };
    let stale_frames: Vec<gtk::Widget> = {
        let packed = conversation.turns.borrow();
        packed
            .iter()
            .skip(reuse)
            .filter_map(|turn| turn.frame.clone())
            .collect()
    };
    for frame in stale_frames {
        conversation.messages.remove(&frame);
    }
    conversation.turns.borrow_mut().truncate(reuse);

    // Fast path for the streaming tail: the turn's header (name, status,
    // streaming) is unchanged, so only its blocks moved. Update the packed
    // block widgets in place — for a growing text block that means rewriting
    // the TextView buffer, not recreating the widget.
    let in_place_body = {
        let packed = conversation.turns.borrow();
        let debug = std::env::var_os("EB_DEBUG").is_some();
        if debug {
            eprintln!(
                "fill_messages: reuse={reuse} packed_len={} fresh_len={} first={first} \
                 signed={signed_ms:.1}ms",
                packed.len(),
                fresh.len(),
            );
        }
        packed.get(reuse).and_then(|old| {
            let header_match = old.sig == fresh[reuse].sig;
            if debug && !header_match {
                eprintln!(
                    "fill_messages: header sig mismatch old={} new={}",
                    old.sig, fresh[reuse].sig
                );
            }
            (header_match
                && old.body.is_some()
                && matches!(task.turns.get(first + reuse), Some(Turn::Agent { .. })))
            .then(|| old.body.clone())
        })
    };
    if let Some(Some(body)) = in_place_body {
        if let Some(Turn::Agent { blocks, .. }) = task.turns.get(first + reuse) {
            let sync_started = std::time::Instant::now();
            {
                let mut packed = conversation.turns.borrow_mut();
                if let Some(old) = packed.get_mut(reuse) {
                    sync_blocks(&body, &mut old.blocks, &mut old.widgets, blocks);
                }
            }
            let sync_ms = sync_started.elapsed().as_secs_f64() * 1000.0;
            // Any further turns (a batch that also appended) still need packing;
            // pack_turn records their render state.
            let tail_started = std::time::Instant::now();
            for index in first + reuse + 1..task.turns.len() {
                pack_turn(
                    conversation,
                    &task.turns[index],
                    &fresh[index - first],
                    task.started_at,
                    prompt_before(&task.turns, index),
                    turn_artifacts(index),
                );
            }
            if std::env::var_os("EB_DEBUG").is_some() {
                eprintln!(
                    "fill_messages: in-place sync={sync_ms:.1}ms tail={:.1}ms",
                    tail_started.elapsed().as_secs_f64() * 1000.0
                );
            }
            conversation.follow_output();
            return;
        }
    }

    let pack_started = std::time::Instant::now();
    for index in reuse..fresh.len() {
        let turn_index = first + index;
        pack_turn(
            conversation,
            &task.turns[turn_index],
            &fresh[index],
            task.started_at,
            prompt_before(&task.turns, turn_index),
            turn_artifacts(turn_index),
        );
    }
    conversation.turns.borrow_mut().truncate(fresh.len());
    let pack_ms = pack_started.elapsed().as_secs_f64() * 1000.0;
    if std::env::var_os("EB_DEBUG").is_some() {
        eprintln!(
            "fill_messages: packed {} turns in {pack_ms:.1}ms (reuse={reuse} first={first} \
             mounted={})",
            conversation.turns.borrow().len().saturating_sub(reuse),
            conversation.turns.borrow().len(),
        );
    }
    conversation.follow_output();
    update_permission_card(conversation, task);
}

/// The user text directly before `index`, which is what "Regenerate" resubmits.
fn prompt_before(turns: &[Turn], index: usize) -> Option<String> {
    turns[..index].iter().rev().find_map(|turn| match turn {
        Turn::User { text, .. } => Some(text.clone()),
        _ => None,
    })
}

/// Render one turn and remember what was packed.
///
/// `artifacts` is the task's written files, but only for the turn that owns
/// the card row — see `artifact_owner`.
fn pack_turn(
    conversation: &Conversation,
    turn: &Turn,
    state: &TurnRender,
    started_at: Option<std::time::Instant>,
    prompt: Option<String>,
    artifacts: &[Artifact],
) {
    let render = build_turn(conversation, turn, state, started_at, prompt, artifacts);
    conversation.turns.borrow_mut().push(render);
}

/// Render one turn into the message list and return its render state.
///
/// Split from `pack_turn` because the scroll-up path packs a slice at the *top*
/// of the list and has to reorder the frames it just built; the normal path
/// appends. Both need the handles, so neither can leave them inside a push.
fn build_turn(
    conversation: &Conversation,
    turn: &Turn,
    state: &TurnRender,
    started_at: Option<std::time::Instant>,
    prompt: Option<String>,
    artifacts: &[Artifact],
) -> TurnRender {
    let actions = TurnActions {
        edit: conversation.on_edit.clone(),
        submit: conversation.on_submit.clone(),
        artifact: conversation.open_artifact_at.clone(),
        prompt,
    };
    match turn {
        Turn::User { text, time } => {
            let frame = user_turn(text, time, &actions);
            // Show what this turn just built, not the whole list: a blanket
            // `messages.show_all()` walks every widget in the conversation on
            // every frame, which is what made a long transcript expensive to
            // repaint (ui-spec §10-34).
            frame.show_all();
            conversation.messages.pack_start(&frame, false, false, 0);
            TurnRender {
                sig: state.sig,
                frame: Some(frame.upcast()),
                ..Default::default()
            }
        }
        Turn::Agent {
            name,
            status,
            blocks,
            streaming,
            time,
        } => {
            let status = status.as_deref().map(|status| {
                status_with_elapsed(status, *streaming, started_at)
            });
            let parts = agent_turn(
                name,
                status.as_deref(),
                blocks,
                *streaming,
                artifacts,
                time.as_ref().map(clock_hhmm).as_deref(),
                &actions,
            );
            let mut widgets: Vec<gtk::Widget> = Vec::new();
            for block in blocks {
                let widget = block_widget(block);
                parts.body.pack_start(&widget, false, false, 0);
                widgets.push(widget);
            }
            parts.frame.show_all();
            conversation.messages.pack_start(&parts.frame, false, false, 0);
            TurnRender {
                sig: state.sig,
                blocks: state.blocks.clone(),
                body: Some(parts.body),
                widgets,
                frame: Some(parts.frame.upcast()),
            }
        }
    }
}

/// Bring an agent turn's block widgets in line with `blocks`, reusing what is
/// already on screen. A text block that only grew is updated by rewriting its
/// TextView buffer — the streaming hot path.
fn sync_blocks(
    body: &gtk::Box,
    block_sigs: &mut Vec<u64>,
    widgets: &mut Vec<gtk::Widget>,
    blocks: &[Block],
) {
    if blocks.is_empty() {
        for widget in widgets.drain(..) {
            body.remove(&widget);
        }
        block_sigs.clear();
        return;
    }
    let mut index = 0;
    while index < blocks.len() && index < widgets.len() {
        if block_sigs.get(index) != Some(&block_sig(&blocks[index])) {
            break;
        }
        index += 1;
    }
    // The common case: the final text or thought block grew by a streaming
    // chunk.
    if index == blocks.len() - 1 && widgets.len() == blocks.len() {
        match &blocks[index] {
            Block::Text(_) => {
                if rewrite_markdown_text(&widgets[index], &markdown_text(&blocks[index])) {
                    block_sigs[index] = block_sig(&blocks[index]);
                    return;
                }
            }
            Block::Thought {
                text,
                collapsed: false,
            } => {
                // Still streaming: rewrite the label. A fold replaces the
                // widget, because the body has to disappear.
                if rewrite_thought(&widgets[index], text) {
                    block_sigs[index] = block_sig(&blocks[index]);
                    return;
                }
            }
            _ => {}
        }
    }
    // Anything else: replace the tail from the first changed block on.
    for widget in widgets.drain(index..) {
        body.remove(&widget);
    }
    block_sigs.truncate(index);
    for block in &blocks[index..] {
        let widget = block_widget(block);
        widget.show_all();
        body.pack_start(&widget, false, false, 0);
        widgets.push(widget);
        block_sigs.push(block_sig(block));
    }
}

fn block_sig(block: &Block) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{block:?}").hash(&mut hasher);
    hasher.finish()
}

fn markdown_text(block: &Block) -> String {
    match block {
        Block::Text(value) => value.clone(),
        _ => String::new(),
    }
}

/// Streaming growth of a thought block: rewrite the label in place instead of
/// rebuilding the tail, so thinking does not flicker (same discipline as
/// streamed text).
fn rewrite_thought(widget: &gtk::Widget, value: &str) -> bool {
    let Some(label) = thought_label(widget) else {
        return false;
    };
    label.set_text(value);
    true
}

fn thought_label(widget: &gtk::Widget) -> Option<gtk::Label> {
    if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
        if label.style_context().has_class("eb-thought") {
            return Some(label);
        }
    }
    let Ok(container) = widget.clone().downcast::<gtk::Container>() else {
        return None;
    };
    container.children().into_iter().find_map(|child| thought_label(&child))
}

/// Rewrite an already-packed markdown text view with fresh content. Fails for
/// widget trees that are not a single text view (fenced code and friends).
fn rewrite_markdown_text(widget: &gtk::Widget, value: &str) -> bool {
    let Ok(box_) = widget.clone().downcast::<gtk::Box>() else {
        return false;
    };
    let Some(view) = box_
        .children()
        .first()
        .and_then(|child| child.clone().downcast::<gtk::TextView>().ok())
    else {
        return false;
    };
    crate::markdown::write_markdown_text(&view, value)
}

/// Hash everything a turn renders *except* its blocks; agent block moves are
/// handled by `sync_blocks` in place.
fn turn_state(turn: &Turn) -> TurnRender {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut state = TurnRender::default();
    match turn {
        Turn::User { text, time } => {
            "user".hash(&mut hasher);
            text.hash(&mut hasher);
            time.hash(&mut hasher);
        }
        Turn::Agent {
            name,
            status,
            streaming,
            time,
            ..
        } => {
            "agent".hash(&mut hasher);
            name.hash(&mut hasher);
            status.hash(&mut hasher);
            streaming.hash(&mut hasher);
            time.hash(&mut hasher);
            state.blocks = match turn {
                Turn::Agent { blocks, .. } => blocks.iter().map(block_sig).collect(),
                _ => Vec::new(),
            };
        }
    }
    state.sig = hasher.finish();
    state
}

impl Conversation {
    /// Expands the search pill in the magnifier's slot — what the magnifier
    /// click does, reused by the ⌘F accelerator.
    pub fn show_search(&self) {
        self.search_button.set_visible(false);
        self.search_bar.set_visible(true);
        self.search_entry.grab_focus();
    }

    /// Collapses the pill and clears the query — the × / Escape behaviour.
    pub fn hide_search(&self) {
        self.search_entry.set_text("");
        self.search_bar.set_visible(false);
        self.search_button.set_visible(true);
    }

    /// Applies a new conversation-search query: recomputes the matching turns
    /// and selects the first occurrence.
    pub fn search_set_query(&self, query: String) {
        self.search_state.borrow_mut().query = query.clone();
        self.search_recompute();
        self.search_jump(0);
    }

    /// Steps through matching turns: positive is next, negative is previous.
    pub fn search_step(&self, delta: i32) {
        let (hits, current) = {
            let state = self.search_state.borrow();
            (state.hits.len(), state.cursor)
        };
        if hits == 0 {
            return;
        }
        let next = (current as i32 + delta).rem_euclid(hits as i32) as usize;
        self.search_state.borrow_mut().cursor = next;
        self.search_jump(next);
    }

    /// Clears the query; selections are left as they are.
    pub fn search_close(&self) {
        self.search_state.borrow_mut().query.clear();
    }

    /// Recomputes which packed turns contain the query.
    fn search_recompute(&self) {
        let query = self.search_state.borrow().query.to_lowercase();
        let mut hits = Vec::new();
        if !query.is_empty() {
            for (index, render) in self.turns.borrow().iter().enumerate() {
                if let Some(frame) = &render.frame {
                    if widget_text_contains(frame, &query) {
                        hits.push(index);
                    }
                }
            }
        }
        self.search_state.borrow_mut().hits = hits;
    }

    /// Refreshes the hit list after the turn widgets were rebuilt.
    fn search_refresh(&self) {
        if !self.search_state.borrow().query.is_empty() {
            self.search_recompute();
        }
    }

    /// Brings hit `cursor` on screen and selects the first occurrence.
    fn search_jump(&self, hit: usize) {
        let (turn_index, query) = {
            let state = self.search_state.borrow();
            (
                state.hits.get(hit).copied(),
                state.query.to_lowercase(),
            )
        };
        let (Some(turn_index), false) = (turn_index, query.is_empty()) else {
            return;
        };
        let frame = self.turns.borrow().get(turn_index).and_then(|r| r.frame.clone());
        let Some(frame) = frame else {
            return;
        };
        scroll_to_widget(&self.scroll, &frame);
        select_first_match(&frame, &query);
    }
}

/// Case-insensitive text of every TextView/Label under `widget`.
fn widget_text_contains(widget: &gtk::Widget, needle: &str) -> bool {
    if let Ok(view) = widget.clone().downcast::<gtk::TextView>() {
        let buffer = view.buffer().expect("a GtkTextView always has a buffer");
        return buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .map(|text| text.to_lowercase().contains(needle))
            .unwrap_or(false);
    }
    if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
        return label.text().to_lowercase().contains(needle);
    }
    if let Ok(container) = widget.clone().downcast::<gtk::Container>() {
        return container
            .children()
            .iter()
            .any(|child| widget_text_contains(child, needle));
    }
    false
}

/// Selects the first case-insensitive occurrence of `query` inside any
/// TextView/Label under `widget`.
fn select_first_match(widget: &gtk::Widget, query: &str) {
    if let Ok(view) = widget.clone().downcast::<gtk::TextView>() {
        let buffer = view.buffer().expect("a GtkTextView always has a buffer");
        let start = buffer.start_iter();
        if let Some((begin, end)) = start.forward_search(
            query,
            gtk::TextSearchFlags::CASE_INSENSITIVE,
            None,
        ) {
            buffer.select_range(&begin, &end);
        }
        return;
    }
    if let Ok(label) = widget.clone().downcast::<gtk::Label>() {
        let text = label.text().to_lowercase();
        if let Some(position) = text.find(query) {
            let start = text[..position].chars().count();
            let end = start + query.chars().count();
            label.select_region(start as i32, end as i32);
        }
        return;
    }
    if let Ok(container) = widget.clone().downcast::<gtk::Container>() {
        for child in container.children() {
            select_first_match(&child, query);
            if let Some(label) = child.downcast_ref::<gtk::Label>() {
                if label.selection_bounds().is_some() {
                    return;
                }
            }
            if let Some(view) = child.downcast_ref::<gtk::TextView>() {
                if view.buffer().expect("buffer").has_selection() {
                    return;
                }
            }
        }
    }
}

/// Scrolls `scroll` so `widget` (a descendant) is near the top.
fn scroll_to_widget(scroll: &gtk::ScrolledWindow, widget: &gtk::Widget) {
    if let Some((_, y)) = widget.translate_coordinates(scroll, 0, 0) {
        let adjustment = scroll.vadjustment();
        let max = (adjustment.upper() - adjustment.page_size()).max(0.0);
        adjustment.set_value((y as f64 - 24.0).clamp(0.0, max));
    }
}


fn search_set_query(
    state: &std::cell::RefCell<SearchState>,
    turns: &std::cell::RefCell<Vec<TurnRender>>,
    scroll: &gtk::ScrolledWindow,
    query: String,
) {
    state.borrow_mut().query = query.clone();
    let hits = search_hits(turns, &query);
    {
        let mut st = state.borrow_mut();
        st.hits = hits;
        st.cursor = 0;
    }
    search_jump(state, turns, scroll, 0);
}

fn search_step(
    state: &std::cell::RefCell<SearchState>,
    turns: &std::cell::RefCell<Vec<TurnRender>>,
    scroll: &gtk::ScrolledWindow,
    delta: i32,
) {
    let (hits, current) = {
        let st = state.borrow();
        (st.hits.len(), st.cursor)
    };
    if hits == 0 {
        return;
    }
    let next = (current as i32 + delta).rem_euclid(hits as i32) as usize;
    state.borrow_mut().cursor = next;
    search_jump(state, turns, scroll, next);
}

fn search_hits(turns: &std::cell::RefCell<Vec<TurnRender>>, query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    let mut hits = Vec::new();
    if !query.is_empty() {
        for (index, render) in turns.borrow().iter().enumerate() {
            if let Some(frame) = &render.frame {
                if widget_text_contains(frame, &query) {
                    hits.push(index);
                }
            }
        }
    }
    hits
}

fn search_jump(
    state: &std::cell::RefCell<SearchState>,
    turns: &std::cell::RefCell<Vec<TurnRender>>,
    scroll: &gtk::ScrolledWindow,
    hit: usize,
) {
    let (turn_index, query) = {
        let st = state.borrow();
        (st.hits.get(hit).copied(), st.query.to_lowercase())
    };
    let (Some(turn_index), false) = (turn_index, query.is_empty()) else {
        return;
    };
    let frame = turns.borrow().get(turn_index).and_then(|r| r.frame.clone());
    let Some(frame) = frame else {
        return;
    };
    scroll_to_widget(scroll, &frame);
    select_first_match(&frame, &query);
}

fn status_with_elapsed(status: &str, streaming: bool, started: Option<std::time::Instant>) -> String {
    if !streaming {
        return status.to_string();
    }
    let Some(started) = started else {
        return status.to_string();
    };
    let seconds = started.elapsed().as_secs();
    if seconds == 0 {
        return status.to_string();
    }
    if seconds >= 60 {
        format!("{status} · {}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{status} · {seconds}s")
    }
}

/// How far from the bottom still counts as "watching the newest message".
/// Half a line of text: enough that a trackpad flick that lands a few pixels
/// short keeps following, but a deliberate scroll up stops it.
const FOLLOW_SLACK: f64 = 24.0;

fn distance_from_end(adjustment: &gtk::Adjustment) -> f64 {
    adjustment.upper() - adjustment.value() - adjustment.page_size()
}

/// The arrow is for a reader who left the tail. While they are still following,
/// new tokens grow the transcript before the view catches up, and that gap must
/// not flash the button.
fn jump_latest_visible(adjustment: &gtk::Adjustment, pinned: bool) -> bool {
    !pinned && distance_from_end(adjustment) > FOLLOW_SLACK
}

/// The down arrow that brings a reader who scrolled up back to the newest
/// message. Measured from WorkBuddy `.cr-scroll-to-bottom`: a 32px circle,
/// 16px chevron, 20px above the composer, centred on the column. It is an
/// overlay of the pane (the same one as the fade), not of the 752px scroller,
/// so the centring matches the column instead of being clipped to it.
fn attach_jump_latest(
    viewport: &gtk::Overlay,
    scroll: &gtk::ScrolledWindow,
    pinned: &Rc<Cell<bool>>,
    auto_scroll: &Rc<Cell<bool>>,
    anchor: &Rc<RefCell<Option<(gtk::Widget, f64)>>>,
    pending_load: &Rc<Cell<bool>>,
) -> gtk::Button {
    let jump = flat_button("eb-jump-latest");
    jump.set_tooltip_text(Some("Back to latest message"));
    jump.set_halign(gtk::Align::Center);
    jump.set_valign(gtk::Align::End);
    jump.set_margin_bottom(20);
    jump.set_size_request(32, 32);
    if let Some(icon) = crate::icons::image("chevron-down", 16) {
        jump.add(&icon);
    } else {
        jump.add(&glyph("⌄"));
    }
    // `show_all` on the window must not reveal it: it starts hidden and only
    // appears once the reader has left the newest message.
    hide_from_show_all(&jump);
    viewport.add_overlay(&jump);

    let sync: Rc<dyn Fn(&gtk::Adjustment)> = Rc::new({
        let jump = jump.clone();
        let pinned = pinned.clone();
        move |adjustment: &gtk::Adjustment| {
            let show = jump_latest_visible(adjustment, pinned.get());
            if jump.is_visible() != show {
                jump.set_visible(show);
            }
        }
    });
    {
        // Runs after the follow handler connected above, which is what clears
        // the history anchor. This one only records whether the tail is in view
        // and shows the arrow. A jump the view makes itself (`auto_scroll`) is
        // not the reader leaving.
        let pinned = pinned.clone();
        let auto_scroll = auto_scroll.clone();
        let sync = sync.clone();
        scroll.vadjustment().connect_value_changed(move |adjustment| {
            if !auto_scroll.get() {
                pinned.set(distance_from_end(adjustment) <= FOLLOW_SLACK);
            }
            sync(adjustment);
        });
    }
    {
        let sync = sync.clone();
        scroll.vadjustment().connect_changed(move |adjustment| {
            sync(adjustment);
        });
    }
    jump.connect_clicked({
        let scroll = scroll.clone();
        let pinned = pinned.clone();
        let auto_scroll = auto_scroll.clone();
        let anchor = anchor.clone();
        let pending_load = pending_load.clone();
        let jump = jump.clone();
        move |_| {
            reveal_latest(&scroll, &pinned, &auto_scroll, &anchor, &pending_load);
            jump.set_visible(false);
        }
    });
    jump
}

/// Re-pin and jump to the newest line. Clears a history load's anchor first:
/// otherwise the next content-size change puts the view back on the line the
/// reader had asked to leave.
fn reveal_latest(
    scroll: &gtk::ScrolledWindow,
    pinned: &Rc<Cell<bool>>,
    auto_scroll: &Rc<Cell<bool>>,
    anchor: &Rc<RefCell<Option<(gtk::Widget, f64)>>>,
    pending_load: &Rc<Cell<bool>>,
) {
    pinned.set(true);
    anchor.borrow_mut().take();
    pending_load.set(false);
    let adjustment = scroll.vadjustment();
    let guard = auto_scroll.clone();
    let place = |adjustment: &gtk::Adjustment, guard: &Rc<Cell<bool>>| {
        guard.set(true);
        adjustment.set_value(adjustment.upper() - adjustment.page_size());
        guard.set(false);
    };
    place(&adjustment, &guard);
    gtk::glib::idle_add_local_once(move || place(&adjustment, &guard));
}

/// Conversations kept rendered while off screen. Switching between them is then
/// a re-attach; past the cap the oldest is rebuilt on its next visit, which is
/// what a task list with hundreds of entries would otherwise pin in memory.
const DETACHED_CONVERSATIONS: usize = 4;

/// Turns rendered when a conversation is painted: the newest, because the newest
/// is what opening a task is for. A deep-research transcript runs to a hundred
/// turns or more, and a switch costs a layout per mounted turn — 122 of them
/// measured 68ms of synchronous layout, which starved the main loop (ui-spec
/// §10-35). Earlier turns come back by scrolling to the top.
///
/// `pub(crate)` because the harnesses state their expectations in these units
/// rather than hard-coding a second copy of the window size.
pub(crate) const ATTACH_TURNS: usize = 20;

/// Turns attached each time the reader reaches the top edge. Smaller than the
/// first window so a load is a brief hitch on the deliberate gesture that asked
/// for it, and holding at the top keeps loading.
pub(crate) const ATTACH_STEP: usize = 10;

/// How close to the top of the rendered window counts as "the reader wants more"
/// — two screens, so the load lands before they run out of transcript.
const LOAD_EDGE: f64 = 400.0;

/// Corrections a single load may spend keeping the reader's line. The late
/// re-measure of a slice lands within a few layout passes; a budget, not a
/// "settled" test, is what bounds how long a load can hold the load gate.
const ANCHOR_SETTLE_PASSES: u8 = 16;

/// Frames kept per off-screen conversation. A reader who had scrolled all the
/// way to the first turn would otherwise hand the next switch a full transcript
/// to lay out again; the frames past this are rebuilt from the earlier slice.
const DETACHED_FRAMES: usize = 30;

impl Conversation {
    /// Keep the newest message in view — but only while the reader is already
    /// there. The layout is only final after the new children have been
    /// allocated, so the jump happens on the next idle turn rather than here.
    fn follow_output(&self) {
        // The screenshot harness can keep the top of a long conversation visible
        // so the first block is not clipped by the scroll-to-bottom behaviour.
        if std::env::var_os("EB_NO_SCROLL").is_some() || !self.pinned.get() {
            return;
        }
        let adjustment = self.scroll.vadjustment();
        let guard = self.auto_scroll.clone();
        gtk::glib::idle_add_local_once(move || {
            guard.set(true);
            adjustment.set_value(adjustment.upper() - adjustment.page_size());
            guard.set(false);
        });
    }

    /// Re-pin and jump: the user opened another task, so the newest message is
    /// the point of the switch.
    pub fn follow_now(&self) {
        self.pinned.set(true);
        self.follow_output();
    }

    /// The rendered window: its first turn in the task, and how many turns are
    /// mounted. Read by the switch harness, which cannot name the private cells.
    pub fn window(&self) -> (usize, usize) {
        (self.first.get(), self.turns.borrow().len())
    }

    /// Move the message list from one task's conversation to another's.
    ///
    /// The outgoing frames leave the container but keep their widgets: they go
    /// into `detached`, keyed by task, with the turn their first frame renders.
    /// Re-attaching them is a `pack_start` per turn, where the rebuild this
    /// replaces was a `block_widget` per block — markdown parse, code cards,
    /// tables, diff cards, all of it — for a transcript that had not changed.
    ///
    /// `total` is the incoming task's turn count. Without a cached window the
    /// conversation renders from `total - ATTACH_TURNS` down, which is what
    /// keeps a switch cheap on a task with a hundred turns of history.
    fn switch_conversation(&self, target: usize, total: usize) {
        let previous = match self.painted_task.get() {
            Some(previous) => previous,
            // Nothing painted yet: the packing path below builds the
            // conversation, so there is nothing to move out of the way — but it
            // builds a window of it, not the whole transcript.
            None => {
                self.first.set(total.saturating_sub(ATTACH_TURNS));
                return;
            }
        };
        let outgoing_first = self.first.get();
        let mut outgoing: Vec<TurnRender> = std::mem::take(&mut self.turns.borrow_mut());
        // One resize for the whole swap. `messages` is the child of the
        // scroller's internal GtkViewport, so hiding it from its parent stops
        // every `remove`/`pack_start` below from
        // queueing a resize over a transcript of thousands of widgets: 122 adds
        // costed ~112ms of UI-thread time each switch, which is what kept
        // starving the main loop on a long conversation.
        self.messages.set_child_visible(false);
        let removed_started = std::time::Instant::now();
        for render in &outgoing {
            if let Some(frame) = &render.frame {
                self.messages.remove(frame);
            }
        }
        let removed_ms = removed_started.elapsed().as_secs_f64() * 1000.0;
        if !outgoing.is_empty() {
            // The window comes along, but only its newest frames: the way back
            // re-pins to the bottom, and holding a history the reader scrolled
            // through would make the next switch cost O(that history) again.
            let dropped = outgoing.len().saturating_sub(DETACHED_FRAMES);
            outgoing.drain(..dropped);
            self.detached
                .borrow_mut()
                .insert(previous, (outgoing_first + dropped, outgoing));
            let mut order = self.detached_order.borrow_mut();
            order.retain(|index| *index != previous);
            order.push(previous);
            // Keep the list bounded: an old conversation is rebuilt on demand
            // rather than held alive for the rest of the session.
            while order.len() > DETACHED_CONVERSATIONS {
                let oldest = order.remove(0);
                self.detached.borrow_mut().remove(&oldest);
            }
        }

        let incoming = self.detached.borrow_mut().remove(&target);
        self.detached_order
            .borrow_mut()
            .retain(|index| *index != target);
        // A miss means this is the first visit since the entry was dropped (or
        // never built); the caller packs a fresh window below.
        match incoming {
            Some((incoming_first, incoming)) => {
                let count = incoming.len();
                let attached_started = std::time::Instant::now();
                for render in &incoming {
                    if let Some(frame) = &render.frame {
                        self.messages.pack_start(frame, false, false, 0);
                    }
                }
                self.first.set(incoming_first);
                *self.turns.borrow_mut() = incoming;
                if std::env::var_os("EB_DEBUG").is_some() {
                    eprintln!(
                        "fill_messages: re-attached {count} frames for task {target} from turn \
                         {incoming_first} removed={removed_ms:.1}ms attached={:.1}ms",
                        attached_started.elapsed().as_secs_f64() * 1000.0
                    );
                }
            }
            None => {
                let start = total.saturating_sub(ATTACH_TURNS);
                self.first.set(start);
                if std::env::var_os("EB_DEBUG").is_some() {
                    eprintln!(
                        "fill_messages: task {target} has no cached frames, window starts at \
                         turn {start} (removed={removed_ms:.1}ms)"
                    );
                }
            }
        }
        let reveal_started = std::time::Instant::now();
        self.messages.set_child_visible(true);
        if std::env::var_os("EB_DEBUG").is_some() {
            eprintln!(
                "fill_messages: reveal cost {:.1}ms",
                reveal_started.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    /// Attach the slice of transcript immediately above what is rendered.
    ///
    /// Runs inside a repaint, not inside the scroll handler: the new frames need
    /// the same `TurnRender` bookkeeping as the normal packing path, and the
    /// handler only raises `pending_load` and asks the shell to repaint.
    fn load_earlier(&self, task: &Task) {
        let first = self.first.get();
        let start = first.saturating_sub(ATTACH_STEP);
        if start >= first {
            return;
        }
        // The frames land above the current top, so the reader would be pushed
        // down by exactly their height. Record the line under the top edge and
        // put the view back on it once the new frames have been measured.
        let adjustment = self.scroll.vadjustment();
        let (value, upper_before) = (adjustment.value(), adjustment.upper());
        match self.turns.borrow().first().and_then(|r| r.frame.clone()) {
            Some(frame) => {
                let line = frame.allocation().y() as f64 - value;
                *self.anchor.borrow_mut() = Some((frame, line));
            }
            // Nothing mounted but turns to attach: the list fills from empty and
            // there is no line to lose. Clearing the anchor keeps a stale frame
            // from pulling the view later.
            None => {
                self.anchor.borrow_mut().take();
            }
        }
        self.anchor_passes.set(0);
        let owner = artifact_owner(task);
        // Held invisible so the whole prepend is one layout pass, like a switch.
        self.messages.set_child_visible(false);
        let built_started = std::time::Instant::now();
        let mut built: Vec<TurnRender> = Vec::new();
        for index in start..first {
            let mut state = turn_state(&task.turns[index]);
            if owner == Some(index) {
                state.sig = state.sig.wrapping_add(artifact_sig(&task.artifacts));
            }
            let artifacts: &[Artifact] = if owner == Some(index) {
                &task.artifacts
            } else {
                &[]
            };
            built.push(build_turn(
                self,
                &task.turns[index],
                &state,
                task.started_at,
                prompt_before(&task.turns, index),
                artifacts,
            ));
        }
        // `build_turn` packs at the end; the new slice belongs above the window.
        for (position, render) in built.iter().enumerate() {
            if let Some(frame) = &render.frame {
                self.messages.reorder_child(frame, position as i32);
            }
        }
        self.turns.borrow_mut().splice(0..0, built);
        self.first.set(start);
        self.messages.set_child_visible(true);
        let built_ms = built_started.elapsed().as_secs_f64() * 1000.0;

        let anchor = self.anchor.clone();
        let guard = self.auto_scroll.clone();
        let adjustment = adjustment.clone();
        gtk::glib::idle_add_local_once(move || {
            // The anchor is this load's token: a switch or a gesture the reader
            // made themselves takes it away, and then the view must stay where
            // they left it instead of jumping by a slice that no longer applies.
            if anchor.borrow().is_none() {
                return;
            }
            // Guarded: this jump is the view keeping *its* place, not the reader
            // leaving the bottom, so it must not touch `pinned`.
            let upper = adjustment.upper();
            let target = value + (upper - upper_before);
            guard.set(true);
            adjustment.set_value(target);
            guard.set(false);
            if std::env::var_os("EB_DEBUG").is_some() {
                eprintln!(
                    "load_earlier: hold {value:.0}px -> {target:.0}px (upper {upper_before:.0} \
                     -> {upper:.0}, page {:.0})",
                    adjustment.page_size()
                );
            }
        });
        if std::env::var_os("EB_DEBUG").is_some() {
            eprintln!(
                "fill_messages: loaded {} earlier turns up to {first} in {built_ms:.1}ms, \
                 mounted={}",
                first - start,
                self.turns.borrow().len(),
            );
        }
    }
}

/// Live long bubble is 720 inside the 752 column: the right edge stays flush
/// with the message column, and 32px stays empty on the left. The same inset
/// applies when the artifact pane narrows the column to 500.
const USER_BUBBLE_INSET: i32 = 32;

/// The small clipboard button used under every turn. Copying is invisible, so
/// the glyph flashes to a check to confirm it worked.
fn copy_button(value: &str, tooltip: &str) -> gtk::Button {
    let copy = icon_button("⧉", "eb-icon-mini", tooltip);
    let value = value.to_string();
    copy.connect_clicked(move |button| {
        let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        clipboard.set_text(&value);
        clipboard.store();
        flash(button, "✓", "Copied");
    });
    copy
}

/// "Copy answer" copies the model's prose only. Tool, exec, diff and plan
/// blocks already expose their own content; reasoning is explicitly not part
/// of the answer.
fn answer_text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for block in blocks {
        if let Block::Text(value) = block {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(value);
        }
    }
    out
}

/// Measured: 16px above the bubble, then a 28px toolbar in the frame's bottom
/// padding. The bubble hugs short text and grows until it is
/// `USER_BUBBLE_INSET` short of the message column (live: 99px and 720px
/// inside a 752px frame).
fn user_turn(value: &str, time: &str, actions: &TurnActions) -> gtk::EventBox {
    let outer = gtk::EventBox::new();
    class(&outer, "eb-frame");
    class(&outer, "eb-frame-user");

    let frame = vbox(0);

    let line = hbox(0);
    line.set_halign(gtk::Align::Fill);

    // Same clamp as agent prose: a pasted URL or a CJK run has no spaces, and
    // the label's minimum is then the whole run. The bubble stays one pixel
    // of minimum and rewraps to the width the column actually gives it.
    let bubble = wrap_column(0);
    class(&bubble, "eb-bubble");
    bubble.set_halign(gtk::Align::End);
    bubble.set_hexpand(false);
    // Outside the bubble, so a long one stops 32px short of the column while
    // its right edge stays on the column's right edge. A short one still hugs
    // its text.
    bubble.set_margin_start(USER_BUBBLE_INSET);
    // A wrapping TextView reports almost no natural width, so the bubble
    // collapsed to a few characters. A wrapping label's minimum is one
    // character and its natural width is the text, capped at the column, and
    // it rewraps to whatever width it is actually given.
    let label = gtk::Label::new(Some(value));
    label.set_selectable(true);
    label.set_line_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(1.0);
    label.set_justify(gtk::Justification::Right);
    label.set_width_chars(1);
    label.set_max_width_chars(bubble_max_chars(&label));
    class(&label, "eb-bubble-text");
    bubble.pack_start(&label, false, false, 0);
    line.pack_end(&bubble, false, false, 0);
    frame.pack_start(&line, false, false, 0);

    let toolbar = hbox(8);
    class(&toolbar, "eb-message-toolbar");
    toolbar.set_halign(gtk::Align::End);
    let stamp = text(time, "eb-message-time");
    toolbar.pack_start(&stamp, false, false, 0);

    let copy = copy_button(value, "Copy message");
    toolbar.pack_start(&copy, false, false, 0);

    let edit = icon_button("✏", "eb-icon-mini", "Edit and resend");
    let value = value.to_string();
    let on_edit = actions.edit.clone();
    edit.connect_clicked(move |_| on_edit(value.clone()));
    toolbar.pack_start(&edit, false, false, 0);

    frame.pack_start(&toolbar, false, false, 0);

    outer.add(&frame);
    attach_hover_reveal(&outer, &stamp);

    outer
}

/// Reveal a turn's timestamp only while the pointer is over the message.
///
/// The reveal lives on the turn's `EventBox`: it owns a GdkWindow, so the
/// crossing events fire reliably, which a plain `GtkBox` (no window, no
/// `:hover` state) never gets. The reveal is `set_opacity`, so the label keeps
/// its layout width and the toolbar never reflows as the pointer moves.
fn attach_hover_reveal<W: gtk::prelude::WidgetExt>(area: &W, stamp: &gtk::Label) {
    stamp.set_opacity(0.0);
    area.add_events(
        gtk::gdk::EventMask::ENTER_NOTIFY_MASK | gtk::gdk::EventMask::LEAVE_NOTIFY_MASK,
    );
    let reveal = stamp.clone();
    area.connect_enter_notify_event(move |_, event| {
        // Crossing into a child (a button, the selectable bubble) must not
        // hide what the frame already revealed.
        if event.detail() != gtk::gdk::NotifyType::Inferior {
            reveal.set_opacity(1.0);
        }
        gtk::glib::Propagation::Proceed
    });
    let hide = stamp.clone();
    area.connect_leave_notify_event(move |_, event| {
        if event.detail() != gtk::gdk::NotifyType::Inferior {
            hide.set_opacity(0.0);
        }
        gtk::glib::Propagation::Proceed
    });
}

/// Character cap whose pixel width reaches the closed message column.
///
/// Short text stays narrower than this, because the label's natural width is
/// the text itself until it exceeds the cap. `width_chars(1)` is the minimum,
/// so the open-artifact column (500) can still shrink a long bubble.
fn bubble_max_chars(label: &gtk::Label) -> i32 {
    let char_px = label
        .layout()
        .map(|layout| {
            let context = layout.context();
            let desc = context.font_description();
            let metrics = context.metrics(desc.as_ref(), None);
            metrics
                .approximate_char_width()
                .max(metrics.approximate_digit_width())
                / gtk::pango::SCALE
        })
        .unwrap_or(8)
        .max(1);
    (spec::MESSAGE_WIDTH_CLOSED + char_px - 1) / char_px
}

/// Measured: a 500 wide frame; a 24.5px header with a 10px gap between the
/// agent mark and its name (14px/600); then the blocks.
struct AgentTurnParts {
    frame: gtk::EventBox,
    body: gtk::Box,
}

/// Local `HH:MM` for a turn's unix-second stamp, revealed under the message
/// on hover. Out-of-range epochs fall back to no timestamp rather than 00:00.
fn clock_hhmm(epoch: &u64) -> String {
    gtk::glib::DateTime::from_unix_local(i64::try_from(*epoch).unwrap_or(0))
        .map(|time| format!("{:02}:{:02}", time.hour(), time.minute()))
        .unwrap_or_default()
}

fn agent_turn(
    name: &str,
    status: Option<&str>,
    blocks: &[Block],
    streaming: bool,
    artifacts: &[Artifact],
    time: Option<&str>,
    actions: &TurnActions,
) -> AgentTurnParts {
    // The EventBox owns a GdkWindow, which is what the hover timestamp's
    // crossing events hang on; a plain box would never see them.
    let outer = gtk::EventBox::new();
    class(&outer, "eb-frame");

    let frame = vbox(0);

    let header = hbox(10);
    header.set_valign(gtk::Align::Center);

    // EvoBuddy's own agent mark (embedded PNG), not the product mascot.
    let logo = crate::icons::named("agent", 24, "eb-agent-logo");
    header.pack_start(&logo, false, false, 0);
    let agent_name = text(name, "eb-agent-name");
    agent_name.set_width_chars(1);
    agent_name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    header.pack_start(&agent_name, false, false, 0);
    frame.pack_start(&header, false, false, 0);

    if let Some(status) = status {
        // Measured: 15px at rgba(0,0,0,.5), directly under the header. Errors
        // arrive through this line too, so it must be able to wrap instead of
        // becoming the window's minimum width. WorkBuddy marks the line with a
        // trailing chevron.
        let line = text(&format!("{status} ›"), "eb-agent-status");
        line.set_margin_top(8);
        line.set_line_wrap(true);
        line.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        line.set_width_chars(1);
        line.set_xalign(0.0);
        frame.pack_start(&line, false, false, 0);
    }

    // Blocks are packed by `pack_turn`, which also records them for in-place
    // streaming updates; packing here as well would render everything twice.
    let body = vbox(0);
    body.set_margin_top(12);
    frame.pack_start(&body, false, false, 0);

    // Files the agent wrote, as clickable cards under its prose. Only the
    // owner turn receives a non-empty slice (see `artifact_owner`).
    if !streaming && !artifacts.is_empty() {
        pack_artifact_cards(&frame, artifacts, actions);
    }

    if streaming {
        frame.pack_start(&streaming_footer(), false, false, 0);
    } else {
        // The product's message-action row was never captured, so this is the
        // host's own set (ui-spec §10 discipline): actions that really work
        // copy, edit and regenerate; the two rating buttons say what they
        // would do instead of pretending to store a rating.
        let toolbar = hbox(8);
        class(&toolbar, "eb-message-toolbar");
        toolbar.set_halign(gtk::Align::Start);

        let answer = answer_text(blocks);
        if !answer.is_empty() {
            let copy = copy_button(&answer, "Copy answer");
            toolbar.pack_start(&copy, false, false, 0);
        }

        let like = icon_button("👍", "eb-icon-mini", "Good answer");
        attach_notice(&like, "This UI-only build does not send answer feedback.");
        toolbar.pack_start(&like, false, false, 0);

        let dislike = icon_button("👎", "eb-icon-mini", "Bad answer");
        attach_notice(&dislike, "This UI-only build does not send answer feedback.");
        toolbar.pack_start(&dislike, false, false, 0);

        if let Some(prompt) = &actions.prompt {
            let redo = icon_button("↻", "eb-icon-mini", "Regenerate answer");
            let submit = actions.submit.clone();
            let prompt = prompt.clone();
            redo.connect_clicked(move |_| submit(prompt.clone()));
            toolbar.pack_start(&redo, false, false, 0);
        }

        let mut stamp = None;
        if let Some(time) = time {
            let label = text(time, "eb-message-time");
            toolbar.pack_start(&label, false, false, 0);
            stamp = Some(label);
        }

        frame.pack_start(&toolbar, false, false, 0);
        if let Some(stamp) = &stamp {
            attach_hover_reveal(&outer, stamp);
        }
    }

    outer.add(&frame);

    AgentTurnParts { frame: outer, body }
}

/// The last settled agent turn owns the artifact card row; earlier and
/// streaming turns never show it.
fn artifact_owner(task: &Task) -> Option<usize> {
    if task.artifacts.is_empty() {
        return None;
    }
    task.turns
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, turn)| match turn {
            Turn::Agent {
                streaming: false,
                ..
            } => Some(index),
            _ => None,
        })
}

/// Identity of the card row content: names plus sizes, so a rewrite that
/// changes a file's length repaints the byte count.
fn artifact_sig(artifacts: &[Artifact]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for artifact in artifacts {
        artifact.name.hash(&mut hasher);
        artifact.code.len().hash(&mut hasher);
    }
    hasher.finish()
}

/// Measured (DPR 2, capture 1656x352): two 370x64 cards with a 12px gap,
/// filling the 752px panel-closed column; then the "View all artifacts (N)"
/// link ~26px below the cards. Max two cards; the link only appears when more
/// files exist than are shown.
fn pack_artifact_cards(frame: &gtk::Box, artifacts: &[Artifact], actions: &TurnActions) {
    let row = hbox(12);
    row.set_homogeneous(true);
    row.set_margin_top(12);
    class(&row, "eb-artifact-row");
    for (index, artifact) in artifacts.iter().enumerate().take(2) {
        let card = artifact_card(artifact, index, actions);
        row.pack_start(&card, true, true, 0);
    }
    frame.pack_start(&row, false, false, 0);

    if artifacts.len() > 2 {
        let link = flat_button("eb-artifact-viewall");
        link.set_halign(gtk::Align::Start);
        link.set_margin_top(26);
        let inner = hbox(4);
        inner.pack_start(
            &text(&format!("View all artifacts ({})", artifacts.len()), "eb-artifact-viewall-label"),
            false,
            false,
            0,
        );
        if let Some(chevron) = crate::icons::image("chevron-right", 12) {
            chevron.set_valign(gtk::Align::Center);
            inner.pack_start(&chevron, false, false, 0);
        }
        link.add(&inner);
        link.set_tooltip_text(Some("Open the artifact panel"));
        let open = actions.artifact.clone();
        let newest = artifacts.len() - 1;
        link.connect_clicked(move |_| open(newest));
        frame.pack_start(&link, false, false, 0);
    }
}

/// One file card: 24px blue badge, name (15px/600) over size (13px tertiary),
/// 14px left inset and a 12px icon-to-text gap, per the capture. Clicking it
/// opens the artifact panel with this file's tab selected.
fn artifact_card(artifact: &Artifact, index: usize, actions: &TurnActions) -> gtk::Button {
    let card = flat_button("eb-artifact-card");
    card.set_tooltip_text(Some(&format!("Open {} in the artifact panel", artifact.name)));

    let inner = hbox(12);
    // `image` is None when gdk-pixbuf cannot decode the PNG. The bare Debian
    // 10 bundle is one of those hosts, and unwrapping here aborts the process
    // the first time a card is built. `named` keeps a label in that case.
    let badge_name = if is_code_file(&artifact.name) {
        "artifact-code"
    } else {
        "artifact-unknown"
    };
    let badge = crate::icons::named(badge_name, 24, "eb-artifact-badge");
    badge.set_valign(gtk::Align::Center);
    inner.pack_start(&badge, false, false, 0);

    let lines = vbox(2);
    lines.set_valign(gtk::Align::Center);
    lines.set_hexpand(true);
    let name = ellipsized(&artifact.name, "eb-artifact-name");
    name.set_hexpand(true);
    lines.pack_start(&name, false, false, 0);
    lines.pack_start(
        &text(&format_bytes(artifact.code.len()), "eb-artifact-size"),
        false,
        false,
        0,
    );
    inner.pack_start(&lines, true, true, 0);
    card.add(&inner);

    let open = actions.artifact.clone();
    card.connect_clicked(move |_| open(index));
    card
}

/// Extensions the product's "</>" badge covers, per the capture's code files.
fn is_code_file(name: &str) -> bool {
    let Some((_, extension)) = name.rsplit_once('.') else {
        return false;
    };
    if extension.eq_ignore_ascii_case(name) {
        return false;
    }
    [
        "c", "h", "cc", "cpp", "py", "js", "ts", "tsx", "jsx", "java", "go", "rs", "rb", "sh",
        "css", "html", "json", "toml", "yaml", "yml", "md", "sql", "kt", "swift",
    ]
    .contains(&extension.to_ascii_lowercase().as_str())
}

/// The capture shows "131 B" and "32.6 KB": bytes under a KiB, one-decimal
/// KiB/MiB above.
fn format_bytes(len: usize) -> String {
    if len < 1024 {
        format!("{len} B")
    } else if len < 1024 * 1024 {
        format!("{:.1} KB", len as f64 / 1024.0)
    } else {
        format!("{:.1} MB", len as f64 / (1024.0 * 1024.0))
    }
}

/// Tool, diff, table, plan, and group blocks keep the measured 14px rhythm.
/// A text block does not: live markdown has no outside margin. Its 26px line
/// box and 12px paragraph gap are inside the text view.
fn block_widget(block: &Block) -> gtk::Widget {
    let widget = block_widget_raw(block);
    if !matches!(block, Block::Text(_)) {
        widget.set_margin_top(14);
        widget.set_margin_bottom(14);
    }
    widget
}

/// A block without the inter-block 14px rhythm, used inside a tool group where
/// the product keeps rows on a compact 26px line.
fn block_widget_raw(block: &Block) -> gtk::Widget {
    match block {
        Block::Text(value) => crate::markdown::render(value).upcast(),
        Block::Thought { text, collapsed } => thought_block(text, *collapsed).upcast(),
        Block::Tool {
            status,
            primary,
            detail,
        } => tool_row(status, primary, detail.as_deref()).upcast(),
        Block::Diff {
            path,
            added,
            removed,
            lines,
        } => diff_block(path, *added, *removed, lines).upcast(),
        Block::Exec {
            command,
            output,
            exit_code,
        } => exec_block(command, output, *exit_code).upcast(),
        Block::Group {
            title,
            blocks,
            collapsed,
        } => group_block(title, blocks, *collapsed).upcast(),
        Block::Plan { entries } => plan_block(entries).upcast(),
        Block::Table { headers, rows } => table(headers, rows).upcast(),
    }
}

/// Measured: a single 26.25px line, 8px gap — status, command, then the muted
/// right-hand detail (duration or diff size).
fn tool_row(status: &str, primary: &str, detail: Option<&str>) -> gtk::Box {
    let row = hbox(8);
    class(&row, "eb-tool-row");

    row.pack_start(&glyph("▸"), false, false, 0);
    let status = text(status, "eb-tool-status");
    status.set_width_chars(1);
    row.pack_start(&status, false, false, 0);
    let primary = ellipsized(primary, "eb-tool-primary");
    primary.set_width_chars(1);
    row.pack_start(&primary, true, true, 0);
    if let Some(detail) = detail {
        let detail = ellipsized(detail, "eb-tool-detail");
        detail.set_width_chars(1);
        detail.set_max_width_chars(40);
        row.pack_end(&detail, false, false, 0);
    }

    row
}

/// Measured: a rounded card with a tinted header row, hairline separators and
/// 8px/12px cell padding.
/// The light-green diff card: 752x278 (12 lines) in the capture, a 16px
/// radius, and 21px monospace lines with 16px side padding.
fn diff_block(path: &str, added: u32, removed: u32, lines: &[DiffLine]) -> gtk::Box {
    let card = vbox(0);
    class(&card, "eb-diff-card");
    class(&card, "eb-radius-2xl");
    card.set_tooltip_text(Some(&format!("{path}  +{added} −{removed}")));

    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    // Keep long code lines from becoming the window's minimum width.
    scroll.set_propagate_natural_width(false);
    scroll.set_max_content_width(spec::MESSAGE_WIDTH_CLOSED);
    let content_height = (lines.len() as i32 * 21).clamp(21, 252);
    scroll.set_min_content_height(content_height);
    scroll.set_size_request(-1, content_height);
    class(&scroll, "eb-diff-scroll");

    let rows = vbox(0);
    for line in lines {
        let (sign, text_value, kind) = match line {
            DiffLine::Add(value) => ("+", value, "eb-diff-add"),
            DiffLine::Remove(value) => ("−", value, "eb-diff-remove"),
            DiffLine::Context(value) => (" ", value, "eb-diff-context"),
        };
        let row = hbox(6);
        row.set_size_request(-1, 21);
        class(&row, "eb-diff-line");
        class(&row, kind);

        let sign = text(sign, "eb-diff-sign");
        sign.set_size_request(14, -1);
        class(&sign, "eb-fs-3");
        row.pack_start(&sign, false, false, 0);

        let value = text(text_value, "eb-diff-code");
        class(&value, "eb-fs-3");
        value.set_xalign(0.0);
        value.set_hexpand(true);
        value.set_width_chars(1);
        value.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.pack_start(&value, true, true, 0);

        rows.pack_start(&row, false, false, 0);
    }
    scroll.add(&rows);
    card.pack_start(&scroll, true, true, 0);

    let height = (content_height + 26).clamp(47, 278);
    card.set_size_request(-1, height);
    card
}

/// The command card: 752x300 with a 16px radius, 12/16 padding, and a
/// vertically scrolling monospace body.
fn exec_block(command: &str, output: &str, exit_code: Option<i32>) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    // Long command output is horizontally scrollable, not a reason to widen the
    // conversation column (which used to drag the whole window).
    scroll.set_propagate_natural_width(false);
    scroll.set_max_content_width(spec::MESSAGE_WIDTH_CLOSED);
    scroll.set_min_content_width(1);
    scroll.set_hexpand(true);
    scroll.set_min_content_height(300);
    scroll.set_size_request(1, 300);
    class(&scroll, "eb-exec-card");
    class(&scroll, "eb-radius-2xl");
    if let Some(code) = exit_code {
        scroll.set_tooltip_text(Some(&format!("exit code {code}")));
    }

    let body = vbox(4);
    class(&body, "eb-exec-body");
    body.set_margin_top(spec::spacing::SPACING_4);
    body.set_margin_bottom(spec::spacing::SPACING_4);
    body.set_margin_start(spec::spacing::SPACING_5);
    body.set_margin_end(spec::spacing::SPACING_5);

    let title = text("bash", "eb-exec-title");
    class(&title, "eb-fs-3");
    body.pack_start(&title, false, false, 0);

    let command = text(command, "eb-exec-command");
    class(&command, "eb-fs-3");
    command.set_ellipsize(gtk::pango::EllipsizeMode::End);
    command.set_width_chars(1);
    body.pack_start(&command, false, false, 0);

    let output_view = gtk::TextView::new();
    output_view.set_editable(false);
    output_view.set_cursor_visible(false);
    output_view.set_monospace(true);
    output_view.set_wrap_mode(gtk::WrapMode::Char);
    output_view.set_top_margin(0);
    output_view.set_bottom_margin(0);
    output_view.set_left_margin(0);
    output_view.set_right_margin(0);
    class(&output_view, "eb-exec-output");
    class(&output_view, "eb-fs-3");
    if let Some(buffer) = output_view.buffer() {
        buffer.set_text(output);
    }
    let lines = output.lines().count().max(1) as i32;
    output_view.set_hexpand(true);
    output_view.set_size_request(1, lines * 18);
    body.pack_start(&output_view, false, false, 0);

    scroll.add(&body);
    scroll
}

/// A collapsible tool group: a 26.25px header with a chevron/title and the
/// nested blocks below. The header is a flat button; the group starts in the
/// state the plugin declared and toggles locally.
fn plan_block(entries: &[PlanEntry]) -> gtk::Box {
    let card = wrap_column(4);
    class(&card, "eb-plan");
    let title = text("Plan", "eb-plan-title");
    card.pack_start(&title, false, false, 0);
    for entry in entries {
        let row = hbox(8);
        class(&row, "eb-plan-row");
        let icon = match entry.status.as_str() {
            "completed" => "check",
            "in_progress" => "ring",
            _ => "chevron-right",
        };
        row.pack_start(&crate::icons::named(icon, 16, "eb-plan-icon"), false, false, 0);
        let label = text(&entry.content, "eb-plan-text");
        label.set_line_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_width_chars(1);
        label.set_xalign(0.0);
        row.pack_start(&label, true, true, 0);
        card.pack_start(&row, false, false, 0);
    }
    card
}

/// Streamed reasoning. While the thought is the live tail the body stays
/// open and grows in place. Once thinking ends the body is folded behind
/// the header; a click opens it again. The product look is uncaptured, so
/// the header reuses the tool-group chevron.
fn thought_block(value: &str, collapsed: bool) -> gtk::Box {
    let root = vbox(0);
    class(&root, "eb-thought-block");

    let header = flat_button("eb-group-header");
    class(&header, "eb-thought-header");
    header.set_tooltip_text(Some("Toggle reasoning"));
    let row = hbox(8);
    row.set_valign(gtk::Align::Center);
    let title = text("Thinking", "eb-group-title");
    title.set_width_chars(1);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let chevron = text(if collapsed { "›" } else { "⌄" }, "eb-group-chevron");
    row.pack_start(&title, false, false, 0);
    row.pack_start(&chevron, false, false, 0);
    header.add(&row);

    let label = text(value, "eb-thought");
    label.set_line_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_width_chars(1);
    label.set_xalign(0.0);
    label.set_tooltip_text(Some("Agent reasoning, not part of the answer"));
    let content = wrap_column(0);
    class(&content, "eb-thought-body");
    content.pack_start(&label, false, false, 0);
    let expanded = Rc::new(Cell::new(!collapsed));
    if collapsed {
        hide_from_show_all(&content);
    }
    header.connect_clicked({
        let content = content.clone();
        let chevron = chevron.clone();
        let expanded = expanded.clone();
        move |_| {
            let next = !expanded.get();
            expanded.set(next);
            content.set_visible(next);
            chevron.set_text(if next { "⌄" } else { "›" });
        }
    });

    root.pack_start(&header, false, false, 0);
    root.pack_start(&content, false, false, 0);
    root
}

fn group_block(title: &str, blocks: &[Block], collapsed: bool) -> gtk::Box {
    let root = vbox(0);
    class(&root, "eb-group");

    let header = flat_button("eb-group-header");
    header.set_tooltip_text(Some("Toggle tool details"));
    let row = hbox(8);
    row.set_valign(gtk::Align::Center);
    let chevron = text(if collapsed { "›" } else { "⌄" }, "eb-group-chevron");
    let group_title = text(title, "eb-group-title");
    group_title.set_width_chars(1);
    group_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    row.pack_start(&group_title, false, false, 0);
    row.pack_start(&chevron, false, false, 0);
    header.add(&row);

    let content = vbox(0);
    class(&content, "eb-group-content");
    for block in blocks {
        content.pack_start(&block_widget_raw(block), false, false, 0);
    }
    let expanded = Rc::new(Cell::new(!collapsed));
    if collapsed {
        hide_from_show_all(&content);
    }
    header.connect_clicked({
        let content = content.clone();
        let chevron = chevron.clone();
        let expanded = expanded.clone();
        move |_| {
            let next = !expanded.get();
            expanded.set(next);
            content.set_visible(next);
            chevron.set_text(if next { "⌄" } else { "›" });
        }
    });

    root.pack_start(&header, false, false, 0);
    root.pack_start(&content, false, false, 0);
    root
}

fn table(headers: &[String], rows: &[Vec<String>]) -> gtk::Box {
    let card = vbox(0);
    class(&card, "eb-table");
    card.set_halign(gtk::Align::Fill);

    let grid = gtk::Grid::new();
    grid.set_column_spacing(0);
    grid.set_row_spacing(0);
    class(&grid, "eb-table-grid");

    for (column, header) in headers.iter().enumerate() {
        let cell = table_cell(header, "eb-th", true);
        grid.attach(&cell, column as i32, 0, 1, 1);
    }
    for (row, values) in rows.iter().enumerate() {
        for (column, value) in values.iter().enumerate() {
            let cell = table_cell(value, "eb-td", false);
            grid.attach(&cell, column as i32, row as i32 + 1, 1, 1);
        }
    }

    card.pack_start(&grid, false, false, 0);
    card
}

fn table_cell(value: &str, class_name: &str, header: bool) -> gtk::Box {
    let cell = hbox(0);
    class(&cell, class_name);
    let label = gtk::Label::new(Some(value));
    label.set_selectable(true);
    label.set_xalign(0.0);
    label.set_line_wrap(true);
    label.set_width_chars(1);
    label.set_max_width_chars(40);
    if header {
        label.set_hexpand(true);
    }
    cell.pack_start(&label, true, true, 0);
    cell.set_hexpand(true);
    cell
}

/// Measured: `conversation-streaming-footer`, a 26.25px line. Only shown while
/// a turn is still arriving — which, with no agent attached, never happens.
fn streaming_footer() -> gtk::Box {
    let row = hbox(8);
    class(&row, "eb-streaming");
    row.pack_start(&glyph("◌"), false, false, 0);
    row.pack_start(&text("Thinking", "eb-streaming-text"), false, false, 0);
    row
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;
    use crate::model::Block;

    /// The streaming hot path: a text block that only grew must keep its
    /// TextView widget (same instance) and get its buffer rewritten in place,
    /// instead of tearing the widget down and repainting from scratch.
    #[test]
    fn sync_blocks_updates_growing_text_in_place() {
        if crate::widgets::try_init_gtk_for_test().is_none() {
            eprintln!("skipping: gtk needs the main thread and a display");
            return;
        }
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let before = Block::Text("hello world".into());
        let widget = block_widget(&before);
        body.pack_start(&widget, false, false, 0);
        let widget_ptr = widget.as_ptr();
        let mut block_sigs = vec![block_sig(&before)];
        let mut widgets = vec![widget];

        let after = Block::Text("hello world, streamed further".into());
        sync_blocks(&body, &mut block_sigs, &mut widgets, &[after]);

        assert_eq!(widgets.len(), 1, "the growing text block reuses its widget");
        assert!(
            std::ptr::eq(widgets[0].as_ptr(), widget_ptr),
            "the text block widget must survive the update"
        );
        let text_view = text_view_in(&widgets[0]);
        let buffer = text_view.buffer().expect("buffer");
        let text = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .unwrap_or_default();
        assert!(
            text.contains("streamed further"),
            "buffer must carry the streamed content, got {text:?}"
        );
    }

    /// Reasoning stays on its widget while chunks arrive, then the fold
    /// replaces that widget with one whose body is hidden until the header
    /// is clicked.
    #[test]
    fn a_streaming_thought_folds_when_thinking_ends() {
        if crate::widgets::try_init_gtk_for_test().is_none() {
            eprintln!("skipping: gtk needs the main thread and a display");
            return;
        }
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let open = Block::Thought {
            text: "one".into(),
            collapsed: false,
        };
        let widget = block_widget(&open);
        body.pack_start(&widget, false, false, 0);
        let widget_ptr = widget.as_ptr();
        let mut block_sigs = vec![block_sig(&open)];
        let mut widgets = vec![widget];

        let grown = Block::Thought {
            text: "one two".into(),
            collapsed: false,
        };
        sync_blocks(&body, &mut block_sigs, &mut widgets, &[grown]);
        assert!(
            std::ptr::eq(widgets[0].as_ptr(), widget_ptr),
            "a growing thought must keep its label"
        );
        let live = widget_by_class(&widgets[0], "eb-thought")
            .expect("thought label")
            .downcast::<gtk::Label>()
            .expect("thought label");
        assert_eq!(live.text().as_str(), "one two");
        assert!(
            widget_by_class(&widgets[0], "eb-thought-body")
                .expect("thought body")
                .is_visible(),
            "the stream stays on screen"
        );

        let folded = Block::Thought {
            text: "one two".into(),
            collapsed: true,
        };
        sync_blocks(&body, &mut block_sigs, &mut widgets, &[folded]);
        assert!(
            !std::ptr::eq(widgets[0].as_ptr(), widget_ptr),
            "folding replaces the open thought"
        );
        let folded_body = widget_by_class(&widgets[0], "eb-thought-body").expect("folded body");
        assert!(!folded_body.is_visible(), "finished thinking is folded");
        let folded_label = widget_by_class(&widgets[0], "eb-thought")
            .expect("folded thought label")
            .downcast::<gtk::Label>()
            .expect("thought label");
        assert_eq!(folded_label.text().as_str(), "one two");
        widget_by_class(&widgets[0], "eb-thought-header")
            .expect("thought header")
            .downcast::<gtk::Button>()
            .expect("header button")
            .clicked();
        assert!(folded_body.is_visible(), "the header opens the thought");
    }

    #[test]
    fn user_bubble_uses_the_column_for_long_text() {
        if crate::widgets::try_init_gtk_for_test().is_none() {
            eprintln!("skipping: gtk needs the main thread and a display");
            return;
        }
        ensure_shell_css();
        let short = allocated_bubble("Hi", 752);
        let long = allocated_bubble(&"word ".repeat(40), 752);
        let narrowed = allocated_bubble(&"word ".repeat(40), 500);
        assert!(
            short.width < 280,
            "a short bubble hugs its text, got {}",
            short.width
        );
        assert_eq!(
            long.width,
            752 - USER_BUBBLE_INSET,
            "a long bubble stays 32px inside a 752 column"
        );
        assert_eq!(
            narrowed.width,
            500 - USER_BUBBLE_INSET,
            "a long bubble stays 32px inside a 500 column"
        );
        assert_eq!(short.right_gap, 0, "a short bubble is right-aligned");
        assert_eq!(long.right_gap, 0, "a long bubble is right-aligned");
        assert_eq!(narrowed.right_gap, 0, "a narrowed bubble is right-aligned");
    }

    /// A pasted URL and a CJK run have no spaces. They used to set the message
    /// column's minimum to the whole line, so the transcript grew a horizontal
    /// scrollbar instead of wrapping. Code stays on one line and scrolls inside
    /// its own card.
    #[test]
    fn long_history_text_wraps_inside_the_column() {
        if crate::widgets::try_init_gtk_for_test().is_none() {
            eprintln!("skipping: gtk needs the main thread and a display");
            return;
        }
        ensure_shell_css();
        let url = format!("https://example.com/{}", "segment".repeat(40));
        let cjk = "甲".repeat(120);
        let code = "x".repeat(400);
        let agent = format!("{cjk}\n\n```\n{code}\n```");

        for width in [752, 420] {
            let messages = gtk::Box::new(gtk::Orientation::Vertical, 0);
            messages.pack_start(&user_turn(&url, "12:00", &TurnActions::detached()), false, false, 0);
            messages.pack_start(&block_widget(&Block::Text(agent.clone())), false, false, 0);
            messages.pack_start(
                &block_widget(&Block::Thought {
                    text: url.clone(),
                    collapsed: false,
                }),
                false,
                false,
                0,
            );
            messages.pack_start(
                &block_widget(&Block::Plan {
                    entries: vec![PlanEntry {
                        content: url.clone(),
                        status: "pending".into(),
                    }],
                }),
                false,
                false,
                0,
            );
            messages.set_size_request(width, -1);

            let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
            scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::External);
            scroll.set_propagate_natural_width(true);
            scroll.set_min_content_width(1);
            scroll.set_max_content_width(width);
            scroll.add(&messages);

            let window = gtk::OffscreenWindow::new();
            window.add(&scroll);
            window.set_default_size(width, 800);
            window.show_all();
            drain_gtk();

            let mut height = 800;
            for _ in 0..6 {
                scroll.size_allocate(&gtk::Allocation::new(0, 0, width, height));
                drain_gtk();
                let (_, nat) = messages.preferred_height();
                height = nat.clamp(200, 4000);
            }

            let horizontal = scroll.hadjustment();
            assert!(
                horizontal.upper() <= horizontal.page_size() + 1.0,
                "column {width} scrolls sideways: page {:.0} upper {:.0}",
                horizontal.page_size(),
                horizontal.upper()
            );

            let bubble = widget_by_class(messages.upcast_ref(), "eb-bubble").expect("user bubble");
            assert!(
                bubble.allocation().width() <= width - USER_BUBBLE_INSET,
                "user bubble is wider than the {width} column: {}",
                bubble.allocation().width()
            );
            let label = widget_by_class(messages.upcast_ref(), "eb-bubble-text")
                .expect("bubble text")
                .downcast::<gtk::Label>()
                .expect("bubble text is a label");
            let lines = label.layout().map(|layout| layout.line_count()).unwrap_or(1);
            assert!(
                lines > 1,
                "user URL stayed on {lines} line(s) in a {width} column"
            );

            let view = widget_by_class(messages.upcast_ref(), "eb-md-view")
                .expect("agent prose")
                .downcast::<gtk::TextView>()
                .expect("agent prose is a text view");
            assert!(
                view.allocation().width() <= width,
                "agent prose is wider than the {width} column: {}",
                view.allocation().width()
            );
            let (_, prose_height) = view.preferred_height();
            assert!(
                prose_height > 40,
                "agent prose did not wrap in a {width} column (height {prose_height})"
            );

            let card = widget_by_class(messages.upcast_ref(), "eb-code-block").expect("code card");
            assert!(
                card.allocation().width() <= width,
                "code card is wider than the {width} column: {}",
                card.allocation().width()
            );
            let code_scroll = card
                .clone()
                .downcast::<gtk::Container>()
                .expect("code card")
                .children()
                .into_iter()
                .find_map(|child| child.downcast::<gtk::ScrolledWindow>().ok())
                .expect("code card scroller");
            for class_name in ["eb-thought", "eb-plan-text"] {
                let label = widget_by_class(messages.upcast_ref(), class_name)
                    .unwrap_or_else(|| panic!("missing {class_name}"))
                    .downcast::<gtk::Label>()
                    .unwrap_or_else(|_| panic!("{class_name} is not a label"));
                let lines = label.layout().map(|layout| layout.line_count()).unwrap_or(1);
                assert!(
                    lines > 1,
                    "{class_name} stayed on {lines} line(s) in a {width} column"
                );
            }

            let code_h = code_scroll.hadjustment();
            assert!(
                code_h.upper() > code_h.page_size() + 8.0,
                "long code should scroll inside its card at {width}: page {:.0} upper {:.0}",
                code_h.page_size(),
                code_h.upper()
            );

            window.close();
            drain_gtk();
        }
    }

    /// Scrolling up reveals a down arrow at the bottom of the transcript.
    /// Clicking it returns to the newest line and resumes following, and a
    /// growth of the tail while the reader is still up does not jump them or
    /// hide the arrow.
    #[test]
    fn jump_arrow_returns_to_the_latest_message() {
        if crate::widgets::try_init_gtk_for_test().is_none() {
            eprintln!("skipping: gtk needs the main thread and a display");
            return;
        }
        ensure_shell_css();
        let messages = gtk::Box::new(gtk::Orientation::Vertical, 0);
        messages.set_size_request(400, 2400);
        let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
        scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        scroll.set_propagate_natural_height(false);
        scroll.add(&messages);

        let viewport = gtk::Overlay::new();
        viewport.set_size_request(400, 240);
        viewport.add(&scroll);

        let pinned = Rc::new(Cell::new(true));
        let auto_scroll = Rc::new(Cell::new(false));
        let anchor: Rc<RefCell<Option<(gtk::Widget, f64)>>> = Rc::new(RefCell::new(None));
        let pending_load = Rc::new(Cell::new(false));
        let jump = attach_jump_latest(
            &viewport,
            &scroll,
            &pinned,
            &auto_scroll,
            &anchor,
            &pending_load,
        );

        let window = gtk::OffscreenWindow::new();
        window.add(&viewport);
        window.set_default_size(400, 240);
        window.show_all();
        drain_gtk();
        settle(&viewport, &scroll, 400, 240);

        let adjustment = scroll.vadjustment();
        adjustment.set_value(adjustment.upper() - adjustment.page_size());
        drain_gtk();
        let below = distance_from_end(&adjustment);
        assert!(
            below <= FOLLOW_SLACK,
            "the harness should start on the newest line, {below:.0}px short (upper {:.0}, page {:.0})",
            adjustment.upper(),
            adjustment.page_size()
        );
        assert!(pinned.get(), "the tail stays followed");
        assert!(!jump.is_visible(), "the arrow stays hidden on the newest message");

        let away = (adjustment.upper() - adjustment.page_size() - 400.0).max(0.0);
        adjustment.set_value(away);
        drain_gtk();
        assert!(!pinned.get(), "scrolling up leaves the tail");
        assert!(jump.is_visible(), "the down arrow shows once the reader leaves the end");
        settle(&viewport, &scroll, 400, 240);
        let place = jump.allocation();
        let view = viewport.allocation();
        assert!(
            (place.x() + place.width() / 2 - (view.x() + view.width() / 2)).abs() <= 4,
            "arrow should be centred, got x={} width={} in view x={} width={}",
            place.x(),
            place.width(),
            view.x(),
            view.width()
        );
        let bottom_gap = view.y() + view.height() - (place.y() + place.height());
        assert!(
            (16..=24).contains(&bottom_gap),
            "arrow should sit 20px above the composer edge, gap was {bottom_gap}"
        );
        assert_eq!(place.width(), 32, "arrow button is a 32px circle");
        assert_eq!(place.height(), 32, "arrow button is a 32px circle");

        let held = adjustment.value();
        messages.set_size_request(400, 3200);
        settle(&viewport, &scroll, 400, 240);
        assert!(
            (adjustment.value() - held).abs() <= 1.0,
            "a growing tail must not yank a reader who scrolled up ({held:.0} -> {:.0})",
            adjustment.value()
        );
        assert!(jump.is_visible(), "the arrow stays while the reader is still up");
        assert!(!pinned.get());

        let marker = gtk::Label::new(Some("anchor"));
        *anchor.borrow_mut() = Some((marker.upcast(), 12.0));
        pending_load.set(true);
        jump.clicked();
        drain_gtk();
        let below = distance_from_end(&adjustment);
        assert!(
            below <= FOLLOW_SLACK + 1.0,
            "click lands on the latest message, {below:.0}px short"
        );
        assert!(pinned.get(), "click resumes following");
        assert!(!jump.is_visible(), "the arrow hides once the latest message is in view");
        assert!(anchor.borrow().is_none(), "click drops the history anchor");
        assert!(!pending_load.get(), "click drops a pending history load");

        window.close();
        drain_gtk();
    }

    fn settle(viewport: &gtk::Overlay, scroll: &gtk::ScrolledWindow, width: i32, height: i32) {
        viewport.size_allocate(&gtk::Allocation::new(0, 0, width, height));
        scroll.size_allocate(&gtk::Allocation::new(0, 0, width, height));
        drain_gtk();
    }

    /// Screen CSS matches the app, so the 15px label and the bubble's 12px
    /// horizontal padding are part of the measured width.
    fn ensure_shell_css() {
        let css = format!(
            "{}\n{}\n{}",
            include_str!("../../../docs/ui-spec/gtk/tokens-light.css"),
            include_str!("../../../docs/ui-spec/gtk/tokens-metrics.css"),
            include_str!("../ui/gtk.css"),
        );
        let provider = gtk::CssProvider::new();
        provider
            .load_from_data(css.as_bytes())
            .expect("gtk.css must parse");
        if let Some(screen) = gtk::gdk::Screen::default() {
            gtk::StyleContext::add_provider_for_screen(
                &screen,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    }

    struct BubbleBox {
        width: i32,
        right_gap: i32,
    }

    /// `.eb-bubble` after the frame is given `column` pixels, which is what
    /// the message scroller does (752 closed, 500 with the artifact).
    /// `right_gap` is the space between the bubble's right edge and the line
    /// that holds it; right-aligned means 0.
    fn allocated_bubble(text: &str, column: i32) -> BubbleBox {
        let frame = user_turn(text, "12:00", &TurnActions::detached());
        let window = gtk::OffscreenWindow::new();
        window.add(&frame);
        window.set_default_size(column, 800);
        window.show_all();
        drain_gtk();
        // The window would otherwise grow to the bubble's natural width, so
        // the 500 column would never actually be 500.
        frame.size_allocate(&gtk::Allocation::new(0, 0, column, 800));
        let bubble = widget_by_class(frame.upcast_ref(), "eb-bubble").expect("user bubble");
        let line = bubble.parent().expect("bubble line");
        let allocation = bubble.allocation();
        let placed = BubbleBox {
            width: allocation.width(),
            right_gap: line.allocation().width() - (allocation.x() + allocation.width()),
        };
        window.close();
        drain_gtk();
        placed
    }

    fn drain_gtk() {
        for _ in 0..30 {
            if !gtk::events_pending() {
                break;
            }
            gtk::main_iteration_do(false);
        }
    }

    fn widget_by_class(root: &gtk::Widget, class_name: &str) -> Option<gtk::Widget> {
        if root.style_context().has_class(class_name) {
            return Some(root.clone());
        }
        root.clone().downcast::<gtk::Container>().ok().and_then(|container| {
            container
                .children()
                .into_iter()
                .find_map(|child| widget_by_class(&child, class_name))
        })
    }

    /// Every quick-action button painted under `root`, in pack order.
    fn mini_tooltips(root: &gtk::Widget) -> Vec<String> {
        let mut out = Vec::new();
        if root.style_context().has_class("eb-icon-mini") {
            if let Some(text) = root.tooltip_text() {
                out.push(text.to_string());
            }
        }
        if let Ok(container) = root.clone().downcast::<gtk::Container>() {
            for child in container.children() {
                out.extend(mini_tooltips(&child));
            }
        }
        out
    }

    fn actions_without_shell(prompt: Option<&str>) -> TurnActions {
        let noop: Rc<dyn Fn(String)> = Rc::new(|_| {});
        TurnActions {
            edit: noop.clone(),
            submit: noop,
            artifact: Rc::new(|_| {}),
            prompt: prompt.map(String::from),
        }
    }

    #[test]
    fn user_bubble_toolbar_copies_and_edits() {
        if crate::widgets::try_init_gtk_for_test().is_none() {
            eprintln!("skipping: gtk needs the main thread and a display");
            return;
        }
        let frame = user_turn("hello", "12:00", &actions_without_shell(None));
        assert_eq!(
            mini_tooltips(&frame.upcast()),
            ["Copy message", "Edit and resend"]
        );
    }

    #[test]
    fn hover_reveals_the_timestamp_and_leaving_hides_it() {
        if crate::widgets::try_init_gtk_for_test().is_none() {
            eprintln!("skipping: gtk needs the main thread and a display");
            return;
        }
        let parts = agent_turn(
            "EvoBuddy",
            None,
            &[Block::Text("hi".into())],
            false,
            &[],
            Some("12:00"),
            &TurnActions::detached(),
        );
        let stamp = widget_by_class(parts.frame.upcast_ref(), "eb-message-time")
            .and_then(|widget| widget.downcast::<gtk::Label>().ok())
            .expect("hover timestamp label");
        assert_eq!(stamp.opacity(), 0.0, "hidden at rest");

        // Feed the same crossing events GDK delivers when the pointer moves.
        let enter = gtk::gdk::Event::new(gtk::gdk::EventType::EnterNotify);
        let handled = parts
            .frame
            .emit_by_name::<bool>("enter-notify-event", &[&enter]);
        assert!(handled, "the EventBox handles the enter crossing");
        assert_eq!(stamp.opacity(), 1.0, "revealed on hover");

        let leave = gtk::gdk::Event::new(gtk::gdk::EventType::LeaveNotify);
        parts
            .frame
            .emit_by_name::<bool>("leave-notify-event", &[&leave]);
        assert_eq!(stamp.opacity(), 0.0, "hidden again after the pointer leaves");
    }

    #[test]
    fn settled_answer_gets_actions_streaming_gets_none() {
        if crate::widgets::try_init_gtk_for_test().is_none() {
            eprintln!("skipping: gtk needs the main thread and a display");
            return;
        }
        let actions = actions_without_shell(Some("hello"));
        let parts = agent_turn(
            "EvoBuddy",
            None,
            &[
                Block::Text("hi".into()),
                // Reasoning is part of the card but not of the copyable answer.
                Block::Thought {
                    text: "why".into(),
                    collapsed: true,
                },
            ],
            false,
            &[],
            Some("12:00"),
            &actions,
        );
        assert_eq!(
            mini_tooltips(&parts.frame.upcast()),
            ["Copy answer", "Good answer", "Bad answer", "Regenerate answer"]
        );

        let live = agent_turn(
            "EvoBuddy",
            None,
            &[Block::Text("hi".into())],
            true,
            &[],
            Some("12:00"),
            &actions,
        );
        assert!(
            mini_tooltips(&live.frame.upcast()).is_empty(),
            "no actions while the answer is still streaming"
        );

        // The hover timestamp is part of the settled toolbar; a turn without
        // a stamp (older saved sessions) renders no time label at all.
        let unstamped = agent_turn(
            "EvoBuddy",
            None,
            &[Block::Text("hi".into())],
            false,
            &[],
            None,
            &actions,
        );
        assert!(
            widget_by_class(unstamped.frame.upcast_ref(), "eb-message-time").is_none(),
            "an unstamped turn shows no timestamp"
        );

        // A stamped turn builds the label (hidden at rest by the hover CSS).
        let stamped = agent_turn(
            "EvoBuddy",
            None,
            &[Block::Text("hi".into())],
            false,
            &[],
            Some("12:00"),
            &actions,
        );
        assert!(
            widget_by_class(stamped.frame.upcast_ref(), "eb-message-time").is_some(),
            "a stamped turn carries its hover timestamp"
        );
    }

    fn two_files() -> Vec<Artifact> {
        vec![
            Artifact {
                name: "notes.md".into(),
                code: "x".repeat(2000),
            },
            Artifact {
                name: "quine.c".into(),
                code: "int main(){}".into(),
            },
        ]
    }

    #[test]
    fn settled_turn_with_files_gets_cards_and_the_view_all_link() {
        if crate::widgets::try_init_gtk_for_test().is_none() {
            eprintln!("skipping: gtk needs the main thread and a display");
            return;
        }
        let actions = actions_without_shell(None);
        let parts = agent_turn(
            "EvoBuddy",
            None,
            &[Block::Text("hi".into())],
            false,
            &two_files(),
            None,
            &actions,
        );
        let first =
            widget_by_class(parts.frame.upcast_ref(), "eb-artifact-card").expect("artifact card");
        assert_eq!(
            first.tooltip_text().map(|text| text.to_string()),
            Some("Open notes.md in the artifact panel".into())
        );
        assert!(
            widget_by_class(parts.frame.upcast_ref(), "eb-artifact-viewall").is_some(),
            "a third file hides behind the View all link"
        );

        // A single file: one card, no link.
        let parts = agent_turn(
            "EvoBuddy",
            None,
            &[Block::Text("hi".into())],
            false,
            &two_files()[..1],
            None,
            &actions,
        );
        assert!(widget_by_class(parts.frame.upcast_ref(), "eb-artifact-card").is_some());
        assert!(
            widget_by_class(parts.frame.upcast_ref(), "eb-artifact-viewall").is_none(),
            "nothing to view beyond the shown card"
        );

        // Streaming: no cards mid-answer, whatever the task has written.
        let live = agent_turn(
            "EvoBuddy",
            None,
            &[Block::Text("hi".into())],
            true,
            &two_files(),
            None,
            &actions,
        );
        assert!(
            widget_by_class(live.frame.upcast_ref(), "eb-artifact-card").is_none(),
            "cards wait for the answer to settle"
        );
    }

    #[test]
    fn card_sizes_use_the_captured_units() {
        assert_eq!(format_bytes(131), "131 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(33_382), "32.6 KB");
        assert_eq!(format_bytes(2 * 1024 * 1024), "2.0 MB");
    }

    #[test]
    fn code_badge_follows_the_extension() {
        assert!(is_code_file("quine.c"));
        assert!(is_code_file("app.PY"));
        assert!(is_code_file("notes.md"));
        assert!(!is_code_file("quine"));
        assert!(!is_code_file(".env"));
        assert!(!is_code_file("photo.png"));
    }

    #[test]
    fn copy_answer_collects_only_the_prose() {
        let blocks = [
            Block::Text("first".into()),
            Block::Thought {
                text: "hidden".into(),
                collapsed: true,
            },
            Block::Text("second".into()),
        ];
        assert_eq!(answer_text(&blocks), "first\nsecond");
    }

    /// A rejected stylesheet disables every rule, not just the bad one.
    #[test]
    fn shell_css_still_parses() {
        if crate::widgets::try_init_gtk_for_test().is_none() {
            eprintln!("skipping: gtk needs the main thread and a display");
            return;
        }
        let css = format!(
            "{}\n{}\n{}",
            include_str!("../../../docs/ui-spec/gtk/tokens-light.css"),
            include_str!("../../../docs/ui-spec/gtk/tokens-metrics.css"),
            include_str!("../ui/gtk.css"),
        );
        let provider = gtk::CssProvider::new();
        provider
            .load_from_data(css.as_bytes())
            .expect("gtk.css must parse, including the message-list fade");
    }

    fn text_view_in(widget: &gtk::Widget) -> gtk::TextView {
        if let Ok(view) = widget.clone().downcast::<gtk::TextView>() {
            return view;
        }
        widget
            .clone()
            .downcast::<gtk::Box>()
            .ok()
            .and_then(|box_| {
                box_.children()
                    .into_iter()
                    .find_map(|child| child.downcast::<gtk::TextView>().ok())
            })
            .expect("markdown block holds a text view")
    }

}
