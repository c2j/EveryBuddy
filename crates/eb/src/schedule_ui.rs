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
    static SEARCH: RefCell<String> = const { RefCell::new(String::new()) };
    static RECORD_FILTER: Cell<usize> = const { Cell::new(0) };
    static JOB_FILTER: Cell<usize> = const { Cell::new(0) };
    static BATCH_SELECTED: RefCell<std::collections::HashSet<String>> = {
        RefCell::new(std::collections::HashSet::new())
    };
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
    // Refresh rebuilds the page; without this every poll would stack another
    // copy of every card on top of the last one.
    clear(container);
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
    // Search box filters by title or prompt substring; the query survives
    // page rebuilds in a thread-local. The status combo narrows to active or
    // paused jobs.
    let search_row = hbox(8);
    let search = gtk::Entry::new();
    search.set_placeholder_text(Some("Search tasks…"));
    search.set_hexpand(true);
    search.set_text(&SEARCH.with(|slot| slot.borrow().clone()));
    {
        let refresh = refresh.clone();
        search.connect_changed(move |entry| {
            let query = entry.text().to_string();
            let changed = SEARCH.with(|slot| {
                let mut slot = slot.borrow_mut();
                let changed = *slot != query;
                *slot = query;
                changed
            });
            if changed {
                refresh();
            }
        });
    }
    search_row.pack_start(&search, true, true, 0);
    let status = gtk::ComboBoxText::new();
    for label in ["All", "Active", "Paused"] {
        status.append_text(label);
    }
    status.set_active(Some(JOB_FILTER.with(Cell::get) as u32));
    {
        let refresh = refresh.clone();
        status.connect_changed(move |combo| {
            JOB_FILTER.with(|slot| slot.set(combo.active().unwrap_or(0) as usize));
            refresh();
        });
    }
    search_row.pack_end(&status, false, false, 0);
    let selected_count = BATCH_SELECTED.with(|slot| slot.borrow().len());
    if selected_count > 0 {
        let delete_selected = gtk::Button::with_label(&format!("Delete selected ({selected_count})"));
        let refresh = refresh.clone();
        delete_selected.connect_clicked(move |_| {
            let selected = BATCH_SELECTED.with(|slot| slot.borrow().clone());
            let indices: Vec<usize> = schedule::rows()
                .into_iter()
                .filter(|row| selected.contains(&row.id))
                .map(|row| row.index)
                .collect();
            // Remove from the back so the earlier indices stay valid.
            for index in indices.into_iter().rev() {
                schedule::remove_at(index);
            }
            BATCH_SELECTED.with(|slot| slot.borrow_mut().clear());
            refresh();
        });
        search_row.pack_end(&delete_selected, false, false, 0);
    }
    container.pack_start(&search_row, false, false, 0);

    let query = SEARCH.with(|slot| slot.borrow().to_lowercase());
    let status_filter = JOB_FILTER.with(Cell::get);
    let all_jobs: Vec<schedule::JobRow> = schedule::rows();
    let jobs: Vec<schedule::JobRow> = all_jobs
        .iter()
        .filter(|row| {
            (match status_filter {
                1 => !row.paused,
                2 => row.paused,
                _ => true,
            }) && (query.is_empty()
                || row.title.to_lowercase().contains(&query)
                || row.prompt.to_lowercase().contains(&query))
        })
        .cloned()
        .collect();
    if jobs.is_empty() {
        let text = gtk::Label::new(Some(if query.is_empty() {
            "No scheduled tasks found."
        } else {
            "No tasks match the search."
        }));
        text.set_xalign(0.0);
        class(&text, "eb-skill-meta");
        container.pack_start(&text, false, false, 0);
    }
    // Select-all operates on the filtered view, mirroring the batch toolbar.
    if jobs.len() > 1 {
        let select_all = gtk::CheckButton::with_label("Select all");
        let all_selected = !jobs.is_empty()
            && jobs
                .iter()
                .all(|job| BATCH_SELECTED.with(|slot| slot.borrow().contains(&job.id)));
        select_all.set_active(all_selected);
        {
            let jobs = jobs.clone();
            let refresh = refresh.clone();
            select_all.connect_toggled(move |button| {
                let ids: Vec<String> =
                    jobs.iter().filter(|job| !job.id.is_empty()).map(|job| job.id.clone()).collect();
                BATCH_SELECTED.with(|slot| {
                    let mut slot = slot.borrow_mut();
                    if button.is_active() {
                        for id in &ids {
                            slot.insert(id.clone());
                        }
                    } else {
                        for id in &ids {
                            slot.remove(id);
                        }
                    }
                });
                refresh();
            });
        }
        container.pack_start(&select_all, false, false, 0);
    }
    for job in jobs {
        let card = vbox(3);
        class(&card, "eb-skill-card");
        let head = hbox(8);
        // Batch selection checkbox, keyed by the stable job id.
        if !job.id.is_empty() {
            let check = gtk::CheckButton::new();
            check.set_active(BATCH_SELECTED.with(|slot| slot.borrow().contains(&job.id)));
            {
                let job_id = job.id.clone();
                let refresh = refresh.clone();
                check.connect_toggled(move |button| {
                    BATCH_SELECTED.with(|slot| {
                        if button.is_active() {
                            slot.borrow_mut().insert(job_id.clone());
                        } else {
                            slot.borrow_mut().remove(&job_id);
                        }
                    });
                    refresh();
                });
            }
            head.pack_start(&check, false, false, 0);
        }
        let name = gtk::Label::new(Some(&job.title));
        name.set_xalign(0.0);
        class(&name, "eb-skill-title");
        head.pack_start(&name, false, false, 0);
        let when = gtk::Label::new(Some(&job.when));
        class(&when, "eb-skill-source");
        head.pack_start(&when, false, false, 0);
        let pause = gtk::Button::with_label(if job.paused { "Resume" } else { "Pause" });
        let refresh_pause = refresh.clone();
        let index = job.index;
        let paused = job.paused;
        pause.connect_clicked(move |_| {
            schedule::set_paused(index, !paused);
            refresh_pause();
        });
        head.pack_end(&pause, false, false, 0);
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
        if job.plan {
            let records = gtk::Button::with_label("Records");
            let refresh_records = refresh.clone();
            records.connect_clicked(move |_| {
                TAB.with(|slot| slot.set(2));
                refresh_records();
            });
            head.pack_end(&records, false, false, 0);
        }
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
    // The toolbar-level approval covers every pending plan at once, for the
    // morning "just approve everything" case across several plan jobs.
    if pending.len() > 1 {
        let approve_everything = gtk::Button::with_label("Approve all plans");
        let refresh = refresh.clone();
        let pump = pump.clone();
        approve_everything.connect_clicked(move |_| {
            for plan in schedule::plans() {
                if plan.status == "pending" {
                    schedule::decide(&plan.id, DecisionKind::All);
                }
            }
            pump();
            refresh();
        });
        container.pack_start(&approve_everything, false, false, 0);
    }
    if pending.is_empty() {
        let empty = gtk::Label::new(Some("No plans waiting for approval."));
        empty.set_xalign(0.0);
        class(&empty, "eb-skill-meta");
        container.pack_start(&empty, false, false, 0);
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

        // Per-card select-all: the checkbox shows how many of the card's items
        // are already picked.
        let picked = plan.items.iter().filter(|item| item.selected).count();
        let select_head = hbox(8);
        let select_all = gtk::CheckButton::with_label(&format!(
            "Select all ({picked}/{})",
            plan.items.len()
        ));
        select_all.set_active(picked == plan.items.len());
        {
            let plan_items = plan.items.clone();
            let plan_id = plan.id.clone();
            let refresh = refresh.clone();
            select_all.connect_toggled(move |button| {
                for item in &plan_items {
                    schedule::set_selected(&plan_id, &item.id, button.is_active());
                }
                refresh();
            });
        }
        select_head.pack_start(&select_all, false, false, 0);
        card.pack_start(&select_head, false, false, 0);

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

    // Expired plans stay visible with what happened to them, but carry no
    // actions: their remaining items already merged into the next plan.
    let expired: Vec<PlanRun> = schedule::plans()
        .into_iter()
        .filter(|plan| plan.resolution == "timeout_skip_merge" && !plan.archived)
        .collect();
    if !expired.is_empty() {
        let section = gtk::Label::new(Some("Expired"));
        section.set_xalign(0.0);
        class(&section, "eb-catalog-section");
        container.pack_start(&section, false, false, 0);
        for plan in expired {
            let card = vbox(3);
            class(&card, "eb-skill-card");
            let head = gtk::Label::new(Some(&format!(
                "{} · {} · expired",
                plan.title,
                schedule::format_local(plan.created_at)
            )));
            head.set_xalign(0.0);
            class(&head, "eb-skill-title");
            card.pack_start(&head, false, false, 0);
            let note = gtk::Label::new(Some(
                "Timed out without approval. Its remaining items merged into the next plan.",
            ));
            note.set_xalign(0.0);
            note.set_line_wrap(true);
            class(&note, "eb-skill-meta");
            card.pack_start(&note, false, false, 0);
            container.pack_start(&card, false, false, 0);
        }
    }
}

fn fill_records(
    container: &gtk::Box,
    store: &Store,
    refresh: &Rc<dyn Fn()>,
    open_task: &Rc<dyn Fn(usize)>,
) {
    // Status filter survives rebuilds.
    let filter_row = hbox(8);
    let filter_label = gtk::Label::new(Some("Status"));
    class(&filter_label, "eb-skill-meta");
    filter_row.pack_start(&filter_label, false, false, 0);
    let filter = gtk::ComboBoxText::new();
    for label in ["All", "Approved", "Partially approved", "Skipped", "Waiting"] {
        filter.append_text(label);
    }
    filter.set_active(Some(RECORD_FILTER.with(Cell::get) as u32));
    {
        let refresh = refresh.clone();
        filter.connect_changed(move |combo| {
            RECORD_FILTER.with(|slot| slot.set(combo.active().unwrap_or(0) as usize));
            refresh();
        });
    }
    filter_row.pack_start(&filter, false, false, 0);
    container.pack_start(&filter_row, false, false, 0);

    let active_filter = RECORD_FILTER.with(Cell::get);
    let matches = |plan: &PlanRun| match active_filter {
        1 => plan.status == "approved",
        2 => plan.status == "partial",
        3 => plan.status == "skipped",
        4 => plan.status == "pending",
        _ => true,
    };
    let mut plans: Vec<PlanRun> = schedule::plans()
        .into_iter()
        .filter(|plan| !plan.archived && matches(plan))
        .collect();
    plans.reverse();
    if plans.is_empty() {
        let empty = gtk::Label::new(Some("No plan runs yet."));
        empty.set_xalign(0.0);
        class(&empty, "eb-skill-meta");
        container.pack_start(&empty, false, false, 0);
        return;
    }

    // Group by local day with a Today / Yesterday / date header.
    let mut last_day = String::new();
    for plan in plans {
        let day = schedule::format_local_date(plan.created_at);
        if day != last_day {
            last_day = day.clone();
            let header = gtk::Label::new(Some(&day_header(&day)));
            header.set_xalign(0.0);
            class(&header, "eb-catalog-section");
            container.pack_start(&header, false, false, 0);
        }
        let card = vbox(3);
        class(&card, "eb-skill-card");
        let head = hbox(8);
        let title = gtk::Label::new(Some(&format!(
            "{} · {} · {}",
            plan.title,
            schedule::format_local(plan.created_at),
            status_label(&plan)
        )));
        title.set_xalign(0.0);
        title.set_line_wrap(true);
        class(&title, "eb-skill-title");
        head.pack_start(&title, true, true, 0);
        let archive = gtk::Button::with_label("Archive");
        {
            let refresh = refresh.clone();
            let plan_id = plan.id.clone();
            archive.connect_clicked(move |_| {
                schedule::set_archived(&plan_id, true);
                set_notice("Run archived.");
                refresh();
            });
        }
        head.pack_end(&archive, false, false, 0);
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
    // High risk: the source session already failed, or the item was carried
    // over from an earlier plan.
    if item.risk == "high" {
        let risk = gtk::Label::new(Some("⚠ high risk"));
        class(&risk, "eb-skill-error");
        row.pack_start(&risk, false, false, 0);
    }
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
        let job_id = plan.job_id.clone();
        let item_id = item.id.clone();
        let prompt = item.prompt.clone();
        let agent_route = item.agent_route.clone();
        let workspace_id = item.workspace_id.clone();
        edit.connect_clicked(move |_| {
            open_prompt_dialog(
                &store,
                &refresh,
                &plan_id,
                &job_id,
                &item_id,
                &prompt,
                agent_route.clone(),
                workspace_id.clone(),
            );
        });
        row.pack_end(&edit, false, false, 0);
    }
    row
}

fn day_header(day: &str) -> String {
    let today = schedule::format_local_date(schedule::unix_now());
    let yesterday = schedule::format_local_date(schedule::unix_now().saturating_sub(86_400));
    if day == today {
        "Today".to_string()
    } else if day == yesterday {
        "Yesterday".to_string()
    } else {
        day.to_string()
    }
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

/// A labelled entry packed into its own row box, so the pair can be shown or
/// hidden together when the mode changes.
fn labeled_entry(container: &gtk::Box, label: &str) -> (gtk::Box, gtk::Entry) {
    let row = vbox(2);
    let label = gtk::Label::new(Some(label));
    class(&label, "eb-skill-meta");
    label.set_xalign(0.0);
    row.pack_start(&label, false, false, 0);
    let entry = gtk::Entry::new();
    row.pack_start(&entry, false, false, 0);
    container.pack_start(&row, false, false, 0);
    (row, entry)
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

    let (name_row, name) = labeled_entry(&root, "Name");

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
    let (time_row, time) = labeled_entry(&root, "Time of day (HH:mm)");
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
    let (month_days_row, month_days) = labeled_entry(&period_box, "Day of month (1-31)");
    let (month_num_row, month_num) = labeled_entry(&period_box, "Month (1-12, yearly only)");
    root.pack_start(&period_box, false, false, 0);

    // --- interval-only field ---------------------------------------------
    let (hours_row, hours) = labeled_entry(&root, "Hours between runs (1-24)");
    hours.set_text("2");

    // --- plan-only fields -------------------------------------------------
    let (deadline_row, deadline) = labeled_entry(&root, "Approval deadline in minutes");
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
        let time_row = time_row.clone();
        let period_box = period_box.clone();
        let weekday_box = weekday_box.clone();
        let month_days_row = month_days_row.clone();
        let month_num_row = month_num_row.clone();
        let hours_row = hours_row.clone();
        let deadline_row = deadline_row.clone();
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
            show(&hours_row, mode == Mode::Interval);
            show(&deadline_row, mode == Mode::Plan);
            show(&template_label, mode == Mode::Plan);
            show(&template_scroll, mode == Mode::Plan);
            time_row.set_visible(matches!(mode, Mode::Once | Mode::Periodic | Mode::Plan));
            // Inside periodic: weekdays for week/biweek, month fields for
            // month/year.
            let weekdays_visible = mode == Mode::Periodic && matches!(freq_index, 1 | 2);
            show(&weekday_box, weekdays_visible);
            show(&month_days_row, mode == Mode::Periodic && matches!(freq_index, 3 | 4));
            show(&month_num_row, mode == Mode::Periodic && freq_index == 4);
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
    // `show_all` undoes the initial visibility sync, so sync again after it —
    // otherwise every mode shows the periodic and plan fields.
    window.show_all();
    sync();
}

fn open_prompt_dialog(
    store: &Store,
    refresh: &Rc<dyn Fn()>,
    plan_id: &str,
    job_id: &str,
    item_id: &str,
    prompt: &str,
    agent_route: Option<String>,
    workspace_id: Option<String>,
) {
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Edit this item");
    window.set_default_size(520, 460);
    window.set_modal(true);
    let root = vbox(8);
    root.set_margin_top(16);
    root.set_margin_bottom(16);
    root.set_margin_start(16);
    root.set_margin_end(16);
    let hint = gtk::Label::new(Some("These edits apply to this item only."));
    hint.set_xalign(0.0);
    class(&hint, "eb-skill-meta");
    root.pack_start(&hint, false, false, 0);

    let view = gtk::TextView::new();
    view.set_wrap_mode(gtk::WrapMode::Word);
    if let Some(buffer) = view.buffer() {
        buffer.set_text(prompt);
    }
    let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scroll.set_size_request(-1, 150);
    scroll.add(&view);
    root.pack_start(&scroll, false, false, 0);

    // Target: agent route and workspace, both optional (empty keeps the
    // default; "(new workspace)" lets dispatch create a fresh one).
    let (_agent_row, agent) = labeled_entry(&root, "Agent route (empty = default)");
    if let Some(route) = &agent_route {
        agent.set_text(route);
    }
    let workspace_box = hbox(4);
    let workspace_label = gtk::Label::new(Some("Workspace"));
    class(&workspace_label, "eb-skill-meta");
    workspace_box.pack_start(&workspace_label, false, false, 0);
    let workspace = gtk::ComboBoxText::new();
    let workspaces: Vec<(String, String)> = {
        let app = store.borrow();
        app.workspaces
            .iter()
            .map(|workspace| (workspace.id.clone(), workspace.name.clone()))
            .collect()
    };
    workspace.append_text("(new workspace)");
    let mut selected: Option<u32> = None;
    for (index, (id, workspace_name)) in workspaces.iter().enumerate() {
        workspace.append_text(workspace_name);
        if workspace_id.as_deref() == Some(id.as_str()) {
            selected = Some(index as u32 + 1);
        }
    }
    workspace.set_active(selected);
    workspace_box.pack_start(&workspace, true, true, 0);
    root.pack_start(&workspace_box, false, false, 0);

    let preference = gtk::CheckButton::with_label("Also save this prompt as the template default");
    root.pack_start(&preference, false, false, 0);

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
    let job_id = job_id.to_string();
    let item_id = item_id.to_string();
    save.connect_clicked(move |_| {
        let prompt = text_of(&view);
        if prompt.trim().is_empty() {
            return;
        }
        schedule::set_prompt(&plan_id, &item_id, &prompt);
        // Workspace: index 0 = create a new one (None), otherwise the
        // chosen existing workspace.
        let workspace_id = match workspace.active() {
            Some(0) | None => None,
            Some(index) => workspaces
                .get(index as usize - 1)
                .map(|(id, _)| id.clone()),
        };
        let agent_route = {
            let route = agent.text().trim().to_string();
            if route.is_empty() { None } else { Some(route) }
        };
        schedule::set_item_target(&plan_id, &item_id, agent_route, workspace_id);
        if preference.is_active() {
            schedule::update_job_template(&job_id, schedule::without_preamble(&prompt));
        }
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
                task.agent_route =
                    agent_route_propagated(&task, task.agent_route.clone());
            }
        }
        refresh();
        window_for_save.close();
    });
    window.show_all();
}

fn agent_route_propagated<T>(_: &T, route: Option<String>) -> Option<String> {
    route
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
