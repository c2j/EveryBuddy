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
use eb_api::model::{TaskStatus, Workspace};
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
    /// What the latest agent turn reports. The row's trailing slot is driven by
    /// this: working and planning both spin, and a glyph in that slot replaces
    /// the timestamp the way the product's `trailingStatus` hides its
    /// `timeContainer` (ui-spec §10-40).
    pub status: TaskStatus,
    /// A failure the user has not opened the task for yet: the red badge stays
    /// until `open_task` clears it.
    pub failure_unseen: bool,
    /// A turn settled while the task was in the background: the row carries the
    /// unread dot next to its timestamp until the task is opened.
    pub unread: bool,
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
/// 24px 12px`, 255x109) rather than a centred one. With tasks it shows the
/// sections, each introduced by a label pill: `Pinned (n)` only while anything
/// is pinned, then `Tasks (n)` and `Workspaces (n)`.
///
/// The label pills collapse their section. That is this build's own behaviour:
/// the capture shows the chevron but never caught a click on it (ui-spec §10).
pub fn fill_content(
    content: &gtk::Box,
    rows: &[TaskRow],
    workspaces: &[Workspace],
    collapsed: [bool; 3],
    open_task: &std::rc::Rc<dyn Fn(usize)>,
    section_toggle: &std::rc::Rc<dyn Fn(usize)>,
    workspace_action: &std::rc::Rc<dyn Fn(WorkspaceAction, String)>,
    new_workspace: &std::rc::Rc<dyn Fn()>,
    task_action: &std::rc::Rc<dyn Fn(TaskAction, usize)>,
    workspace_folded: &std::collections::BTreeMap<String, bool>,
    workspace_toggle: &std::rc::Rc<dyn Fn(String)>,
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

    // The product partitions the list before it groups it: `buildGroupedResult`
    // drops archived rows, and `isPinned === true` pushes onto the pinned array
    // and `continue`s - so a pinned task is drawn in Pinned *only*, never again
    // under Tasks or under its own workspace (ui-spec §10-41).
    let pinned: Vec<&TaskRow> = rows.iter().filter(|row| row.pinned).collect();
    let rest: Vec<&TaskRow> = rows.iter().filter(|row| !row.pinned).collect();
    let standalone: Vec<&TaskRow> = rest
        .iter()
        .copied()
        .filter(|row| row.workspace_id.is_none())
        .collect();
    let mut grouped: BTreeMap<String, Vec<&TaskRow>> = BTreeMap::new();
    for row in rest.iter().filter(|row| row.workspace_id.is_some()) {
        grouped
            .entry(row.workspace_id.clone().unwrap_or_default())
            .or_default()
            .push(row);
    }

    // `PinnedSection` returns `null` while nothing is pinned, so the section is
    // absent rather than empty. `.conversation-pinned-list{gap:2px}` is the
    // same spacing the other lists use, and the section closes with 4px of
    // padding when folded and 20px when open.
    let pinned_section = if pinned.is_empty() {
        None
    } else {
        let list = vbox(2);
        for row in &pinned {
            list.pack_start(&task_row(row, open_task, false, task_action), false, false, 0);
        }
        let parts = section(
            &format!("Pinned ({})", pinned.len()),
            if collapsed[0] { 4 } else { 20 },
            list,
            collapsed[0],
            0,
            section_toggle,
            None,
        );
        content.pack_start(&parts.0, false, false, 0);
        Some(parts)
    };

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
        collapsed[1],
        1,
        section_toggle,
        None,
    );
    content.pack_start(&tasks, false, false, 0);

    let workspaces_list = vbox(2);
    if workspaces.is_empty() {
        workspaces_list.pack_start(&text("No workspaces", "eb-task-placeholder"), false, false, 0);
    } else {
        for workspace in workspaces {
            // The folder icon is the fold: open shows the group's tasks,
            // closed hides them. WorkBuddy's capture has no folding here
            // (ui-spec §10-41), so the open/closed pair is this build's own
            // behaviour, like the section pills.
            let folded = workspace_folded
                .get(&workspace.id)
                .copied()
                .unwrap_or(false);
            let header = workspace_header(workspace, workspace_action, folded, workspace_toggle);
            workspaces_list.pack_start(&header, false, false, 0);
            if folded {
                continue;
            }
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
        collapsed[2],
        2,
        section_toggle,
        Some(header_plus),
    );
    content.pack_start(&workspaces_section, false, false, 0);

    // `show_all` shows every body, folded or not, so the fold has to be applied
    // *after* it. Opting a body out of `show_all` is not an option either:
    // `gtk_widget_show_all` returns immediately for a `no_show_all` widget, so
    // its children would never be shown at all.
    let mut fold: Vec<(&gtk::Box, bool)> = Vec::new();
    if let Some((_, body)) = pinned_section.as_ref() {
        fold.push((body, collapsed[0]));
    }
    fold.push((&tasks_body, collapsed[1]));
    fold.push((&workspaces_body, collapsed[2]));
    content.show_all();
    for (body, folded) in fold {
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
    folded: bool,
    toggle: &std::rc::Rc<dyn Fn(String)>,
) -> gtk::Box {
    // WorkBuddy row: [icon] name ..... [...] [+]. Both round buttons reveal on
    // hover; "..." opens Open Folder / Rename / Delete Workspace, "+" starts a
    // new task in this workspace.
    let container = hbox(2);

    let fold = flat_button("eb-workspace-fold");
    fold.set_tooltip_text(Some(if folded {
        "Expand workspace tasks"
    } else {
        "Fold workspace tasks"
    }));
    // Open folder while the tasks show, closed once they are folded away.
    let mark = if folded {
        crate::icons::image("folder", 16)
    } else {
        crate::icons::image("folder-open", 16)
    };
    match mark {
        Some(image) => fold.add(&image),
        None => fold.add(&glyph(if folded { "▱" } else { "▱" })),
    }
    {
        let toggle = toggle.clone();
        let id = workspace.id.clone();
        fold.connect_clicked(move |_| toggle(id.clone()));
    }

    let button = flat_button("eb-workspace");
    button.set_tooltip_text(Some("Workspace actions"));
    let row = hbox(8);
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

    container.pack_start(&fold, false, false, 0);
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
        line.pack_start(&pin_button(row, task_action), false, false, 0);
    }
    line.pack_start(&ellipsized(&row.title, "eb-task-title"), true, true, 0);
    // The trailing slot is 16x16 in the product and the row's timestamp is
    // hidden exactly while something stands in it (`:has([class*=trailingStatus])
    // [class*=timeContainer]{display:none}`). The priority is the bundle's:
    // `failed|error|terminated` shows the failed icon until the task is seen,
    // then nothing; a working or planning turn spins in `--wb-palette-brand-8`.
    let running = matches!(row.status, TaskStatus::Working | TaskStatus::Planning);
    let badged = matches!(row.status, TaskStatus::Failed) && row.failure_unseen;
    // `showUnreadDot: conv.unread && !working` - the dot rides alongside the
    // timestamp instead of replacing it, and never while a turn is running.
    let mut marks: Vec<gtk::Widget> = Vec::new();
    if row.unread && !running {
        let dot = unread_dot();
        marks.push(dot.clone().upcast());
        line.pack_end(&dot, false, false, 0);
    }
    if running {
        // Capture: a running task's row shows a spinner where the timestamp
        // would be. Clock-driven, so sidebar rebuilds mid-stream keep it
        // turning (widgets::spinner).
        let spinner = crate::widgets::spinner(16);
        class(&spinner, "eb-row-spinner");
        marks.push(spinner.clone().upcast());
        line.pack_end(&spinner, false, false, 0);
    } else if badged {
        let badge = failure_badge();
        marks.push(badge.clone());
        line.pack_end(&badge, false, false, 0);
    } else if !row.meta.is_empty() {
        let meta = text(&row.meta, "eb-task-meta");
        marks.push(meta.clone().upcast());
        line.pack_end(&meta, false, false, 0);
    }
    button.add(&line);

    let index = row.index;
    button.connect_clicked({
        let open_task = open_task.clone();
        move |_| open_task(index)
    });
    wrapper.pack_start(&button, true, true, 0);

    let fade = RowFade::new(marks);
    button.connect_enter_notify_event({
        let fade = fade.clone();
        move |_, _| {
            fade.set(RowFade::HOVER, true);
            gtk::glib::Propagation::Proceed
        }
    });
    button.connect_leave_notify_event({
        let fade = fade.clone();
        move |_, _| {
            fade.set(RowFade::HOVER, false);
            gtk::glib::Propagation::Proceed
        }
    });

    let menu = flat_button("eb-task-menu");
    menu.set_tooltip_text(Some("Task actions"));
    menu.add(&glyph("⋯"));
    attach_task_menu(&menu, &fade, row, task_action);
    wrapper.pack_start(&menu, false, false, 0);
    wrapper
}

/// The two reasons a row's trailing marks disappear: the pointer is on the row,
/// or the row's own menu is open - the product's `:hover` and `_menuOpen` rules,
/// which drop `timeContainer`, `timePrefixIcon` and `trailingStatus` together.
/// GTK CSS has no `display`, so the marks fade through GtkWidget opacity; they
/// stay allocated their space, which keeps the row from reflowing (ui-spec
/// §10-40).
#[derive(Clone)]
struct RowFade {
    hidden: std::rc::Rc<std::cell::Cell<u8>>,
    marks: std::rc::Rc<Vec<gtk::Widget>>,
}

impl RowFade {
    const HOVER: u8 = 1;
    const MENU: u8 = 2;

    fn new(marks: Vec<gtk::Widget>) -> RowFade {
        RowFade {
            hidden: std::rc::Rc::new(std::cell::Cell::new(0)),
            marks: std::rc::Rc::new(marks),
        }
    }

    fn set(&self, reason: u8, on: bool) {
        let hidden = self.hidden.get();
        self.hidden.set(if on { hidden | reason } else { hidden & !reason });
        let alpha = if self.hidden.get() == 0 { 1.0 } else { 0.0 };
        for mark in self.marks.iter() {
            mark.set_opacity(alpha);
        }
    }
}

/// The pin mark in a pinned row, and the product's un-pin control: the base
/// card keeps `._pinButtonLeft` at `display:none`, shows it at `opacity:.3`
/// under `_pinned`, and `.conversation-pinned-item` raises it to full opacity.
/// Since a pinned task is only ever drawn inside the pinned list, one look is
/// enough here. The hover swap to the un-pin icon is not implemented (§10-41).
fn pin_button(row: &TaskRow, action: &std::rc::Rc<dyn Fn(TaskAction, usize)>) -> gtk::Button {
    let button = flat_button("eb-row-pin");
    button.set_tooltip_text(Some("Unpin"));
    button.add(&glyph("pin"));
    let index = row.index;
    let action = std::rc::Rc::clone(action);
    button.connect_clicked(move |_| action(TaskAction::Pin, index));
    button
}

/// The unread mark: a 6px dot in `#00c29a`, the product's `--wb-accent-unread`.
/// The list CSS rewrites the row's own dot for this surface - `.conversation-
/// agent-card [class*=unreadDot]` becomes a 16x16 `position:static` slot with
/// `order:99` and the 6px mark in its `:before` - so it rides in the trailing
/// row, not over the corner the way the base component's 8px `#ef4444` dot
/// does elsewhere (ui-spec §10-40).
fn unread_dot() -> gtk::Box {
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.set_size_request(6, 6);
    dot.set_valign(gtk::Align::Center);
    dot.set_halign(gtk::Align::Center);
    class(&dot, "eb-row-unread");
    dot
}

/// The unseen-failure mark in a row's trailing 16x16 slot: the product's
/// `TaskFailedIcon`, whose dark-theme layer (`__solid`, `display:inline` under
/// `.cb-dark`) is a disc of radius 6.5 filled `#f64041` with the exclamation
/// knocked out to transparency. Drawn as `icons/task-failed.png` from those
/// path coordinates; the light theme's line-art ring variant is not
/// implemented because EvoBuddy only ships the dark capture (ui-spec §10-40).
fn failure_badge() -> gtk::Widget {
    // The pixbuf decode can fail in a bare environment (a missing
    // gdk-pixbuf loader cache, as in the Debian 10 verify container), so a
    // text glyph backs the image up instead of aborting the app.
    let badge: gtk::Widget = match crate::icons::image("task-failed", 16) {
        Some(image) => {
            class(&image, "eb-row-alert");
            image.set_valign(gtk::Align::Center);
            image.set_tooltip_text(Some("Task failed"));
            image.upcast()
        }
        None => {
            let fallback = gtk::Label::new(Some("!"));
            class(&fallback, "eb-row-alert");
            fallback.set_valign(gtk::Align::Center);
            fallback.set_tooltip_text(Some("Task failed"));
            fallback.upcast()
        }
    };
    badge
}

fn attach_task_menu(
    button: &gtk::Button,
    fade: &RowFade,
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
    // The product's `_menuOpen` hides the row's timestamp and status glyph the
    // same way hover does, so the menu never opens over a fighting state mark.
    popover.connect_show({
        let fade = fade.clone();
        move |_| fade.set(RowFade::MENU, true)
    });
    popover.connect_hide({
        let fade = fade.clone();
        move |_| fade.set(RowFade::MENU, false)
    });
}
