//! Scheduled page: one-shot jobs, the daily plan queue, and run history.
//!
//! The page is rebuilt on every refresh while the application state is already
//! borrowed, so this module does not touch that state while painting. Clicks
//! run later and may.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::model::Store;
use crate::schedule::{self, DecisionKind, PlanItem, PlanRun};
use crate::widgets::*;

thread_local! {
    static TAB: Cell<usize> = const { Cell::new(0) };
    static NOTICE: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn set_notice(text: impl Into<String>) {
    let text = text.into();
    NOTICE.with(|slot| *slot.borrow_mut() = Some(text));
}

pub fn notify_desktop(body: &str) {
    #[cfg(target_os = "macos")]
    {
        let body = apple_string(&truncate(body, 180));
        let script = format!("display notification \"{body}\" with title \"EvoBuddy\"");
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = body;
    }
}

pub fn fill(
    container: &gtk::Box,
    store: &Store,
    refresh: &Rc<dyn Fn()>,
    pump: &Rc<dyn Fn()>,
    on_run: &Rc<dyn Fn()>,
    open_task: &Rc<dyn Fn(usize)>,
) {
    let tab = TAB.with(|slot| slot.get());
    let pending = schedule::pending_count();

    if let Some(notice) = NOTICE.with(|slot| slot.borrow().clone()) {
        let bar = hbox(8);
        let label = gtk::Label::new(Some(&notice));
        label.set_xalign(0.0);
        label.set_line_wrap(true);
        label.set_hexpand(true);
        class(&label, "eb-skill-meta");
        bar.pack_start(&label, true, true, 0);
        let dismiss = gtk::Button::with_label("Dismiss");
        let refresh = refresh.clone();
        dismiss.connect_clicked(move |_| {
            NOTICE.with(|slot| *slot.borrow_mut() = None);
            refresh();
        });
        bar.pack_end(&dismiss, false, false, 0);
        container.pack_start(&bar, false, false, 0);
    }

    let header = hbox(8);
    let title = gtk::Label::new(Some("Scheduled Tasks"));
    title.set_xalign(0.0);
    class(&title, "eb-page-title");
    header.pack_start(&title, false, false, 0);
    let spacer = gtk::Label::new(None);
    spacer.set_hexpand(true);
    header.pack_start(&spacer, true, true, 0);
    let daily = gtk::Button::with_label("Daily plan");
    class(&daily, "eb-add-skill");
    let refresh_daily = refresh.clone();
    daily.connect_clicked(move |_| open_task_dialog(&refresh_daily, Mode::Plan, "Daily plan"));
    header.pack_end(&daily, false, false, 0);
    let new = gtk::Button::with_label("+ New");
    class(&new, "eb-add-skill");
    let refresh_new = refresh.clone();
    new.connect_clicked(move |_| open_task_dialog(&refresh_new, Mode::Once, "New Scheduled Task"));
    header.pack_end(&new, false, false, 0);
    container.pack_start(&header, false, false, 0);

    let tabs = hbox(8);
    for (index, label) in [
        "Tasks".to_string(),
        format!("Approvals ({pending})"),
        "Records".to_string(),
    ]
    .into_iter()
    .enumerate()
    {
        let button = gtk::Button::with_label(&label);
        class(
            &button,
            if index == tab {
                "eb-catalog-tab-active"
            } else {
                "eb-catalog-tab"
            },
        );
        let refresh = refresh.clone();
        button.connect_clicked(move |_| {
            TAB.with(|slot| slot.set(index));
            refresh();
        });
        tabs.pack_start(&button, false, false, 0);
    }
    container.pack_start(&tabs, false, false, 0);

    match tab {
        1 => fill_approvals(container, store, refresh, pump),
        2 => fill_records(container, store, refresh, open_task),
        _ => fill_jobs(container, refresh, on_run),
    }
}

fn fill_jobs(container: &gtk::Box, refresh: &Rc<dyn Fn()>, on_run: &Rc<dyn Fn()>) {
    let jobs = schedule::rows();
    if jobs.is_empty() {
        let text = gtk::Label::new(Some("No scheduled tasks found."));
        text.set_xalign(0.0);
        class(&text, "eb-skill-meta");
        container.pack_start(&text, false, false, 0);
    }
    for job in jobs {
        let card = vbox(3);
        class(&card, "eb-skill-card");
        let head = hbox(8);
        let name = gtk::Label::new(Some(&job.title));
        name.set_xalign(0.0);
        class(&name, "eb-skill-title");
        head.pack_start(&name, false, false, 0);
        let when = gtk::Label::new(Some(&job.when));
        class(&when, "eb-skill-source");
        head.pack_start(&when, false, false, 0);
        let run = gtk::Button::with_label("Run now");
        let refresh_run = refresh.clone();
        let on_run = on_run.clone();
        let index = job.index;
        let plan = job.plan;
        run.connect_clicked(move |_| {
            if !schedule::run_now(index) && plan {
                set_notice("A plan is already waiting for approval.");
            } else {
                // Run now only marks the job due. Poll once so the plan or the
                // one-shot prompt appears without waiting for the five-second timer.
                on_run();
            }
            refresh_run();
        });
        head.pack_end(&run, false, false, 0);
        let delete = gtk::Button::with_label("Delete");
        let refresh_delete = refresh.clone();
        delete.connect_clicked(move |_| {
            schedule::remove_at(index);
            refresh_delete();
        });
        head.pack_end(&delete, false, false, 0);
        card.pack_start(&head, false, false, 0);
        let body = gtk::Label::new(Some(&job.prompt));
        body.set_xalign(0.0);
        body.set_line_wrap(true);
        class(&body, "eb-skill-meta");
        card.pack_start(&body, false, false, 0);
        container.pack_start(&card, false, false, 0);
    }

    let popular = gtk::Label::new(Some("Try these popular tasks"));
    popular.set_xalign(0.0);
    class(&popular, "eb-catalog-section");
    container.pack_start(&popular, false, false, 0);
    let grid = gtk::Grid::new();
    grid.set_column_spacing(12);
    grid.set_row_spacing(12);
    grid.set_column_homogeneous(true);
    let templates = [
        ("Daily AI News Briefing", "Track key AI updates with a short daily brief."),
        ("Daily 5 English Words", "Recommend five practical words each day."),
        ("Weekly Work Report", "Review weekly PR and issue activity."),
        ("On This Day", "Pick one interesting historical event."),
        ("Daily Why", "Ask one fun why-question, then explain."),
        ("Pre-meeting Preparation", "Prepare a short agenda before a meeting."),
        ("Health Checkup Reminder", "Remind me to take a break and stretch."),
        ("Daily Bedtime Story", "Write a gentle 3-minute bedtime story."),
        ("Classic Movie Pick", "Recommend one highly acclaimed classic movie."),
    ];
    for (index, (name, description)) in templates.iter().enumerate() {
        let name = (*name).to_string();
        let description = (*description).to_string();
        let card = vbox(3);
        class(&card, "eb-skill-card");
        let title = gtk::Label::new(Some(&name));
        title.set_xalign(0.0);
        class(&title, "eb-skill-title");
        card.pack_start(&title, false, false, 0);
        let body = gtk::Label::new(Some(&description));
        body.set_xalign(0.0);
        body.set_line_wrap(true);
        class(&body, "eb-skill-meta");
        card.pack_start(&body, false, false, 0);
        let add = gtk::Button::with_label("Schedule +1m");
        add.set_halign(gtk::Align::Start);
        let refresh = refresh.clone();
        add.connect_clicked(move |_| {
            schedule::add_once(&name, &description, 60);
            refresh();
        });
        card.pack_start(&add, false, false, 0);
        grid.attach(&card, (index % 3) as i32, (index / 3) as i32, 1, 1);
    }
    container.pack_start(&grid, false, false, 0);
}

fn fill_approvals(
    container: &gtk::Box,
    store: &Store,
    refresh: &Rc<dyn Fn()>,
    pump: &Rc<dyn Fn()>,
) {
    let note = gtk::Label::new(Some(
        "Approved tasks run one at a time and pre-approve tool calls. The conversation you have open stays put.",
    ));
    note.set_xalign(0.0);
    note.set_line_wrap(true);
    class(&note, "eb-skill-meta");
    container.pack_start(&note, false, false, 0);

    let pending: Vec<PlanRun> = schedule::plans()
        .into_iter()
        .filter(|plan| plan.status == "pending")
        .collect();
    if pending.is_empty() {
        let empty = gtk::Label::new(Some("No plans waiting for approval."));
        empty.set_xalign(0.0);
        class(&empty, "eb-skill-meta");
        container.pack_start(&empty, false, false, 0);
        return;
    }
    let now = schedule::unix_now();
    for plan in pending {
        let card = vbox(4);
        class(&card, "eb-skill-card");
        let head = gtk::Label::new(Some(&format!(
            "{} · {}",
            plan.title,
            schedule::format_local(plan.created_at)
        )));
        head.set_xalign(0.0);
        class(&head, "eb-skill-title");
        card.pack_start(&head, false, false, 0);
        let countdown = deadline_line(now, plan.deadline_at);
        let meta = gtk::Label::new(Some(&countdown));
        meta.set_xalign(0.0);
        meta.set_line_wrap(true);
        class(&meta, "eb-skill-source");
        card.pack_start(&meta, false, false, 0);
        let summary = gtk::Label::new(Some(&plan.summary));
        summary.set_xalign(0.0);
        summary.set_line_wrap(true);
        class(&summary, "eb-skill-meta");
        card.pack_start(&summary, false, false, 0);
        let diff = gtk::Label::new(Some(&plan.diff));
        diff.set_xalign(0.0);
        diff.set_line_wrap(true);
        class(&diff, "eb-skill-meta");
        card.pack_start(&diff, false, false, 0);

        for item in &plan.items {
            card.pack_start(
                &item_row(store, refresh, &plan, item, true),
                false,
                false,
                0,
            );
        }

        let actions = hbox(8);
        let selected = gtk::Button::with_label("Approve selected");
        let plan_id = plan.id.clone();
        let refresh_selected = refresh.clone();
        let pump_selected = pump.clone();
        selected.connect_clicked(move |_| {
            let queued = schedule::decide(&plan_id, DecisionKind::Selected);
            if queued == 0 {
                set_notice("Select at least one item, or the plan is already decided.");
            }
            pump_selected();
            refresh_selected();
        });
        actions.pack_end(&selected, false, false, 0);
        let all = gtk::Button::with_label("Approve all");
        let plan_id = plan.id.clone();
        let refresh_all = refresh.clone();
        let pump_all = pump.clone();
        all.connect_clicked(move |_| {
            schedule::decide(&plan_id, DecisionKind::All);
            pump_all();
            refresh_all();
        });
        actions.pack_end(&all, false, false, 0);
        let skip = gtk::Button::with_label("Skip");
        let plan_id = plan.id.clone();
        let refresh_skip = refresh.clone();
        skip.connect_clicked(move |_| {
            schedule::decide(&plan_id, DecisionKind::Skip);
            set_notice("Plan skipped. Remaining items merge into the next plan.");
            refresh_skip();
        });
        actions.pack_end(&skip, false, false, 0);
        card.pack_start(&actions, false, false, 0);
        container.pack_start(&card, false, false, 0);
    }
}

fn fill_records(
    container: &gtk::Box,
    store: &Store,
    refresh: &Rc<dyn Fn()>,
    open_task: &Rc<dyn Fn(usize)>,
) {
    let mut plans = schedule::plans();
    plans.reverse();
    if plans.is_empty() {
        let empty = gtk::Label::new(Some("No plan runs yet."));
        empty.set_xalign(0.0);
        class(&empty, "eb-skill-meta");
        container.pack_start(&empty, false, false, 0);
        return;
    }
    for plan in plans {
        let card = vbox(3);
        class(&card, "eb-skill-card");
        let head = gtk::Label::new(Some(&format!(
            "{} · {} · {}",
            plan.title,
            schedule::format_local(plan.created_at),
            status_label(&plan)
        )));
        head.set_xalign(0.0);
        head.set_line_wrap(true);
        class(&head, "eb-skill-title");
        card.pack_start(&head, false, false, 0);
        if !plan.resolution.is_empty() {
            let resolution = gtk::Label::new(Some(&resolution_label(&plan.resolution)));
            resolution.set_xalign(0.0);
            resolution.set_line_wrap(true);
            class(&resolution, "eb-skill-source");
            card.pack_start(&resolution, false, false, 0);
        }
        let diff = gtk::Label::new(Some(&plan.diff));
        diff.set_xalign(0.0);
        diff.set_line_wrap(true);
        class(&diff, "eb-skill-meta");
        card.pack_start(&diff, false, false, 0);
        for item in &plan.items {
            let row = hbox(8);
            let label = gtk::Label::new(Some(&format!(
                "{} · {}",
                item.title,
                item_state(item)
            )));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_line_wrap(true);
            class(&label, "eb-skill-meta");
            row.pack_start(&label, true, true, 0);
            if item.started || item.session_id.is_some() {
                let open = gtk::Button::with_label("Open");
                let store = store.clone();
                let open_task = open_task.clone();
                let refresh = refresh.clone();
                let item_id = item.id.clone();
                open.connect_clicked(move |_| {
                    let index = store.borrow().tasks.iter().position(|task| {
                        task.plan_item_id.as_deref() == Some(item_id.as_str())
                    });
                    if let Some(index) = index {
                        open_task(index);
                    } else {
                        set_notice("That dispatched conversation is no longer in the sidebar.");
                        refresh();
                    }
                });
                row.pack_end(&open, false, false, 0);
            }
            card.pack_start(&row, false, false, 0);
        }
        container.pack_start(&card, false, false, 0);
    }
}

fn item_row(
    store: &Store,
    refresh: &Rc<dyn Fn()>,
    plan: &PlanRun,
    item: &PlanItem,
    editable: bool,
) -> gtk::Box {
    let row = hbox(8);
    let check = gtk::CheckButton::with_label(&item.title);
    check.set_active(item.selected);
    let plan_id = plan.id.clone();
    let item_id = item.id.clone();
    check.connect_toggled(move |button| {
        schedule::set_selected(&plan_id, &item_id, button.is_active());
    });
    row.pack_start(&check, true, true, 0);
    let meta = format!(
        "{} · {} · {}",
        source_label(&item.source),
        if item.workspace_name.is_empty() {
            "new workspace"
        } else {
            item.workspace_name.as_str()
        },
        item.agent_route.as_deref().unwrap_or("default agent")
    );
    let detail = gtk::Label::new(Some(&meta));
    detail.set_xalign(0.0);
    class(&detail, "eb-skill-source");
    row.pack_start(&detail, false, false, 0);
    if editable && !item.started {
        let edit = gtk::Button::with_label("Edit");
        let store = store.clone();
        let refresh = refresh.clone();
        let plan_id = plan.id.clone();
        let item_id = item.id.clone();
        let prompt = item.prompt.clone();
        edit.connect_clicked(move |_| {
            open_prompt_dialog(&store, &refresh, &plan_id, &item_id, &prompt);
        });
        row.pack_end(&edit, false, false, 0);
    }
    row
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Once,
    Periodic,
    Interval,
    Plan,
}

/// Which recurrence the Periodic tile uses. Mirrors `schedule::Periodic`.
#[derive(Clone, Copy, PartialEq)]
enum Freq {
    Day,
    Week,
    Biweek,
    Month,
    Year,
}

const MODE_TILES: [(Mode, &str); 4] = [
    (Mode::Once, "One-shot"),
    (Mode::Periodic, "Periodic"),
    (Mode::Interval, "Interval"),
    (Mode::Plan, "Plan · approve · dispatch"),
];

fn open_task_dialog(refresh: &Rc<dyn Fn()>, initial: Mode, title: &str) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title(title);
    window.set_default_size(560, 560);
    window.set_modal(true);
    let root = vbox(8);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);

    let name = labeled_entry(&root, "Name");

    // --- the four-tile mode selector -------------------------------------
    let tiles = gtk::Grid::new();
    tiles.set_column_spacing(8);
    tiles.set_row_spacing(8);
    tiles.set_column_homogeneous(true);
    let tile_buttons: Vec<gtk::ToggleButton> = MODE_TILES
        .iter()
        .enumerate()
        .map(|(index, (_, label))| {
            let button = gtk::ToggleButton::with_label(label);
            class(&button, "eb-catalog-tab");
            tiles.attach(&button, index as i32, 0, 1, 1);
            button
        })
        .collect();
    root.pack_start(&tiles, false, false, 0);
    let mode: Rc<Cell<Mode>> = Rc::new(Cell::new(initial));
    for (index, button) in tile_buttons.iter().enumerate() {
        if MODE_TILES[index].0 == initial {
            button.set_active(true);
            class(button, "eb-catalog-tab-active");
        }
        let mode = mode.clone();
        let buttons = tile_buttons.clone();
        button.connect_toggled(move |toggled| {
            if !toggled.is_active() {
                return;
            }
            mode.set(MODE_TILES[index].0);
            for (other, other_button) in buttons.iter().enumerate() {
                let active = MODE_TILES[other].0 == mode.get();
                if other_button.is_active() != active {
                    other_button.set_active(active);
                }
                if active {
                    class(other_button, "eb-catalog-tab-active");
                }
            }
        });
    }

    // --- shared time field (once / periodic / plan) ----------------------
    let time = labeled_entry(&root, "Time of day (HH:mm)");
    time.set_text("08:30");

    // --- periodic-only fields --------------------------------------------
    let period_box = vbox(4);
    let freq_box = hbox(4);
    let freq_label = gtk::Label::new(Some("Repeats"));
    class(&freq_label, "eb-skill-meta");
    freq_box.pack_start(&freq_label, false, false, 0);
    let freq = gtk::ComboBoxText::new();
    for label in ["Daily", "Weekly", "Every 2 weeks", "Monthly", "Yearly"] {
        freq.append_text(label);
    }
    freq.set_active(Some(1));
    freq_box.pack_start(&freq, true, true, 0);
    period_box.pack_start(&freq_box, false, false, 0);
    let weekday_box = hbox(4);
    let weekday_checks: Vec<gtk::CheckButton> = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"]
        .iter()
        .map(|label| {
            let check = gtk::CheckButton::with_label(label);
            weekday_box.pack_start(&check, false, false, 0);
            check
        })
        .collect();
    period_box.pack_start(&weekday_box, false, false, 0);
    let weekdays_hint = gtk::Label::new(Some(
        "Pick at least one weekday. Monthly needs day-of-month; yearly needs month and day.",
    ));
    weekdays_hint.set_xalign(0.0);
    weekdays_hint.set_line_wrap(true);
    class(&weekdays_hint, "eb-skill-meta");
    period_box.pack_start(&weekdays_hint, false, false, 0);
    let month_days = labeled_entry(&period_box, "Day of month (1-31)");
    let month_num = labeled_entry(&period_box, "Month (1-12, yearly only)");
    root.pack_start(&period_box, false, false, 0);

    // --- interval-only field ---------------------------------------------
    let hours = labeled_entry(&root, "Hours between runs (1-24)");
    hours.set_text("2");

    // --- plan-only fields -------------------------------------------------
    let deadline = labeled_entry(&root, "Approval deadline in minutes");
    deadline.set_text("45");

    // --- content: prompt for the three prompt modes, standing tasks for plan
    let content_label = gtk::Label::new(Some("Prompt"));
    class(&content_label, "eb-skill-meta");
    content_label.set_xalign(0.0);
    root.pack_start(&content_label, false, false, 0);
    let content = gtk::TextView::new();
    content.set_wrap_mode(gtk::WrapMode::Word);
    let content_scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    content_scroll.set_size_request(-1, 110);
    content_scroll.add(&content);
    root.pack_start(&content_scroll, true, true, 0);

    // --- template (plan only) --------------------------------------------
    let template_label = gtk::Label::new(Some(
        "Item prompt template ({task.title}, {task.context}, {task.done_when})",
    ));
    template_label.set_xalign(0.0);
    class(&template_label, "eb-skill-meta");
    root.pack_start(&template_label, false, false, 0);
    let template = gtk::TextView::new();
    template.set_wrap_mode(gtk::WrapMode::Word);
    if let Some(buffer) = template.buffer() {
        buffer.set_text(crate::schedule::default_template());
    }
    let template_scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    template_scroll.set_size_request(-1, 70);
    template_scroll.add(&template);
    root.pack_start(&template_scroll, false, false, 0);

    let error = gtk::Label::new(None);
    error.set_xalign(0.0);
    class(&error, "eb-skill-error");
    root.pack_start(&error, false, false, 0);

    // --- visibility sync --------------------------------------------------
    let sync = {
        let mode = mode.clone();
        let freq = freq.clone();
        let time = time.clone();
        let period_box = period_box.clone();
        let weekday_box = weekday_box.clone();
        let month_days = month_days.clone();
        let month_num = month_num.clone();
        let hours = hours.clone();
        let deadline = deadline.clone();
        let content_label = content_label.clone();
        let template_label = template_label.clone();
        let template_scroll = template_scroll.clone();
        move || {
            let mode = mode.get();
            let freq_index = freq.active().unwrap_or(1);
            fn show<W: glib::object::IsA<gtk::Widget>>(widget: &W, visible: bool) {
                widget.set_visible(visible);
            }
            show(&period_box, mode == Mode::Periodic);
            show(&hours, mode == Mode::Interval);
            show(&deadline, mode == Mode::Plan);
            show(&template_label, mode == Mode::Plan);
            show(&template_scroll, mode == Mode::Plan);
            time.set_visible(matches!(mode, Mode::Once | Mode::Periodic | Mode::Plan));
            // Inside periodic: weekdays for week/biweek, month fields for
            // month/year.
            let weekdays_visible = mode == Mode::Periodic && matches!(freq_index, 1 | 2);
            show(&weekday_box, weekdays_visible);
            show(&month_days, mode == Mode::Periodic && matches!(freq_index, 3 | 4));
            show(&month_num, mode == Mode::Periodic && freq_index == 4);
            content_label.set_text(if mode == Mode::Plan {
                "Standing tasks, one per line"
            } else {
                "Prompt"
            });
        }
    };
    sync();
    {
        let sync = sync.clone();
        freq.connect_changed(move |_| sync());
    }

    let actions = hbox(8);
    let cancel = gtk::Button::with_label("Cancel");
    let create = gtk::Button::with_label("Schedule");
    actions.pack_end(&create, false, false, 0);
    actions.pack_end(&cancel, false, false, 0);
    root.pack_start(&actions, false, false, 0);
    window.add(&root);
    let window_for_cancel = window.clone();
    cancel.connect_clicked(move |_| window_for_cancel.close());
    let window_for_create = window.clone();
    let refresh = refresh.clone();
    create.connect_clicked(move |_| {
        let name = name.text().to_string();
        if name.trim().is_empty() {
            error.set_text("Name the task.");
            return;
        }
        let mode = mode.get();
        let content = text_of(&content);
        match mode {
            Mode::Once | Mode::Periodic => {
                let Some(minutes) = schedule::parse_hhmm(&time.text()) else {
                    error.set_text("Use a time like 08:30.");
                    return;
                };
                if mode == Mode::Once {
                    schedule::add_once_at(name.trim(), content.trim(), minutes);
                } else {
                    let freq_index = freq.active().unwrap_or(1);
                    let freq = ["day", "week", "biweek", "month", "year"][freq_index as usize];
                    let byday: Vec<u8> = weekday_checks
                        .iter()
                        .enumerate()
                        .filter(|(_, check)| check.is_active())
                        .map(|(index, _)| index as u8)
                        .collect();
                    let bymonthday: Vec<u32> = month_days
                        .text()
                        .split(',')
                        .filter_map(|part| part.trim().parse::<u32>().ok())
                        .filter(|day| (1..=31).contains(day))
                        .collect();
                    if matches!(freq_index, 1 | 2) && byday.is_empty() {
                        error.set_text("Pick at least one weekday.");
                        return;
                    }
                    if freq_index == 3 && bymonthday.is_empty() {
                        error.set_text("Give a day of month, like 15.");
                        return;
                    }
                    if content.trim().is_empty() {
                        error.set_text("Write a prompt.");
                        return;
                    }
                    let bymonth = month_num.text().trim().parse::<u32>().unwrap_or(1);
                    if freq_index == 4 && !(1..=12).contains(&bymonth) {
                        error.set_text("Month must be 1-12.");
                        return;
                    }
                    let offset = schedule::local_offset_secs(schedule::unix_now());
                    let anchor_week = ((schedule::unix_now() as i64 + offset as i64)
                        .div_euclid(86_400)
                        / 7) as u64;
                    let periodic = schedule::Periodic {
                        freq: freq.into(),
                        minutes,
                        byday,
                        bymonthday,
                        bymonth,
                        anchor_week,
                    };
                    if schedule::next_periodic_run(schedule::unix_now(), &periodic, offset).is_none()
                    {
                        error.set_text("That recurrence never fires; check the fields.");
                        return;
                    }
                    schedule::add_periodic(name.trim(), content.trim(), &periodic);
                }
            }
            Mode::Interval => {
                if content.trim().is_empty() {
                    error.set_text("Write a prompt.");
                    return;
                }
                let Ok(raw) = hours.text().trim().parse::<u32>() else {
                    error.set_text("Hours must be a number.");
                    return;
                };
                if !(1..=24).contains(&raw) {
                    error.set_text("Hours must be 1-24.");
                    return;
                }
                schedule::add_interval(name.trim(), content.trim(), raw);
            }
            Mode::Plan => {
                let Some(minutes) = schedule::parse_hhmm(&time.text()) else {
                    error.set_text("Use a time like 08:30.");
                    return;
                };
                if content.trim().is_empty() {
                    error.set_text("Add at least one standing task line.");
                    return;
                }
                let suggest: Vec<String> = content
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect();
                let template_text = text_of(&template);
                let deadline_value = deadline.text().trim().parse::<u32>().unwrap_or(45);
                schedule::add_plan_full(name.trim(), minutes, &suggest, &template_text, deadline_value);
            }
        }
        TAB.with(|slot| slot.set(0));
        refresh();
        window_for_create.close();
    });
    window.show_all();
}

fn labeled_entry(container: &gtk::Box, label: &str) -> gtk::Entry {
    let label = gtk::Label::new(Some(label));
    class(&label, "eb-skill-meta");
    label.set_xalign(0.0);
    container.pack_start(&label, false, false, 0);
    let entry = gtk::Entry::new();
    container.pack_start(&entry, false, false, 0);
    entry
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

fn open_prompt_dialog(
    store: &Store,
    refresh: &Rc<dyn Fn()>,
    plan_id: &str,
    item_id: &str,
    prompt: &str,
) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Edit this item");
    window.set_default_size(520, 320);
    window.set_modal(true);
    let root = vbox(8);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);
    let hint = gtk::Label::new(Some("This edit applies to this item only."));
    hint.set_xalign(0.0);
    class(&hint, "eb-skill-meta");
    root.pack_start(&hint, false, false, 0);
    let view = gtk::TextView::new();
    view.set_wrap_mode(gtk::WrapMode::Word);
    if let Some(buffer) = view.buffer() {
        buffer.set_text(prompt);
    }
    let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scroll.set_size_request(-1, 160);
    scroll.add(&view);
    root.pack_start(&scroll, true, true, 0);
    let actions = hbox(8);
    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::with_label("Save");
    actions.pack_end(&save, false, false, 0);
    actions.pack_end(&cancel, false, false, 0);
    root.pack_start(&actions, false, false, 0);
    window.add(&root);
    let window_for_cancel = window.clone();
    cancel.connect_clicked(move |_| window_for_cancel.close());
    let window_for_save = window.clone();
    let refresh = refresh.clone();
    let store = store.clone();
    let plan_id = plan_id.to_string();
    let item_id = item_id.to_string();
    save.connect_clicked(move |_| {
        let prompt = view
            .buffer()
            .and_then(|buffer| {
                buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .map(|text| text.to_string())
            })
            .unwrap_or_default();
        if prompt.trim().is_empty() {
            return;
        }
        schedule::set_prompt(&plan_id, &item_id, &prompt);
        {
            let mut app = store.borrow_mut();
            if let Some(task) = app.tasks.iter_mut().find(|task| {
                task.plan_item_id.as_deref() == Some(item_id.as_str()) && task.dispatch_queued
            }) {
                if let Some(crate::model::Turn::User { text, .. }) =
                    task.turns.iter_mut().find(|turn| {
                        matches!(turn, crate::model::Turn::User { .. })
                    })
                {
                    *text = prompt;
                }
            }
        }
        refresh();
        window_for_save.close();
    });
    window.show_all();
}

fn deadline_line(now: u64, deadline: u64) -> String {
    if now >= deadline {
        return "Past the deadline. It will be skipped and merged into the next plan.".into();
    }
    let left = deadline - now;
    let when = if left >= 120 {
        format!("{} min", (left + 30) / 60)
    } else {
        format!("{left}s")
    };
    format!("Skipped in {when} if you do nothing. Remaining items merge into the next plan.")
}

fn status_label(plan: &PlanRun) -> &'static str {
    match plan.status.as_str() {
        "pending" => "waiting",
        "approved" => "approved",
        "partial" => "partially approved",
        "skipped" => "skipped",
        _ => "recorded",
    }
}

fn resolution_label(resolution: &str) -> String {
    match resolution {
        "timeout_skip_merge" => "Timed out. Remaining items merge into the next plan.".into(),
        "user_skip" => "Skipped. Remaining items merge into the next plan.".into(),
        "approved" => "Approved and dispatched.".into(),
        "partial" => "Some items were approved. The rest merge into the next plan.".into(),
        other => other.to_string(),
    }
}

fn item_state(item: &PlanItem) -> &'static str {
    if item.started {
        "dispatched"
    } else if item.approved {
        "queued"
    } else if item.skipped {
        "skipped"
    } else {
        "waiting"
    }
}

fn source_label(source: &str) -> &'static str {
    match source {
        "carried" => "carried over",
        "template" => "template",
        _ => "unfinished",
    }
}

fn truncate(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}

fn apple_string(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
        .replace('\r', " ")
}
