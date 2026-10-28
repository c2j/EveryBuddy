//! The artifact panel — the product's `detail-panel-container`, 579 wide.
//!
//! Measured against the live capture (ui-spec §4.4):
//!
//! ```text
//! y=0    header   579x64, padding 0 16px, 4px gap, white
//!          tabs   475x32: the active tab is 240x32 on #f2f2f2, radius 8,
//!                 `padding: 0 8px`, 4px gap; then a 32x32 add button
//!          acts   68x32 = two 32px buttons
//! y=64   toolbar  579x72: file name and its actions
//! y=136  code     579x708: a 68px line-number gutter at x=861 and the code
//!                 column at x=929, 20px lines of 14px monospace
//! ```
//!
//! The real client embeds a Monaco editor. A `GtkTextView` cannot render syntax
//! colours, so the code is shown as plain monospace — the frame is faithful, the
//! highlighting is not (ui-spec §10).

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;

use crate::model::Artifact;
use crate::widgets::*;
use eb_api::contrib::Contribs;

#[derive(Clone)]
pub struct Wiring {
    /// The panel's own collapse button.
    pub close: Rc<dyn Fn()>,
    /// A tab's `×`: dismiss that one file, not the panel.
    pub tab_close: Rc<dyn Fn(usize)>,
    /// The "expand" button in the header.
    pub expand: Rc<dyn Fn()>,
    /// Open one dsh component-host plugin in the side panel.
    pub plugin: Rc<dyn Fn(&'static str)>,
}

pub struct Detail {
    pub root: gtk::Box,
    pub header: gtk::Box,
    /// Repopulated on every refresh.
    pub tabs: gtk::Box,
    /// Repopulated on every refresh.
    pub body: gtk::Box,
    /// The files behind the tab strip and which one is shown. Tab clicks
    /// mutate `selected` and rebuild through the same `rebuild` the refresh
    /// uses, so the panel needs no round-trip through the store.
    view: Rc<RefCell<ArtifactView>>,
}

struct ArtifactView {
    tabs: gtk::Box,
    body: gtk::Box,
    files: Vec<Artifact>,
    selected: usize,
    tab_close: Rc<dyn Fn(usize)>,
    /// The tab strip already built for exactly these (name, content) pairs; a
    /// refresh that changes nothing must not churn the widget tree (see the
    /// old `shown` note: a freshly created text view can paint blank
    /// offscreen).
    built: Option<Vec<(String, String)>>,
}

fn rebuild(view: &Rc<RefCell<ArtifactView>>) {
    let (tabs, body) = {
        let view = view.borrow();
        (view.tabs.clone(), view.body.clone())
    };
    clear(&tabs);
    clear(&body);
    let (selected, names) = {
        let view = view.borrow();
        (view.selected, view.files.iter().map(|f| f.name.clone()).collect::<Vec<_>>())
    };
    let tab_close = view.borrow().tab_close.clone();
    let tab_close = view.borrow().tab_close.clone();
    for (index, name) in names.iter().enumerate() {
        tabs.pack_start(&tab(name, index == selected, tab_close.clone(), view, index), false, false, 0);
    }
    {
        let view = view.borrow();
        if let Some(file) = view.files.get(selected) {
            body.pack_start(&toolbar(&file.name, &file.code), false, false, 0);
            body.pack_start(&code(&file.code), true, true, 0);
        }
    }
    tabs.show_all();
    body.show_all();
    // Children created after the window's first `show_all` are *visible* but
    // never *allocated*: `show_all` alone does not queue a resize, and a
    // `GtkViewport` swallows resize propagations from its child (scrolling,
    // not relayout, is its answer to a growing child). Queue the relayout on
    // the window itself; the live frame clock runs it before the next paint,
    // and `selftest::snapshot` re-allocates explicitly for offscreen captures.
    if let Some(window) = tabs
        .ancestor(gtk::Window::static_type())
        .and_then(|widget| widget.downcast::<gtk::Window>().ok())
    {
        window.queue_resize();
    }
}

pub fn build(wiring: &Wiring, contribs: &Contribs) -> Detail {
    let root = vbox(0);
    class(&root, "eb-detail");

    // ---- tab strip ------------------------------------------------------
    let header = hbox(4);
    class(&header, "eb-detail-header");

    let tabs = hbox(4);
    tabs.set_valign(gtk::Align::Center);
    // Like the code view below, the strip must not forward its natural width:
    // two 240px tabs would push the header — and with it the whole panel — past
    // the 580px share, and GtkPaned's resize-child semantics would snap the
    // sash to that request. Overflow scrolls sideways instead.
    let tabs_scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    tabs_scroll.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
    tabs_scroll.set_propagate_natural_width(false);
    tabs_scroll.set_min_content_width(120);
    tabs_scroll.set_valign(gtk::Align::Center);
    tabs_scroll.add(&tabs);
    // The header's expanding child: without this the scroller sits at its
    // 120px minimum and the strip clips the tabs to nothing.
    header.pack_start(&tabs_scroll, true, true, 0);

    let add = icon_button("+", "eb-detail-add", "New tab");
    attach_notice(&add, contribs.notice("detail.new_tab"));
    header.pack_start(&add, false, false, 0);

    let actions = hbox(4);
    actions.set_valign(gtk::Align::Center);
    header.pack_end(&actions, false, false, 0);

    let theme_plugin = icon_button("◐", "eb-icon", "Open theme UI plugin");
    theme_plugin.connect_clicked({
        let plugin = wiring.plugin.clone();
        move |_| plugin("ui-theme")
    });
    actions.pack_start(&theme_plugin, false, false, 0);

    let feedback_plugin = icon_button("✦", "eb-icon", "Open feedback UI plugin");
    feedback_plugin.connect_clicked({
        let plugin = wiring.plugin.clone();
        move |_| plugin("ui-message-feedback")
    });
    actions.pack_start(&feedback_plugin, false, false, 0);

    let expand = icon_button("⤢", "eb-icon", "Give the panel the whole window");
    expand.connect_clicked({
        let expand = wiring.expand.clone();
        move |_| expand()
    });
    actions.pack_start(&expand, false, false, 0);

    let close = icon_button("▥", "eb-icon", "Hide artifact panel");
    close.connect_clicked({
        let close = wiring.close.clone();
        move |_| close()
    });
    actions.pack_start(&close, false, false, 0);

    root.pack_start(&header, false, false, 0);

    let body = vbox(0);
    body.set_vexpand(true);
    root.pack_start(&body, true, true, 0);

    Detail {
        root,
        header,
        tabs: tabs.clone(),
        body: body.clone(),
        view: Rc::new(RefCell::new(ArtifactView {
            tabs,
            body,
            files: Vec::new(),
            selected: 0,
            tab_close: wiring.tab_close.clone(),
            built: None,
        })),
    }
}

/// Rebuild the tab strip and the editor frame for the task's written files.
///
/// `show_all` at the end is required, not cosmetic: the children are created
/// after the window's first `show_all`, and a newly created widget is not
/// visible until it is shown.
pub fn fill(detail: &Detail, files: &[Artifact]) {
    if files.is_empty() {
        return;
    }
    // Name *and* content: a rewrite of the newest file keeps the single-entry
    // name list identical, and the editor must still refresh.
    let key: Vec<(String, String)> = files
        .iter()
        .map(|file| (file.name.clone(), file.code.clone()))
        .collect();
    {
        let mut view = detail.view.borrow_mut();
        if view.built.as_ref() == Some(&key) {
            return;
        }
        let newest_changed = view
            .files
            .last()
            .map(|file| file.name.as_str())
            .unwrap_or("")
            != files
                .last()
                .map(|file| file.name.as_str())
                .unwrap_or("");
        view.files = files.to_vec();
        if newest_changed {
            // A fresh file takes the stage; a tab closed away falls back to
            // the last remaining one.
            view.selected = files.len() - 1;
        } else {
            view.selected = view.selected.min(files.len() - 1);
        }
    }
    rebuild(&detail.view);
    detail.view.borrow_mut().built = Some(key);
}

/// Select one of the panel's tabs — used when a conversation artifact card is
/// clicked. A no-op while the panel holds no files.
pub fn select(detail: &Detail, index: usize) {
    {
        let mut view = detail.view.borrow_mut();
        if index >= view.files.len() || view.selected == index {
            return;
        }
        view.selected = index;
    }
    rebuild(&detail.view);
}

/// Measured: 240x32, radius 8, `padding: 0 8px`, 4px gap, 14px/600 when active.
///
/// The tab body is a box rather than a button: the measured close affordance
/// inside it is its own control, and a button parent would swallow its clicks.
/// The box listens for presses itself to select the tab; the close button
/// consumes its own press first, so closing never also selects.
fn tab(
    name: &str,
    active: bool,
    tab_close: Rc<dyn Fn(usize)>,
    view: &Rc<RefCell<ArtifactView>>,
    index: usize,
) -> gtk::Box {
    let button = hbox(4);
    class(&button, "eb-tab");
    if active {
        class(&button, "eb-tab-active");
    }
    // Share whatever the strip has left, so a couple of short names fill the
    // header like the capture instead of hugging the left edge.
    button.set_hexpand(true);
    button.set_tooltip_text(Some(name));
    button.add_events(gtk::gdk::EventMask::BUTTON_PRESS_MASK);
    let view = {
        let view = view.clone();
        Rc::downgrade(&view)
    };
    button.connect_button_press_event(move |_widget, event| {
        if event.button() == 1 {
            // Rebuild on idle, not inside the handler: the press would
            // otherwise destroy this very box before its button-release
            // arrives, and GTK asserts the widget is realized for its event.
            let view = view.clone();
            gtk::glib::idle_add_local_once(move || {
                let Some(view) = view.upgrade() else { return };
                view.borrow_mut().selected = index;
                rebuild(&view);
            });
            return gtk::glib::Propagation::Stop;
        }
        gtk::glib::Propagation::Proceed
    });

    button.pack_start(&glyph("▤"), false, false, 0);

    let label = gtk::Label::new(Some(name));
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    // The capture's 240px tab is the *ceiling*, not the size: names are
    // typically short, and fixed-width tabs would push all but one off a
    // 580px strip. `max_width_chars` caps long names at roughly the measured
    // width and the label takes whatever is left of the tab.
    label.set_max_width_chars(24);
    label.set_hexpand(true);
    class(&label, "eb-tab-name");
    button.pack_start(&label, true, true, 0);

    let dismiss = flat_button("eb-tab-close");
    dismiss.set_tooltip_text(Some("Close artifact"));
    dismiss.add(&glyph("×"));
    dismiss.connect_clicked(move |_| tab_close(index));
    button.pack_start(&dismiss, false, false, 0);

    button
}

/// Measured: 579x72. The real client shows the file name, a copy action and a
/// small tool cluster; only the name and copy are reproduced, since the rest of
/// the cluster's meaning was not captured.
fn toolbar(name: &str, code: &str) -> gtk::Box {
    let bar = hbox(8);
    class(&bar, "eb-detail-toolbar");

    let label = gtk::Label::new(Some(name));
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    class(&label, "eb-detail-file");
    bar.pack_start(&label, false, false, 0);

    let copy = icon_button("⧉", "eb-icon-mini", "Copy file contents");
    let code = code.to_string();
    copy.connect_clicked(move |button| {
        let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        clipboard.set_text(&code);
        clipboard.store();
        flash(button, "✓", "Copied");
    });
    bar.pack_start(&copy, false, false, 0);

    bar
}

/// Measured: 579x708, a 68px gutter and a 14px monospace column with 20px lines.
///
/// The gutter is a second text view rather than a drawn widget: two text views
/// with the same font produce the same line height, so they stay in step without
/// any coupling, and the code view keeps its own selection behaviour.
fn code(source: &str) -> gtk::ScrolledWindow {
    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    // A non-wrapping text view requests a minimum width equal to its longest
    // line, and a `GtkScrolledWindow` forwards that request by default. The
    // artifact panel then refuses to shrink to its measured 579px — the sash
    // clamps instead. Turning propagation off keeps the pane resizable and lets
    // the code scroll sideways, which is what the capture shows.
    scroll.set_propagate_natural_width(false);
    scroll.set_min_content_width(120);
    class(&scroll, "eb-code-scroll");

    let row = hbox(0);
    row.set_valign(gtk::Align::Start);

    let numbers: String = (1..=source.lines().count().max(1))
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    row.pack_start(
        &read_only(&numbers, "eb-code-gutter", gtk::Justification::Right),
        false,
        false,
        0,
    );

    let source_view = read_only(source, "eb-code", gtk::Justification::Left);
    // A non-wrapping `GtkTextView` reports the width of the lines it has laid
    // out, and it can never lay out more than the width it was allocated — so
    // its request settles at whatever the allocation was and a long line is
    // clipped with nothing to scroll to. Measuring the widest line with Pango
    // and requesting that width is what gives the scroller a wider child.
    if let Some(width) = widest_line_width(&source_view, source) {
        source_view.set_size_request(width, -1);
    }
    row.pack_start(&source_view, true, true, 0);

    scroll.add(&row);
    scroll
}

/// The pixel width of the widest line, in the code view's own font.
fn widest_line_width(view: &gtk::TextView, source: &str) -> Option<i32> {
    let widest = source.lines().max_by_key(|line| line.chars().count())?;
    if widest.is_empty() {
        return None;
    }
    // A little slack: Pango measures the glyphs, the view also has a caret and
    // a right margin to fit.
    Some(view.create_pango_layout(Some(widest)).pixel_size().0 + 24)
}

/// A text view with editing and the caret turned off — a viewer, not an input.
fn read_only(value: &str, class_name: &str, justify: gtk::Justification) -> gtk::TextView {
    let view = gtk::TextView::new();
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_monospace(true);
    view.set_wrap_mode(gtk::WrapMode::None);
    view.set_justification(justify);
    view.set_valign(gtk::Align::Start);
    view.set_top_margin(0);
    view.set_bottom_margin(0);
    view.set_left_margin(0);
    view.set_right_margin(0);
    class(&view, class_name);
    buffer_of(&view).set_text(value);
    view
}
