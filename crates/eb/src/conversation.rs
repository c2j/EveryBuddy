//! The conversation pane — the product's `conversation-page-chrome`.
//!
//! Measured against the live capture (ui-spec §4.4), 580 wide:
//!
//! ```text
//! y=0    topbar        580x64, padding 0 16px, 8px gap
//!        options       540x32, 12px gap
//!         title        16px/600
//!         actions      68x32 = two 32px buttons, 4px gap
//! y=64   message list  580x636, `padding: 0 40px` -> a 500 wide column
//! y=700  input area    580x168 = a 134px card inset 24px + a 34px AI notice
//! ```
//!
//! Every message sits in a `cr-frame` with 16px of vertical padding. The user's
//! turn is a right-aligned 16/16/0/16 bubble on #f2f2f2; the agent's turn is a
//! left-aligned card with a logo/name header. Tool calls are single muted rows,
//! and each block carries 14px of vertical margin, which is what produces the
//! captured 53px rhythm for a one-line block.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;

use crate::composer::Composer;
use crate::model::{Block, DiffLine, Task, Turn};
use crate::spec;
use crate::widgets::*;

#[derive(Clone)]
pub struct Wiring {
    /// The "show sidebar" affordance shown while the sidebar is collapsed.
    pub reveal: Rc<dyn Fn()>,
}

pub struct Conversation {
    pub root: gtk::Box,
    pub topbar: gtk::Box,
    /// The action group in the topbar: search and share.
    pub actions: gtk::Box,
    /// The topbar title, repainted with the active task's title.
    pub title: gtk::Label,
    /// The scroll region holding the messages.
    pub scroll: gtk::ScrolledWindow,
    /// Repopulated on every refresh.
    pub messages: gtk::Box,
    pub reveal: gtk::Button,
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

    // Measured: exactly two 32px ghost buttons, 4px apart, giving the captured
    // 68x32 group. No third control is invented for reopening the artifact
    // panel — clicking the task's sidebar row does that, because that is what
    // re-opens it in the product too.
    //
    // Neither action was captured working, so each says what it would do rather
    // than doing nothing (ui-spec §10).
    let actions = hbox(4);
    actions.set_valign(gtk::Align::Center);
    let search = icon_button("◉", "eb-icon", "Search in task");
    attach_notice(
        &search,
        "Searching inside a task is not part of this UI-only build.",
    );
    actions.pack_start(&search, false, false, 0);

    let share = icon_button("⤴", "eb-icon", "Share task");
    attach_notice(
        &share,
        "Sharing a task is not part of this UI-only build.",
    );
    actions.pack_start(&share, false, false, 0);

    options.pack_start(&actions, false, false, 0);
    topbar.pack_start(&options, true, true, 0);
    root.pack_start(&topbar, false, false, 0);

    // ---- body -----------------------------------------------------------
    let body = vbox(0);
    body.set_vexpand(true);

    let messages = vbox(0);
    class(&messages, "eb-messages");
    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    // Long model output must scroll inside the message column, never widen the
    // conversation pane (and with it the whole window).
    scroll.set_propagate_natural_width(false);
    scroll.add(&messages);
    body.pack_start(&scroll, true, true, 0);

    // Measured 580x168: the card inset 24px plus the AI notice. Built by the
    // composer module because it shares the editor and toolbar with the landing
    // page; packed here because it is part of this pane's column.
    body.pack_start(&composer.root, false, false, 0);

    root.pack_start(&body, true, true, 0);

    Conversation {
        root,
        topbar,
        actions,
        title,
        scroll,
        messages,
        reveal,
    }
}

/// Rebuild the message list for one task.
///
/// Rebuilding rather than appending keeps a single source of truth: the widgets
/// always describe the current state of `App`, never an accumulation of deltas.
pub fn fill_messages(conversation: &Conversation, task: &Task) {
    clear(&conversation.messages);
    for turn in &task.turns {
        match turn {
            Turn::User { text, time } => {
                conversation.messages.pack_start(&user_turn(text, time), false, false, 0);
            }
            Turn::Agent {
                name,
                status,
                blocks,
                streaming,
            } => {
                conversation
                    .messages
                    .pack_start(&agent_turn(name, status.as_deref(), blocks, *streaming), false, false, 0);
            }
        }
    }
    conversation.messages.show_all();
    scroll_to_bottom(&conversation.scroll);
}

/// Keep the newest message in view, the way a chat log is expected to behave.
fn scroll_to_bottom(scroll: &gtk::ScrolledWindow) {
    // The screenshot harness can keep the top of a long conversation visible so
    // the first block is not clipped by the scroll-to-bottom behaviour.
    if std::env::var_os("EB_NO_SCROLL").is_some() {
        return;
    }
    let adjustment = scroll.vadjustment();
    // The layout is only final after the new children have been allocated, so
    // jump on the next idle turn rather than here.
    let adjustment = adjustment.clone();
    gtk::glib::idle_add_local_once(move || {
        adjustment.set_value(adjustment.upper() - adjustment.page_size());
    });
}

/// Measured: a 500 wide frame with 16px of vertical padding, the bubble pushed
/// to the right edge, then a right-aligned 28px toolbar underneath.
fn user_turn(value: &str, time: &str) -> gtk::Box {
    let frame = vbox(0);
    class(&frame, "eb-frame");

    let line = hbox(0);
    line.set_halign(gtk::Align::Fill);

    let bubble = vbox(0);
    class(&bubble, "eb-bubble");
    let label = gtk::Label::new(Some(value));
    label.set_line_wrap(true);
    label.set_xalign(1.0);
    label.set_justify(gtk::Justification::Right);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_width_chars(1);
    // The product's bubble is a little over half the message column
    // (255 of 500 for a two-line message), so the wrap is pinned to a
    // comparable character count. GTK has no `max-width` percentage.
    label.set_max_width_chars(48);
    class(&label, "eb-bubble-text");
    bubble.pack_start(&label, false, false, 0);
    line.pack_end(&bubble, false, false, 0);
    frame.pack_start(&line, false, false, 0);

    let toolbar = hbox(8);
    class(&toolbar, "eb-message-toolbar");
    toolbar.set_halign(gtk::Align::End);
    toolbar.pack_start(&text(time, "eb-message-time"), false, false, 0);

    let copy = icon_button("⧉", "eb-icon-mini", "Copy message");
    let value = value.to_string();
    copy.connect_clicked(move |button| {
        let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        clipboard.set_text(&value);
        clipboard.store();
        flash(button, "✓", "Copied");
    });
    toolbar.pack_start(&copy, false, false, 0);
    frame.pack_start(&toolbar, false, false, 0);

    frame
}

/// Measured: a 500 wide frame; a 24.5px header with a 10px gap between the
/// agent mark and its name (14px/600); then the blocks.
fn agent_turn(name: &str, status: Option<&str>, blocks: &[Block], streaming: bool) -> gtk::Box {
    let frame = vbox(0);
    class(&frame, "eb-frame");

    let header = hbox(10);
    header.set_valign(gtk::Align::Center);

    // EvoBuddy's own agent mark (embedded PNG), not the product mascot.
    let logo = crate::icons::named("agent", 24, "eb-agent-logo");
    header.pack_start(&logo, false, false, 0);
    header.pack_start(&text(name, "eb-agent-name"), false, false, 0);
    frame.pack_start(&header, false, false, 0);

    if let Some(status) = status {
        // Measured: 15px at rgba(0,0,0,.5), directly under the header. Errors
        // arrive through this line too, so it must be able to wrap instead of
        // becoming the window's minimum width.
        let line = text(status, "eb-agent-status");
        line.set_margin_top(8);
        line.set_line_wrap(true);
        line.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        line.set_width_chars(1);
        line.set_xalign(0.0);
        frame.pack_start(&line, false, false, 0);
    }

    let body = vbox(0);
    body.set_margin_top(12);
    for block in blocks {
        body.pack_start(&block_widget(block), false, false, 0);
    }
    frame.pack_start(&body, false, false, 0);

    if streaming {
        frame.pack_start(&streaming_footer(), false, false, 0);
    }

    frame
}

/// Measured: a one-line block occupies 53px — a 24px line plus 14px of margin
/// on each side. Applying that margin per block reproduces the captured rhythm
/// without any per-block spacing at the container level.
fn block_widget(block: &Block) -> gtk::Widget {
    let widget: gtk::Widget = match block {
        Block::Text(value) => {
            let label = gtk::Label::new(Some(value));
            label.set_line_wrap(true);
            label.set_xalign(0.0);
            label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            // A long unbroken token (JSON errors, URLs) otherwise becomes the
            // label's minimum width and drags the whole window wider.
            label.set_width_chars(1);
            class(&label, "eb-text-block");
            label.upcast()
        }
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
        Block::Table { headers, rows } => table(headers, rows).upcast(),
    };
    widget.set_margin_top(14);
    widget.set_margin_bottom(14);
    widget
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
    output_view.set_wrap_mode(gtk::WrapMode::None);
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
fn group_block(title: &str, blocks: &[Block], collapsed: bool) -> gtk::Box {
    let root = vbox(0);
    class(&root, "eb-group");

    let header = flat_button("eb-group-header");
    header.set_tooltip_text(Some("Toggle tool details"));
    let row = hbox(8);
    row.set_valign(gtk::Align::Center);
    let chevron = text(if collapsed { "›" } else { "⌄" }, "eb-group-chevron");
    row.pack_start(&text(title, "eb-group-title"), false, false, 0);
    row.pack_start(&chevron, false, false, 0);
    header.add(&row);

    let content = vbox(0);
    class(&content, "eb-group-content");
    for block in blocks {
        content.pack_start(&block_widget(block), false, false, 0);
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
    label.set_xalign(0.0);
    label.set_line_wrap(true);
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
