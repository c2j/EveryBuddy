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
    /// Kept so the tab's own close affordance can reach it.
    close: Rc<dyn Fn()>,
    /// The artifact currently built into `tabs`/`body`. Refreshes happen for
    /// every streamed event; rebuilding the same editor each time churns the
    /// widget tree and can leave a freshly created text view unpainted in an
    /// offscreen capture.
    shown: RefCell<Option<(String, String)>>,
}

pub fn build(wiring: &Wiring, contribs: &Contribs) -> Detail {
    let root = vbox(0);
    class(&root, "eb-detail");

    // ---- tab strip ------------------------------------------------------
    let header = hbox(4);
    class(&header, "eb-detail-header");

    let tabs = hbox(4);
    tabs.set_valign(gtk::Align::Center);
    header.pack_start(&tabs, false, false, 0);

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
        tabs,
        body,
        close: wiring.close.clone(),
        shown: RefCell::new(None),
    }
}

/// Rebuild both the tab strip and the editor frame for one artifact.
///
/// `show_all` at the end is required, not cosmetic: the children are created
/// after the window's first `show_all`, and a newly created widget is not
/// visible until it is shown.
pub fn fill(detail: &Detail, artifact: &Artifact) {
    let key = (artifact.name.clone(), artifact.code.clone());
    if detail.shown.borrow().as_ref() == Some(&key) {
        return;
    }

    clear(&detail.tabs);
    clear(&detail.body);

    detail.tabs.pack_start(&tab(&artifact.name, &detail.close), false, false, 0);

    detail.body
        .pack_start(&toolbar(&artifact.name, &artifact.code), false, false, 0);
    detail.body.pack_start(&code(&artifact.code), true, true, 0);

    detail.tabs.show_all();
    detail.body.show_all();
    *detail.shown.borrow_mut() = Some(key);
}

/// Measured: 240x32, radius 8, `padding: 0 8px`, 4px gap, 14px/600 when active.
///
/// The tab is a plain row rather than a button: the only tab is the active one,
/// so clicking its body has nothing to do, and the measured close affordance
/// inside it is its own control. Keeping the body inert is what makes the close
/// button clickable at all.
fn tab(name: &str, close: &Rc<dyn Fn()>) -> gtk::Box {
    let button = hbox(4);
    class(&button, "eb-tab");
    class(&button, "eb-tab-active");
    button.set_tooltip_text(Some(name));

    button.pack_start(&glyph("▤"), false, false, 0);

    let label = gtk::Label::new(Some(name));
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_max_width_chars(24);
    class(&label, "eb-tab-name");
    button.pack_start(&label, false, false, 0);

    let dismiss = flat_button("eb-tab-close");
    dismiss.set_tooltip_text(Some("Close artifact"));
    dismiss.add(&glyph("×"));
    dismiss.connect_clicked({
        let close = close.clone();
        move |_| close()
    });
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
