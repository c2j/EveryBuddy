//! The 280px sidebar: `conversation-sidebar` / `conversation-list`.
//!
//! Identical in both captured routes — the only region that changes is the
//! scrollable content, which either holds the top-aligned empty state (no tasks)
//! or two labelled sections. Welcome-state geometry is in ui-spec §4.3; the task
//! list numbers are in §4.4.
//!
//! The module builds the static chrome and hands back the handles the shell
//! needs: the content box to refill, and the nav buttons whose active class it
//! repaints.

use gtk::prelude::*;

use eb_api::contrib::Contribs;
use crate::spec;
use crate::widgets::*;

/// Callbacks the sidebar raises.
#[derive(Clone)]
pub struct Wiring {
    /// A nav row was chosen.
    pub nav: std::rc::Rc<dyn Fn(usize)>,
    /// The topbar's collapse button.
    pub collapse: std::rc::Rc<dyn Fn()>,
    /// A row of the task-filter menu was chosen.
    pub filter: std::rc::Rc<dyn Fn(usize)>,
    /// A row of the account menu was chosen (0 = Settings).
    pub account: std::rc::Rc<dyn Fn(usize)>,
}

pub struct Sidebar {
    pub root: gtk::Box,
    pub topbar: gtk::Box,
    pub header: gtk::Box,
    /// The scroll region the shell repopulates on every refresh.
    pub content: gtk::Box,
    pub footer: gtk::Box,
    pub nav: Vec<gtk::Button>,
}

/// One row of the task list, flattened from application state.
pub struct TaskRow {
    pub title: String,
    pub meta: String,
    pub active: bool,
    pub index: usize,
    pub workspace: Option<String>,
}

pub fn build(wiring: &Wiring, contribs: &Contribs) -> Sidebar {
    let root = vbox(0);
    class(&root, "eb-sidebar");
    root.set_size_request(spec::SIDEBAR_WIDTH, -1);

    let collapse = icon_button("◧", "eb-icon", "Hide sidebar");
    collapse.connect_clicked({
        let collapse = wiring.collapse.clone();
        move |_| collapse()
    });
    let filter = filter_button(wiring, contribs);

    let topbar = topbar(&collapse, &filter, contribs);
    root.pack_start(&topbar, false, false, 0);

    let (header, nav) = header(wiring, contribs);
    root.pack_start(&header, false, false, 0);

    let content = vbox(0);
    class(&content, "eb-sidebar-content");
    content.set_vexpand(true);
    let scroller = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.add(&content);
    root.pack_start(&scroller, true, true, 0);

    let footer = footer(contribs, &wiring.account);
    root.pack_start(&footer, false, false, 0);

    Sidebar {
        root,
        topbar,
        header,
        content,
        footer,
        nav,
    }
}

/// 279x64, padded `0 12px 0 86px`: the 86px left inset keeps the three buttons
/// clear of the macOS traffic lights, which float over the sidebar.
///
/// Left to right the capture shows collapse, search, filter. `pack_end` places
/// each new child *leftwards*, so they are added in reverse.
fn topbar(collapse: &gtk::Button, filter: &gtk::Button, contribs: &Contribs) -> gtk::Box {
    let bar = hbox(4);
    class(&bar, "eb-sidebar-topbar");

    // Search is its own 800x640 window in the product (`main/window-apps.js`)
    // and its contents were never captured, so the button says so instead of
    // looking broken. The copy is the notices plugin's.
    let search = icon_button("◉", "eb-icon", "Search");
    attach_notice(&search, contribs.notice("sidebar.search"));

    bar.pack_end(filter, false, false, 0);
    bar.pack_end(&search, false, false, 0);
    bar.pack_end(collapse, false, false, 0);

    bar
}

/// Kept as a function because the shell holds this button: the task-filter menu
/// is the one sidebar dropdown whose selection is not part of `App`.
fn filter_button(wiring: &Wiring, contribs: &Contribs) -> gtk::Button {
    let button = icon_button("≡", "eb-icon", "Filter tasks");
    attach_menu(
        &button,
        contribs.menu("filter"),
        std::rc::Rc::new(std::cell::Cell::new(0)),
        gtk::PositionType::Bottom,
        wiring.filter.clone(),
    );
    button
}

/// 279x246: the brand block (255x48) then the nav list (255x168, starting at
/// y=122 once the 32px brand content, its 20px padding and the 2px gap land).
fn header(wiring: &Wiring, contribs: &Contribs) -> (gtk::Box, Vec<gtk::Button>) {
    let header = vbox(2);
    class(&header, "eb-sidebar-header");

    let brand_row = vbox(0);
    class(&brand_row, "eb-brand-row");

    let brand_line = hbox(8);
    class(&brand_line, "eb-brand-line");
    // EvoBuddy's own mark, embedded as a PNG (no icon theme / librsvg path).
    let brand_mark = crate::icons::named("logo", 18, "eb-brand-name");
    brand_line.pack_start(&brand_mark, false, false, 0);
    brand_line.pack_start(&text(spec::APP_NAME, "eb-brand-name"), false, false, 0);

    let version_row = hbox(6);
    class(&version_row, "eb-version-row");
    version_row.pack_start(&text(spec::APP_VERSION, "eb-brand-version"), false, false, 0);

    brand_row.pack_start(&brand_line, false, false, 0);
    brand_row.pack_start(&version_row, false, false, 0);
    header.pack_start(&brand_row, false, false, 0);

    let nav = vbox(2);
    let mut buttons = Vec::new();
    for (index, row) in contribs.nav.iter().enumerate() {
        let button = nav_row(row.glyph, row.label, row.badge);
        button.connect_clicked({
            let nav = wiring.nav.clone();
            move |_| nav(index)
        });
        nav.pack_start(&button, false, false, 0);
        buttons.push(button);
    }
    header.pack_start(&nav, false, false, 0);

    (header, buttons)
}

/// Measured: 255x32, radius 8px, `padding: 0 12px`, 8px icon/label gap. The
/// active row is #ebebeb at weight 600; an inactive row is transparent at 400.
pub fn nav_row(value: &str, label: &str, badge: &str) -> gtk::Button {
    let button = flat_button("eb-nav");

    let row = hbox(8);
    let icon = crate::icons::request(value, "eb-nav-icon");
    row.pack_start(&icon, false, false, 0);
    row.pack_start(&text(label, ""), true, true, 0);

    if !badge.is_empty() {
        row.pack_end(&text(badge, "eb-nav-badge"), false, false, 0);
    }

    button.add(&row);
    button
}

/// 279x56.5, padding `8px 12px 8px 8px`: the user row (223x40) plus a 32x32
/// mini-program button.
fn footer(contribs: &Contribs, account: &std::rc::Rc<dyn Fn(usize)>) -> gtk::Box {
    let footer = hbox(0);
    class(&footer, "eb-sidebar-footer");

    let user = flat_button("eb-user");
    attach_menu(
        &user,
        contribs.menu("account"),
        std::rc::Rc::new(std::cell::Cell::new(0)),
        gtk::PositionType::Top,
        account.clone(),
    );

    let row = hbox(10);
    let avatar = gtk::Label::new(Some("E"));
    avatar.set_size_request(28, 28);
    avatar.set_valign(gtk::Align::Center);
    class(&avatar, "eb-avatar");

    row.pack_start(&avatar, false, false, 0);
    // The product shows the signed-in account here; this is a placeholder.
    row.pack_start(&text("EvoBuddy User", "eb-user-name"), true, true, 0);
    user.add(&row);

    footer.pack_start(&user, true, true, 0);

    let mini = icon_button("▣", "eb-icon-small", "Mini programs");
    attach_notice(&mini, contribs.notice("sidebar.mini_programs"));
    footer.pack_end(&mini, false, false, 0);

    footer
}

// --------------------------------------------------------- dynamic content

/// Repopulate the sidebar's scroll region.
///
/// With no tasks the product shows a **top-aligned** empty state (`padding:
/// 24px 12px`, 255x109) rather than a centred one. With tasks it shows a
/// `Tasks (n)` section and a `Workspaces (0)` section, each introduced by a
/// 30px tinted label pill.
///
/// The label pills collapse their section. That is this build's own behaviour:
/// the capture shows the chevron but never caught a click on it (ui-spec §10).
pub fn fill_content(
    content: &gtk::Box,
    rows: &[TaskRow],
    collapsed: [bool; 2],
    open_task: &std::rc::Rc<dyn Fn(usize)>,
    section_toggle: &std::rc::Rc<dyn Fn(usize)>,
) {
    clear(content);

    if rows.is_empty() {
        let empty = vbox(2);
        class(&empty, "eb-empty");

        empty.pack_start(&text(spec::EMPTY_TITLE, "eb-empty-title"), false, false, 0);
        let body = gtk::Label::new(Some(spec::EMPTY_BODY));
        body.set_line_wrap(true);
        body.set_xalign(0.5);
        body.set_justify(gtk::Justification::Center);
        class(&body, "eb-empty-description");
        empty.pack_start(&body, false, false, 0);

        content.pack_start(&empty, false, false, 0);
        content.show_all();
        return;
    }

    // Measured: the Tasks section is 114 tall = 30 label + 64 list + 20 padding,
    // and the Workspaces section below it is 64.5 = 30 + 30.5 + 4.
    let list = vbox(2);
    for row in rows {
        list.pack_start(&task_row(row, open_task), false, false, 0);
    }
    let (tasks, tasks_body) = section(
        &format!("Tasks ({})", rows.len()),
        20,
        list,
        collapsed[0],
        0,
        section_toggle,
    );
    content.pack_start(&tasks, false, false, 0);

    let workspace_rows: Vec<&str> = rows
        .iter()
        .filter_map(|row| row.workspace.as_deref())
        .collect();
    let workspaces_list = vbox(2);
    if workspace_rows.is_empty() {
        workspaces_list.pack_start(&text("No tasks", "eb-task-placeholder"), false, false, 0);
    } else {
        for path in &workspace_rows {
            let name = std::path::Path::new(path)
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| (*path).to_string());
            workspaces_list.pack_start(&ellipsized(&name, "eb-task-title"), false, false, 0);
        }
    }
    let (workspaces, workspaces_body) = section(
        &format!("Workspaces ({})", workspace_rows.len()),
        4,
        workspaces_list,
        collapsed[1],
        1,
        section_toggle,
    );
    content.pack_start(&workspaces, false, false, 0);

    // `show_all` shows every body, folded or not, so the fold has to be applied
    // *after* it. Opting a body out of `show_all` is not an option either:
    // `gtk_widget_show_all` returns immediately for a `no_show_all` widget, so
    // its children would never be shown at all.
    content.show_all();
    for (body, folded) in [
        (&tasks_body, collapsed[0]),
        (&workspaces_body, collapsed[1]),
    ] {
        body.set_visible(!folded);
    }
}

/// A 30px tinted label pill with a chevron, and the body it folds away.
///
/// The body is returned as well, because the caller has to fold it *after* the
/// container's `show_all` has run over it.
fn section(
    label: &str,
    bottom_pad: i32,
    body: gtk::Box,
    collapsed: bool,
    index: usize,
    toggle: &std::rc::Rc<dyn Fn(usize)>,
) -> (gtk::Box, gtk::Box) {
    let block = vbox(0);
    block.set_margin_bottom(bottom_pad);

    let head = flat_button("eb-section-label");
    head.set_tooltip_text(Some(if collapsed {
        "Expand section"
    } else {
        "Collapse section"
    }));
    head.connect_clicked({
        let toggle = toggle.clone();
        move |_| toggle(index)
    });

    let row = hbox(4);
    row.pack_start(&text(label, "eb-section-title"), true, true, 0);
    row.pack_start(&glyph(if collapsed { "›" } else { "⌄" }), false, false, 0);
    head.add(&row);

    block.pack_start(&head, false, false, 0);
    block.pack_start(&body, false, false, 0);
    (block, body)
}

/// Measured: 255x31, radius 8px, `padding: 0 12px`; the active row is #ebebeb.
fn task_row(row: &TaskRow, open_task: &std::rc::Rc<dyn Fn(usize)>) -> gtk::Button {
    let button = flat_button("eb-task");
    if row.active {
        class(&button, "eb-task-active");
    }

    let line = hbox(8);
    line.pack_start(&ellipsized(&row.title, "eb-task-title"), true, true, 0);
    if !row.meta.is_empty() {
        line.pack_end(&text(&row.meta, "eb-task-meta"), false, false, 0);
    }
    button.add(&line);

    let index = row.index;
    button.connect_clicked({
        let open_task = open_task.clone();
        move |_| open_task(index)
    });
    button
}
