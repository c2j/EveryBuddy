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

use std::collections::BTreeMap;

use gtk::prelude::*;

use eb_api::contrib::Contribs;
use eb_api::model::{Workspace, WorkspaceState};
use crate::spec;
use crate::widgets::*;

/// Callbacks the sidebar raises.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TaskAction {
    Pin,
    Rename,
    SaveToWorkspace,
    ShareTask,
    OpenWindow,
    OpenFolder,
    Archive,
    Delete,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceAction {
    NewTask,
    OpenFolder,
    Rename,
    Delete,
}

#[derive(Clone)]
pub struct Wiring {
    /// A nav row was chosen.
    pub nav: std::rc::Rc<dyn Fn(usize)>,
    /// The topbar's collapse button.
    pub collapse: std::rc::Rc<dyn Fn()>,
    /// Reads the active sidebar filters (status, time).
    pub filter_state: std::rc::Rc<dyn Fn() -> (usize, usize)>,
    /// A filter row was chosen: (0 = status / 1 = time, choice index).
    pub filter_select: std::rc::Rc<dyn Fn(usize, usize)>,
    /// Reset Filters was chosen.
    pub filter_reset: std::rc::Rc<dyn Fn()>,
    /// Opens the global search popover anchored to the given widget.
    pub search: std::rc::Rc<dyn Fn(&gtk::Button)>,
    /// A row of the account menu was chosen (0 = Settings).
    pub account: std::rc::Rc<dyn Fn(usize)>,
    /// Workspace header menu action; the id is empty for NewUnassigned.
    pub workspace_action: std::rc::Rc<dyn Fn(WorkspaceAction, String)>,
    /// The Workspaces-section header "+" (new workspace dialog).
    pub new_workspace: std::rc::Rc<dyn Fn()>,
    /// Task row menu action.
    pub task_action: std::rc::Rc<dyn Fn(TaskAction, usize)>,
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
    /// The latest agent turn is still streaming: the row shows a spinner
    /// where the timestamp would be.
    pub running: bool,
    pub active: bool,
    pub index: usize,
    pub workspace_id: Option<String>,
    pub pinned: bool,
    pub archived: bool,
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

    let search_button = {
        let search = wiring.search.clone();
        let button = icon_button("◉", "eb-icon", "Search");
        let button_for_click = button.clone();
        button.connect_clicked(move |_| search(&button_for_click));
        button
    };
    let topbar = topbar(&collapse, &filter, &search_button, contribs);
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
fn topbar(
    collapse: &gtk::Button,
    filter: &gtk::Button,
    search: &gtk::Button,
    _contribs: &Contribs,
) -> gtk::Box {
    let bar = hbox(4);
    class(&bar, "eb-sidebar-topbar");

    // The product opens a dedicated search window; this build pops the same
    // search over the sidebar. The button is pre-wired by the caller.

    bar.pack_end(filter, false, false, 0);
    bar.pack_end(search, false, false, 0);
    bar.pack_end(collapse, false, false, 0);

    bar
}

/// Kept as a function because the shell holds this button: the task-filter menu
/// is the one sidebar dropdown whose selection is not part of `App`.
fn filter_button(wiring: &Wiring, _contribs: &Contribs) -> gtk::Button {
    let button = icon_button("≡", "eb-icon", "Filter tasks");
    button.connect_clicked({
        let wiring = wiring.clone();
        let button = button.clone();
        move |_| build_filter_menu(&button, &wiring)
    });
    button
}

/// WorkBuddy's filter popover: Filter Status, Filter Time, Reset Filters.
///
/// Geometry and colors are the 2026-10-24 product capture (spec §5.4): 240px
/// card, 37px section headers, 31px rows at a 34px pitch with an 8px-radius
/// `#f7f7f7` pill and a right-aligned green check on the selected row.
fn build_filter_menu(button: &gtk::Button, wiring: &Wiring) {
    let popover = gtk::Popover::new(Some(button));
    popover.set_position(gtk::PositionType::Bottom);
    class(&popover, "eb-menu");
    class(&popover, "eb-filter");
    let column = vbox(0);
    class(&column, "eb-filter-body");
    // The card is 240 CSS px including its 1px border.
    column.set_size_request(238, -1);

    let (status, time) = (wiring.filter_state)();

    let header = |label: &str, column: &gtk::Box| {
        let row = text(label, "eb-filter-header");
        column.pack_start(&row, false, false, 0);
    };
    let section = |labels: &[&'static str],
                   selected: usize,
                   kind: usize,
                   column: &gtk::Box,
                   wiring: &Wiring,
                   popover: &gtk::Popover| {
        let rows = vbox(3);
        for (index, label) in labels.iter().enumerate() {
            let row = flat_button("eb-menu-item");
            class(&row, "eb-filter-row");
            if index == selected {
                class(&row, "eb-filter-row-selected");
            }
            let inner = hbox(8);
            inner.pack_start(&text(label, "eb-filter-label"), true, true, 0);
            if index == selected {
                inner.pack_end(&text("✓", "eb-filter-check"), false, false, 0);
            }
            row.add(&inner);
            let select = wiring.filter_select.clone();
            let popover = popover.clone();
            row.connect_clicked(move |_| {
                popover.popdown();
                select(kind, index);
            });
            rows.pack_start(&row, false, false, 0);
        }
        column.pack_start(&rows, false, false, 0);
    };
    let separator = |column: &gtk::Box| {
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        class(&separator, "eb-menu-separator");
        class(&separator, "eb-filter-separator");
        column.pack_start(&separator, false, false, 0);
    };

    header("Filter Status", &column);
    section(&eb_api::model::FILTER_STATUS_LABELS, status, 0, &column, wiring, &popover);
    separator(&column);
    header("Filter Time", &column);
    section(&eb_api::model::FILTER_TIME_LABELS, time, 1, &column, wiring, &popover);
    separator(&column);

    let reset = flat_button("eb-menu-item");
    class(&reset, "eb-filter-row");
    class(&reset, "eb-filter-reset");
    reset.add(&text("Reset Filters", "eb-filter-label"));
    let reset_all = wiring.filter_reset.clone();
    let popover_for_reset = popover.clone();
    reset.connect_clicked(move |_| {
        popover_for_reset.popdown();
        reset_all();
    });
    column.pack_start(&reset, false, false, 0);

    popover.add(&column);
    column.show_all();
    hide_from_show_all(&popover);
    popover.popup();
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
    workspaces: &[Workspace],
    collapsed: [bool; 2],
    open_task: &std::rc::Rc<dyn Fn(usize)>,
    section_toggle: &std::rc::Rc<dyn Fn(usize)>,
    workspace_action: &std::rc::Rc<dyn Fn(WorkspaceAction, String)>,
    new_workspace: &std::rc::Rc<dyn Fn()>,
    task_action: &std::rc::Rc<dyn Fn(TaskAction, usize)>,
) {
    clear(content);

    if rows.is_empty() && workspaces.is_empty() {
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

    // WorkBuddy separates unassigned tasks from tasks grouped under a
    // workspace. A task appears exactly once: under Tasks or under its
    // workspace. Workspace entities render even when they have no tasks.
    let standalone: Vec<&TaskRow> = rows
        .iter()
        .filter(|row| row.workspace_id.is_none())
        .collect();
    let mut grouped: BTreeMap<String, Vec<&TaskRow>> = BTreeMap::new();
    for row in rows.iter().filter(|row| row.workspace_id.is_some()) {
        grouped
            .entry(row.workspace_id.clone().unwrap_or_default())
            .or_default()
            .push(row);
    }

    let list = vbox(2);
    if standalone.is_empty() {
        list.pack_start(&text("No tasks", "eb-task-placeholder"), false, false, 0);
    } else {
        for row in &standalone {
            list.pack_start(&task_row(row, open_task, false, task_action), false, false, 0);
        }
    }
    let (tasks, tasks_body) = section(
        &format!("Tasks ({})", standalone.len()),
        20,
        list,
        collapsed[0],
        0,
        section_toggle,
        None,
    );
    content.pack_start(&tasks, false, false, 0);

    let workspaces_list = vbox(2);
    if workspaces.is_empty() {
        workspaces_list.pack_start(&text("No workspaces", "eb-task-placeholder"), false, false, 0);
    } else {
        for workspace in workspaces {
            let header = workspace_header(workspace, workspace_action);
            workspaces_list.pack_start(&header, false, false, 0);
            if let Some(tasks_in_group) = grouped.get(&workspace.id) {
                for row in tasks_in_group {
                    workspaces_list.pack_start(
                        &task_row(row, open_task, true, task_action),
                        false,
                        false,
                        0,
                    );
                }
            }
        }
    }
    // WorkBuddy's Workspaces header carries a folder-plus button that opens
    // the new-workspace dialog.
    let header_plus = flat_button("eb-header-plus");
    header_plus.set_tooltip_text(Some("New Workspace"));
    header_plus.add(&glyph("⊞"));
    {
        let open_new_workspace = new_workspace.clone();
        header_plus.connect_clicked(move |_| open_new_workspace());
    }
    let (workspaces_section, workspaces_body) = section(
        &format!("Workspaces ({})", workspaces.len()),
        4,
        workspaces_list,
        collapsed[1],
        1,
        section_toggle,
        Some(header_plus),
    );
    content.pack_start(&workspaces_section, false, false, 0);

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
    header_extra: Option<gtk::Button>,
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

    let head_row = hbox(4);
    head_row.pack_start(&head, true, true, 0);
    if let Some(extra) = header_extra {
        head_row.pack_end(&extra, false, false, 0);
    }

    block.pack_start(&head_row, false, false, 0);
    block.pack_start(&body, false, false, 0);
    (block, body)
}

fn workspace_header(
    workspace: &Workspace,
    action: &std::rc::Rc<dyn Fn(WorkspaceAction, String)>,
) -> gtk::Box {
    // WorkBuddy row: [icon] name ..... [...] [+]. Both round buttons reveal on
    // hover; "..." opens Open Folder / Rename / Delete Workspace, "+" starts a
    // new task in this workspace.
    let container = hbox(2);

    let button = flat_button("eb-workspace");
    button.set_tooltip_text(Some("Workspace actions"));
    let row = hbox(8);
    row.pack_start(&glyph("▱"), false, false, 0);
    row.pack_start(
        &ellipsized(&crate::workspace::display_name(workspace), "eb-workspace-title"),
        true,
        true,
        0,
    );
    // The row itself (icon + name) is always painted, like WorkBuddy; only the
    // "..." affordance and the trailing "+" round button reveal on hover.
    let dots = glyph("⋯");
    class(&dots, "eb-ghost");
    row.pack_end(&dots, false, false, 0);
    button.add(&row);

    let popover = gtk::Popover::new(Some(&button));
    popover.set_position(gtk::PositionType::Bottom);
    class(&popover, "eb-menu");
    let column = vbox(0);
    class(&column, "eb-menu-body");
    let rows: Vec<(&str, WorkspaceAction)> = vec![
        ("Open Folder", WorkspaceAction::OpenFolder),
        ("Rename", WorkspaceAction::Rename),
        ("Delete Workspace", WorkspaceAction::Delete),
    ];
    for (label, kind) in rows {
        let menu_row = flat_button("eb-menu-item");
        if kind == WorkspaceAction::Delete {
            class(&menu_row, "eb-menu-item-danger");
        }
        menu_row.add(&text(label, "eb-menu-item-label"));
        let id = workspace.id.clone();
        let action = action.clone();
        let popover = popover.clone();
        menu_row.connect_clicked(move |_| {
            popover.popdown();
            action(kind, id.clone());
        });
        column.pack_start(&menu_row, false, false, 0);
    }
    popover.add(&column);
    column.show_all();
    hide_from_show_all(&popover);
    button.connect_clicked({
        let popover = popover.clone();
        move |_| popover.popup()
    });

    let plus = flat_button("eb-header-plus");
    plus.set_tooltip_text(Some("New Task"));
    plus.add(&glyph("⊞"));
    {
        let action = action.clone();
        let id = workspace.id.clone();
        plus.connect_clicked(move |_| action(WorkspaceAction::NewTask, id.clone()));
    }

    // Hover reveals the "..." affordance and the trailing "+" round button,
    // like WorkBuddy. They stay realised (opacity 0) so the row keeps its
    // height and hit area; only the paint toggles.
    class(&plus, "eb-ghost");
    // Boxes have no GDK window, so the crossing events have to live on the
    // buttons themselves. Leaving one button ghosts both; entering the other
    // immediately unghosts them again, so the pair looks stable while the
    // pointer stays on the row.
    for sensor in [&button, &plus] {
        let revealed: Vec<gtk::Widget> = vec![dots.clone().upcast(), plus.clone().upcast()];
        sensor.connect_enter_notify_event(move |_, _| {
            for widget in &revealed {
                widget.style_context().remove_class("eb-ghost");
            }
            gtk::glib::Propagation::Proceed
        });
        let revealed: Vec<gtk::Widget> = vec![dots.clone().upcast(), plus.clone().upcast()];
        sensor.connect_leave_notify_event(move |_, _| {
            for widget in &revealed {
                widget.style_context().add_class("eb-ghost");
            }
            gtk::glib::Propagation::Proceed
        });
    }

    container.pack_start(&button, true, true, 0);
    container.pack_start(&plus, false, false, 0);
    container
}

/// Measured: 255x31, radius 8px, `padding: 0 12px`; the active row is #ebebeb.
fn task_row(
    row: &TaskRow,
    open_task: &std::rc::Rc<dyn Fn(usize)>,
    in_workspace: bool,
    task_action: &std::rc::Rc<dyn Fn(TaskAction, usize)>,
) -> gtk::Box {
    // The row is a flat button plus a trailing `...` action button (WorkBuddy
    // parity): the button opens the task, `...` opens its menu.
    let wrapper = hbox(0);
    class(&wrapper, "eb-task-row");

    let button = flat_button("eb-task");
    if row.active {
        class(&button, "eb-task-active");
    }
    if in_workspace {
        class(&button, "eb-task-child");
    }
    if row.archived {
        class(&button, "eb-task-archived");
    }

    let line = hbox(8);
    if row.pinned {
        line.pack_start(&glyph("pin"), false, false, 0);
    }
    line.pack_start(&ellipsized(&row.title, "eb-task-title"), true, true, 0);
    if row.running {
        // Capture: a running task's row shows a spinner where the timestamp
        // would be. Clock-driven, so sidebar rebuilds mid-stream keep it
        // turning (widgets::spinner).
        line.pack_end(&crate::widgets::spinner(16), false, false, 0);
    } else if !row.meta.is_empty() {
        line.pack_end(&text(&row.meta, "eb-task-meta"), false, false, 0);
    }
    button.add(&line);

    let index = row.index;
    button.connect_clicked({
        let open_task = open_task.clone();
        move |_| open_task(index)
    });
    wrapper.pack_start(&button, true, true, 0);

    let menu = flat_button("eb-task-menu");
    menu.set_tooltip_text(Some("Task actions"));
    menu.add(&glyph("⋯"));
    attach_task_menu(&menu, row, task_action);
    wrapper.pack_start(&menu, false, false, 0);
    wrapper
}

fn attach_task_menu(
    button: &gtk::Button,
    row: &TaskRow,
    action: &std::rc::Rc<dyn Fn(TaskAction, usize)>,
) {
    let popover = gtk::Popover::new(Some(button));
    popover.set_position(gtk::PositionType::Bottom);
    class(&popover, "eb-menu");
    let column = vbox(0);
    class(&column, "eb-menu-body");
    // WorkBuddy's menu groups: identity (Pin/Rename/Share Task), window and
    // folder access ("Save to Workspace…" replaces "Open Folder" for
    // unassigned tasks, which have no backing folder), then the destructive
    // Archive/Delete group.
    let mut groups: Vec<Vec<(&str, TaskAction)>> = vec![vec![
        (
            if row.pinned { "Unpin" } else { "Pin" },
            TaskAction::Pin,
        ),
        ("Rename…", TaskAction::Rename),
        ("Share Task", TaskAction::ShareTask),
    ]];
    let mut window_group = vec![("Open in a separate window", TaskAction::OpenWindow)];
    if row.workspace_id.is_none() {
        window_group.push(("Save to Workspace…", TaskAction::SaveToWorkspace));
    } else {
        window_group.push(("Open Folder", TaskAction::OpenFolder));
    }
    groups.push(window_group);
    groups.push(vec![
        (
            if row.archived { "Unarchive" } else { "Archive" },
            TaskAction::Archive,
        ),
        ("Delete Task", TaskAction::Delete),
    ]);

    for (group_index, group) in groups.iter().enumerate() {
        if group_index > 0 {
            let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
            class(&separator, "eb-menu-separator");
            column.pack_start(&separator, false, false, 4);
        }
        for (label, kind) in group {
            let menu_row = flat_button("eb-menu-item");
            if *kind == TaskAction::Delete {
                class(&menu_row, "eb-menu-item-danger");
            }
            menu_row.add(&text(label, "eb-menu-item-label"));
            let index = row.index;
            let kind = *kind;
            let action = action.clone();
            let popover = popover.clone();
            menu_row.connect_clicked(move |_| {
                popover.popdown();
                action(kind, index);
            });
            column.pack_start(&menu_row, false, false, 0);
        }
    }
    popover.add(&column);
    column.show_all();
    hide_from_show_all(&popover);
    button.connect_clicked({
        let popover = popover.clone();
        move |_| popover.popup()
    });
}
