//! Small widget helpers shared by every screen.
//!
//! Three differences between GTK widgets and CSS box model drive everything
//! here, and each helper absorbs one of them:
//!
//!   * there is no `gap`. `GtkBox::spacing` is uniform per box, while the
//!     product's gaps are uneven, so odd gaps become explicit spacers
//!     ([`spacer`]).
//!   * `min-height` and `padding` are *content-box*: writing both for a CSS
//!     "32px row with 5px vertical padding" yields 42px. Rows must be expressed
//!     as `padding: 0 12px; min-height: <row height>` instead (see
//!     `ui/gtk.css`).
//!   * `gtk::Widget` is not `IsA<Container>`, so there is no generic widget-tree
//!     walk. Builders register what a test needs to measure instead.

use std::rc::Rc;

use gtk::prelude::*;

/// Add a single CSS class.
pub fn class<W: WidgetExt>(widget: &W, name: &str) {
    widget.style_context().add_class(name);
}

/// A horizontal `GtkBox`.
pub fn hbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Horizontal, spacing)
}

/// A vertical `GtkBox`.
pub fn vbox(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}

/// A fixed-height gap — the closest equivalent to a one-off CSS `gap`.
pub fn spacer(height: i32) -> gtk::Box {
    let space = vbox(0);
    space.set_size_request(-1, height);
    space
}

/// A left-aligned label with a CSS class.
pub fn text(value: &str, class_name: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(value));
    label.set_xalign(0.0);
    class(&label, class_name);
    label
}

/// A label that truncates rather than forcing its container wider. Long task
/// titles would otherwise set the sidebar's minimum width.
pub fn ellipsized(value: &str, class_name: &str) -> gtk::Label {
    let label = text(value, class_name);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label
}

/// A standalone glyph used as an icon stand-in.
///
/// The product ships its own icon font and SVGs; those are its assets and are
/// not copied. Homebrew's `gtk+3` also ships only the `hicolor` theme, so
/// `GtkImage::from_icon_name` renders nothing on this machine — see ui-spec
/// §10. Text glyphs are the interim stand-in.
pub fn glyph(value: &str) -> gtk::Widget {
    crate::icons::request(value, "eb-glyph")
}

/// A flat, chrome-less button — every button in the product is one of these.
pub fn flat_button(class_name: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_relief(gtk::ReliefStyle::None);
    class(&button, class_name);
    button
}

/// A round-rect icon button with a single glyph inside.
///
/// `tooltip` is the accessible name as well: with no icon theme loaded, a glyph
/// is the only thing a user can see, so the tooltip carries the meaning.
pub fn icon_button(value: &str, class_name: &str, tooltip: &str) -> gtk::Button {
    let button = flat_button(class_name);
    button.set_valign(gtk::Align::Center);
    button.set_tooltip_text(Some(tooltip));
    button.add(&glyph(value));
    button
}

/// Show a widget's contents, then keep the widget itself hidden.
///
/// `gtk_widget_show_all` *returns immediately* for a widget marked
/// `no_show_all` — it does not descend into it either — so opting a widget out of
/// `show_all` before its own contents have been shown leaves those contents
/// invisible for good. That is silent: the button keeps its size and the popover
/// keeps its chrome, it just has nothing in it. Every control that starts hidden
/// and is revealed later has to be shown first and opted out second.
pub fn hide_from_show_all<W: IsA<gtk::Widget>>(widget: &W) {
    let widget: gtk::Widget = widget.clone().upcast();
    widget.show_all();
    widget.set_no_show_all(true);
    widget.set_visible(false);
}

/// Remove every child of a container.
///
/// REFRESH rebuilds regions rather than diffing them, so this runs on each
/// update. `children()` returns an owned `Vec`, which makes removing while
/// iterating safe.
pub fn clear<C: IsA<gtk::Container>>(container: &C) {
    for child in container.children() {
        container.remove(&child);
    }
}

/// `TextViewExt::buffer` is nullable in the binding but never null in practice.
pub fn buffer_of(view: &gtk::TextView) -> gtk::TextBuffer {
    view.buffer().expect("a GtkTextView always has a buffer")
}

/// A popover that names a feature and says it is not in this build.
///
/// For the controls whose product behaviour was never captured — search, share,
/// a new artifact tab, code assist. Showing a plausible-looking screen would be
/// worse than saying what is missing, and silently ignoring the click is worst
/// of all: it reads as a broken app. Invented chrome, recorded in ui-spec §10.
pub fn attach_notice(button: &gtk::Button, message: &str) -> gtk::Popover {
    let popover = gtk::Popover::new(Some(button));
    popover.set_position(gtk::PositionType::Bottom);
    class(&popover, "eb-menu");
    class(&popover, "eb-missing");

    let label = gtk::Label::new(Some(message));
    label.set_line_wrap(true);
    label.set_max_width_chars(36);
    label.set_xalign(0.0);
    class(&label, "eb-missing-label");
    popover.add(&label);

    // Show the label now — a popover is sized from the requisition of what it
    // holds, so a label that never got shown would size the popup to its own
    // empty chrome.
    hide_from_show_all(&popover);

    button.connect_clicked({
        let popover = popover.clone();
        move |_| popover.popup()
    });
    popover
}

/// Swap a button's glyph, and its tooltip, for a moment.
///
/// Copying is invisible — the clipboard has no view — so a button that only
/// copies looks like a button that does nothing. GTK has no toast, and the glyph
/// is the feedback.
pub fn flash(button: &gtk::Button, value: &str, tooltip: &str) {
    let button = button.clone();
    let Some(child) = button.child() else {
        return;
    };
    let previous_tooltip = button.tooltip_text();
    button.set_tooltip_text(Some(tooltip));

    // Icons are GtkImage now; text glyphs are GtkLabel. Handle both by swapping
    // the child for a temporary label, then restoring the original icon.
    if let Ok(label) = child.clone().downcast::<gtk::Label>() {
        let glyph = label.text().to_string();
        label.set_text(value);
        gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
            label.set_text(&glyph);
            button.set_tooltip_text(previous_tooltip.as_deref());
        });
        return;
    }

    button.remove(&child);
    let label = gtk::Label::new(Some(value));
    class(&label, "eb-glyph");
    button.add(&label);
    label.show();
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
        button.remove(&label);
        button.add(&child);
        child.show();
        button.set_tooltip_text(previous_tooltip.as_deref());
    });
}

/// Attach a single-select popover to `button`.
///
/// The product's dropdowns are portalled `wb-popover` nodes positioned against
/// their trigger; `GtkPopover` is the same idea and already handles the grab,
/// click-outside dismissal and Escape. Rows are plain buttons rather than a
/// `GtkListBox` so the styling does not depend on which theme is installed.
///
/// `selection` is only read, to mark the current row; it is repainted every time
/// the menu opens, because a closure built here never runs again. `on_pick`
/// receives the index of the chosen row, and the caller is what actually changes
/// state.
pub fn attach_menu(
    button: &gtk::Button,
    items: &[&'static str],
    selection: Rc<std::cell::Cell<usize>>,
    position: gtk::PositionType,
    on_pick: Rc<dyn Fn(usize)>,
) -> gtk::Popover {
    let popover = gtk::Popover::new(Some(button));
    popover.set_position(position);
    class(&popover, "eb-menu");

    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let mut rows: Vec<gtk::Button> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let row = flat_button("eb-menu-item");
        row.set_tooltip_text(Some(item));
        row.add(&text(item, "eb-menu-item-label"));
        row.connect_clicked({
            let popover = popover.clone();
            let on_pick = on_pick.clone();
            move |_| {
                popover.popdown();
                on_pick(index);
            }
        });
        column.pack_start(&row, false, false, 0);
        rows.push(row);
    }
    popover.add(&column);

    popover.connect_show(move |_| {
        let selected = selection.get();
        for (index, row) in rows.iter().enumerate() {
            let context = row.style_context();
            if index == selected {
                context.add_class("eb-menu-item-active");
            } else {
                context.remove_class("eb-menu-item-active");
            }
        }
    });

    // Show the rows now but keep the popover itself hidden, so the first
    // `popup()` measures a built tree — and the window's `show_all()` must not
    // reveal menus that were never opened.
    hide_from_show_all(&popover);

    button.connect_clicked({
        let popover = popover.clone();
        move |_| popover.popup()
    });

    popover
}
