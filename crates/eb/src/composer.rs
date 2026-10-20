//! The input composer — `cr-input-container` and whatever wraps it.
//!
//! Two measured variants exist and they are *not* the same widget:
//!
//!   * **Welcome** (1160px pane): an 800x144 card, with the workspace and
//!     permission pills in a separate 36px row *below* it, over the page
//!     background.
//!   * **Conversation** (580px pane): a 532x134 card that carries the permission
//!     pill inside its own toolbar, with a 34px AI notice underneath.
//!
//! Both share the editor and the toolbar's right-hand group, so they are built
//! from the same pieces here.
//!
//! The composer is built **once** and never rebuilt on refresh: rebuilding it
//! would throw away whatever the user had typed.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;

use crate::model::Scene;
use crate::spec;
use crate::widgets::*;
use eb_api::contrib::Contribs;

/// Which dropdown was opened.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Picker {
    Add,
    Model,
    Workspace,
    Permission,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Welcome,
    Chat,
}

/// Callbacks the composer needs. Kept in a struct so adding a dropdown does not
/// change every call site.
#[derive(Clone)]
pub struct Wiring {
    pub submit: Rc<dyn Fn(String)>,
    /// Called by the send button while an answer is streaming.
    pub cancel: Rc<dyn Fn()>,
    /// Called with the picker and the chosen index.
    pub pick: Rc<dyn Fn(Picker, usize)>,
    /// Workspace menu labels: No workspace, every workspace name, Add.
    pub workspace_items: Rc<dyn Fn() -> Vec<String>>,
}

impl Wiring {
    /// Bind the picker into a plain `Fn(index)` for one dropdown.
    fn picker(&self, picker: Picker) -> Rc<dyn Fn(usize)> {
        let pick = self.pick.clone();
        Rc::new(move |index| pick(picker, index))
    }
}

struct UsageWidget {
    button: gtk::Button,
    ring: gtk::DrawingArea,
    fraction: Rc<Cell<f64>>,
    detail: gtk::Label,
}

fn usage_widget() -> UsageWidget {
    let button = flat_button("eb-context-usage");
    button.set_valign(gtk::Align::Center);
    button.set_tooltip_text(Some("Context usage"));
    hide_from_show_all(&button);
    button.set_visible(false);

    let ring = gtk::DrawingArea::new();
    ring.set_size_request(16, 16);
    ring.set_halign(gtk::Align::Center);
    ring.set_valign(gtk::Align::Center);
    let fraction = Rc::new(Cell::new(0.0f64));
    {
        let fraction = fraction.clone();
        ring.connect_draw(move |area, cr| {
            let allocation = area.allocation();
            let size = allocation.width().min(allocation.height()) as f64;
            let center = size / 2.0;
            let radius = (size / 2.0 - 2.0).max(1.0);
            let fraction = fraction.get().clamp(0.0, 1.0);

            cr.set_line_width(2.0);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.08);
            cr.arc(center, center, radius, 0.0, std::f64::consts::TAU);
            let _ = cr.stroke();

            cr.set_source_rgba(0.39, 0.40, 0.95, 1.0); // #6366f1, measured
            cr.arc(
                center,
                center,
                radius,
                -std::f64::consts::FRAC_PI_2,
                -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * fraction,
            );
            let _ = cr.stroke();
            gtk::glib::Propagation::Proceed
        });
    }
    button.add(&ring);
    ring.show();

    let popover = gtk::Popover::new(Some(&button));
    popover.set_position(gtk::PositionType::Top);
    let content = vbox(4);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.pack_start(&text("Context Usage", "eb-usage-title"), false, false, 0);
    let detail = text("", "eb-usage-detail");
    content.pack_start(&detail, false, false, 0);
    popover.add(&content);
    content.show_all();

    button.connect_clicked({
        let popover = popover.clone();
        move |_| popover.popup()
    });

    UsageWidget {
        button,
        ring,
        fraction,
        detail,
    }
}

pub struct Composer {
    /// The whole input area, ready to pack into a column.
    pub root: gtk::Box,
    /// The white card: editor plus toolbar.
    pub card: gtk::Box,
    pub toolbar: gtk::Box,
    /// The picker row. Only the Welcome variant lays it out; the Chat variant
    /// folds the same pills into its toolbar, so this is `None` there.
    pub footer: Option<gtk::Box>,
    pub view: gtk::TextView,
    /// A 32x32 ghost button that only the Code scene shows. Measured: switching
    /// Work -> Code on the landing page changes nothing else in the DOM.
    pub enhance: gtk::Button,
    send: gtk::Button,
    pristine: Rc<Cell<bool>>,
    has_text: Rc<Cell<bool>>,
    streaming: Rc<Cell<bool>>,
    workspace_label: Option<gtk::Label>,
    workspace_choice: Rc<Cell<usize>>,
    model_label: gtk::Label,
    model_choice: Rc<Cell<usize>>,
    usage: Option<UsageWidget>,
}

impl Composer {
    pub fn new(
        variant: Variant,
        model: usize,
        space_index: usize,
        permission: usize,
        wiring: Wiring,
        contribs: &Contribs,
    ) -> Composer {
        let copy = contribs.composer_copy();
        let hint = match variant {
            Variant::Welcome => copy.welcome_hint,
            Variant::Chat => copy.chat_hint,
        };
        let editor_height = match variant {
            Variant::Welcome => spec::WELCOME_EDITOR_HEIGHT,
            Variant::Chat => spec::CHAT_EDITOR_HEIGHT,
        };

        let send = flat_button("eb-send");
        send.set_valign(gtk::Align::Center);
        send.set_tooltip_text(Some("Send"));
        send.add(&glyph("↑"));
        // Measured: inactive while the input is empty.
        send.set_sensitive(false);

        let has_text = Rc::new(Cell::new(false));
        let streaming = Rc::new(Cell::new(false));
        let on_change: Rc<dyn Fn(bool)> = {
            let send = send.clone();
            let has_text = has_text.clone();
            let streaming = streaming.clone();
            Rc::new(move |value: bool| {
                has_text.set(value);
                if !streaming.get() {
                    send.set_sensitive(value);
                }
            })
        };

        let (host, view, pristine) = editor(hint, editor_height, on_change);

        let composing = Rc::new(Cell::new(false));
        {
            let composing = composing.clone();
            view.connect_preedit_changed(move |_, preedit| {
                composing.set(!preedit.is_empty());
            });
        }

        let usage = usage_widget();

        // ---- the card ------------------------------------------------------
        let card = vbox(0);
        class(&card, "eb-composer-card");
        card.pack_start(&host, false, false, 0);

        let mut workspace_label = None;
        let mut workspace_choice = Rc::new(Cell::new(space_index));
        let mut model_label = gtk::Label::new(None);
        let mut model_choice = Rc::new(Cell::new(model));
        let (toolbar, enhance) = match variant {
            Variant::Welcome => {
                let (bar, label, choice) =
                    welcome_toolbar(model, &send, &wiring, contribs);
                model_label = label;
                model_choice = choice;
                (bar, hidden_enhance())
            }
            Variant::Chat => {
                let (bar, enhance, ws_label, ws_choice, m_label, m_choice) = chat_toolbar(
                    permission,
                    model,
                    space_index,
                    &send,
                    &usage.button,
                    &wiring,
                    contribs,
                );
                workspace_label = Some(ws_label);
                workspace_choice = ws_choice;
                model_label = m_label;
                model_choice = m_choice;
                (bar, enhance)
            }
        };
        card.pack_start(&toolbar, false, false, 0);

        // ---- the picker row ------------------------------------------------
        // Only the Welcome variant lays this out, as a row *below* the card.
        // Measured: 800x36, `padding: 0 12px 4px`, 4px gap, transparent
        // background — the grey visible there is the card's own shadow.
        let footer = hbox(4);
        class(&footer, "eb-composer-footer");
        if matches!(variant, Variant::Welcome) {
            let (button, label, choice) = workspace_pill(space_index, &wiring);
            workspace_label = Some(label);
            workspace_choice = choice;
            footer.pack_start(&button, false, false, 0);
            footer.pack_start(
                &pill("◇", contribs.menu("permission"), permission, Picker::Permission, &wiring),
                false,
                false,
                0,
            );
        }

        // ---- what surrounds the card ---------------------------------------
        let root = match variant {
            Variant::Welcome => {
                let column = vbox(4);
                column.pack_start(&card, false, false, 0);
                column.pack_start(&footer, false, false, 0);
                column
            }
            Variant::Chat => {
                // Measured 580x168: a 134px card inset 24px sideways, plus a
                // 34px AI notice.
                let area = vbox(0);
                class(&area, "eb-input-area");
                area.set_size_request(-1, spec::CHAT_INPUT_AREA_HEIGHT);

                let inset = hbox(0);
                inset.set_valign(gtk::Align::Start);
                class(&inset, "eb-input-inset");
                inset.pack_start(&card, true, true, 0);
                area.pack_start(&inset, false, false, 0);

                let notice = gtk::Label::new(Some(spec::CHAT_NOTICE));
                class(&notice, "eb-input-notice");
                notice.set_xalign(0.5);
                area.pack_start(&notice, false, false, 0);

                area
            }
        };
        let visible_footer = match variant {
            Variant::Welcome => Some(footer.clone()),
            Variant::Chat => None,
        };

        // ---- submit --------------------------------------------------------
        let submit: Rc<dyn Fn()> = {
            let view = view.clone();
            let pristine = pristine.clone();
            let on_submit = wiring.submit.clone();
            Rc::new(move || {
                if pristine.get() {
                    return;
                }
                let buffer = buffer_of(&view);
                let text = buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .map(|value| value.to_string())
                    .unwrap_or_default();
                if text.trim().is_empty() {
                    return;
                }
                reset(&view, &pristine, hint);
                on_submit(text);
            })
        };

        send.connect_clicked({
            let submit = submit.clone();
            let cancel = wiring.cancel.clone();
            let streaming = streaming.clone();
            move |_| {
                if streaming.get() {
                    cancel();
                } else {
                    submit();
                }
            }
        });

        view.connect_key_press_event({
            let submit = submit.clone();
            let composing = composing.clone();
            move |view, event| {
                let enter = event.keyval() == gtk::gdk::keys::constants::Return
                    || event.keyval() == gtk::gdk::keys::constants::KP_Enter;
                // Shift+Enter falls through to the text view, which inserts a
                // newline, so multi-line prompts still work.
                if !enter || event.state().contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                    return gtk::glib::Propagation::Proceed;
                }
                // A widget's key-press handlers run *before* `GtkTextView`'s own
                // handler, and the text view is where
                // `gtk_im_context_filter_keypress` normally runs — so without
                // this, the Enter that *confirms* an IME candidate would be read
                // as "send" and a half-composed message would go out. Offer the
                // key to the input method first; it returns true only when it
                // consumed the event, and then the composition continues.
                // Only offer Enter to the IME while a preedit is active. XIM
                // can consume a bare Return even with no composition, which
                // made the second Enter look like it did nothing.
                if composing.get() && view.im_context_filter_keypress(event) {
                    return gtk::glib::Propagation::Stop;
                }
                submit();
                gtk::glib::Propagation::Stop
            }
        });

        Composer {
            root,
            card,
            toolbar,
            footer: visible_footer,
            view,
            enhance,
            send,
            pristine,
            has_text,
            streaming,
            workspace_label,
            workspace_choice,
            model_label,
            model_choice,
            usage: if matches!(variant, Variant::Chat) {
                Some(usage)
            } else {
                None
            },
        }
    }

    /// Fill the composer and put the cursor at the end — used by the scenario
    /// chips.
    pub fn fill(&self, value: &str) {
        self.pristine.set(false);
        self.view.style_context().remove_class("eb-placeholder");
        let buffer = buffer_of(&self.view);
        buffer.set_text(value);
        buffer.place_cursor(&buffer.end_iter());
        self.view.grab_focus();
    }

    /// Click the send/stop button programmatically. The fake-agent harness uses
    /// this to exercise the same path a user's click takes.
    pub fn click_send(&self) {
        self.send.clicked();
    }

    /// Update the context-usage ring. No usage event means no ring, which is
    /// the honest state for the synchronous demo agent.
    pub fn set_usage(&self, usage: Option<(u32, u32)>) {
        let Some(widget) = &self.usage else {
            return;
        };
        match usage {
            Some((used, total)) if total > 0 => {
                widget.fraction.set(used as f64 / total as f64);
                widget
                    .detail
                    .set_text(&format!("Used {used} of {total} tokens"));
                widget.ring.show();
                widget.button.show();
                widget.ring.queue_draw();
            }
            _ => widget.button.hide(),
        }
    }

    /// The Code scene adds a toolbar button the Work scene does not have.
    pub fn set_scene(&self, scene: Scene) {
        self.enhance.set_visible(matches!(scene, Scene::Code));
    }

    /// Keep the model selector in sync with the active choice across the two
    /// composer instances.
    pub fn set_model_selection(&self, index: usize) {
        self.model_choice.set(index);
        let items = crate::router::menu_labels();
        if let Some(label) = items.get(index) {
            self.model_label.set_text(label);
        }
    }

    /// Keep the picker label in sync with the active task's workspace.
    pub fn set_workspace_selection(&self, index: usize, name: &str) {
        self.workspace_choice.set(index);
        if let Some(label) = &self.workspace_label {
            label.set_text(name);
        }
    }

    /// While an answer streams, the send button becomes a stop button. The
    /// geometry stays 32x32; only the glyph, tooltip and action change.
    pub fn set_streaming(&self, streaming: bool) {
        if self.streaming.replace(streaming) == streaming {
            return;
        }
        self.send
            .set_tooltip_text(Some(if streaming { "Stop" } else { "Send" }));
        clear(&self.send);
        self.send.add(&glyph(if streaming { "■" } else { "↑" }));
        self.send.show_all();
        self.send.set_sensitive(streaming || self.has_text.get());
    }
}

// ------------------------------------------------------------------ editor

/// The contenteditable equivalent: a `GtkTextView` with a swapped-in
/// placeholder class.
///
/// Keeping a real text view matters beyond looks — it is the widget the IME
/// path from `ensure_gtk_im_modules` feeds, so CJK input works.
fn editor(
    hint: &'static str,
    height: i32,
    on_change: Rc<dyn Fn(bool)>,
) -> (gtk::ScrolledWindow, gtk::TextView, Rc<Cell<bool>>) {
    let view = gtk::TextView::new();
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_accepts_tab(false);
    class(&view, "eb-editor");

    let pristine = Rc::new(Cell::new(true));
    buffer_of(&view).set_text(hint);
    class(&view, "eb-placeholder");

    view.connect_focus_in_event({
        let pristine = pristine.clone();
        move |view, _| {
            // Only clear the hint if the user has not typed over it.
            if pristine.replace(false) {
                buffer_of(view).set_text("");
                view.style_context().remove_class("eb-placeholder");
            }
            gtk::glib::Propagation::Proceed
        }
    });

    view.connect_focus_out_event({
        let pristine = pristine.clone();
        let on_change = on_change.clone();
        move |view, _| {
            let buffer = buffer_of(view);
            let empty = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .map(|value| value.trim().is_empty())
                .unwrap_or(true);
            if empty {
                reset(view, &pristine, hint);
                on_change(false);
            }
            gtk::glib::Propagation::Proceed
        }
    });

    buffer_of(&view).connect_changed({
        let pristine = pristine.clone();
        move |buffer| {
            // While the placeholder is in the buffer there is no user text, so
            // the send button must stay inactive even though the buffer is not
            // empty.
            if pristine.get() {
                on_change(false);
                return;
            }
            let empty = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), false)
                .map(|value| value.trim().is_empty())
                .unwrap_or(true);
            on_change(!empty);
        }
    });

    let host = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    host.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    host.set_size_request(-1, height);
    class(&host, "eb-editor-host");
    host.add(&view);

    (host, view, pristine)
}

fn reset(view: &gtk::TextView, pristine: &Rc<Cell<bool>>, hint: &'static str) {
    pristine.set(true);
    class(view, "eb-placeholder");
    buffer_of(view).set_text(hint);
}

// ----------------------------------------------------------------- toolbars

/// Measured 798x56: `padding: 12px`, 8px gap, the add button on the left and the
/// model selector plus send on the right.
fn welcome_toolbar(
    model: usize,
    send: &gtk::Button,
    wiring: &Wiring,
    contribs: &Contribs,
) -> (gtk::Box, gtk::Label, Rc<Cell<usize>>) {
    let bar = hbox(8);
    class(&bar, "eb-composer-toolbar");

    let left = hbox(8);
    left.set_valign(gtk::Align::Center);
    left.pack_start(
        &glyph_button("+", "eb-tool", "Add context", contribs.menu("add"), Picker::Add, wiring),
        false,
        false,
        0,
    );
    bar.pack_start(&left, true, true, 0);

    let right = hbox(4);
    right.set_valign(gtk::Align::Center);
    let (model_button, model_label, model_choice) = model_selector(model, wiring, contribs);
    right.pack_start(&model_button, false, false, 0);
    right.pack_start(send, false, false, 0);

    bar.pack_end(&right, false, false, 0);
    (bar, model_label, model_choice)
}

/// Measured 530x56: the same padding, but the left group carries the permission
/// pill and the right group can carry the Code-only button.
fn chat_toolbar(
    permission: usize,
    model: usize,
    space_index: usize,
    send: &gtk::Button,
    usage: &gtk::Button,
    wiring: &Wiring,
    contribs: &Contribs,
) -> (
    gtk::Box,
    gtk::Button,
    gtk::Label,
    Rc<Cell<usize>>,
    gtk::Label,
    Rc<Cell<usize>>,
) {
    let bar = hbox(8);
    class(&bar, "eb-composer-toolbar");

    let left = hbox(8);
    left.set_valign(gtk::Align::Center);
    left.pack_start(
        &glyph_button("+", "eb-tool", "Add context", contribs.menu("add"), Picker::Add, wiring),
        false,
        false,
        0,
    );
    let (workspace_button, workspace_label, workspace_choice) =
        workspace_pill(space_index, wiring);
    left.pack_start(&workspace_button, false, false, 0);
    left.pack_start(
        &pill("◇", contribs.menu("permission"), permission, Picker::Permission, wiring),
        false,
        false,
        0,
    );
    bar.pack_start(&left, true, true, 0);

    let right = hbox(4);
    right.set_valign(gtk::Align::Center);
    let (model_button, model_label, model_choice) = model_selector(model, wiring, contribs);

    // Built here, shown only in the Code scene.
    let enhance = icon_button("✎", "eb-tool", "Code assist");
    hide_from_show_all(&enhance);
    attach_notice(&enhance, contribs.notice("composer.code_assist"));
    right.pack_start(&enhance, false, false, 0);

    right.pack_start(usage, false, false, 0);
    right.pack_start(&model_button, false, false, 0);
    right.pack_start(send, false, false, 0);

    bar.pack_end(&right, false, false, 0);
    (
        bar,
        enhance,
        workspace_label,
        workspace_choice,
        model_label,
        model_choice,
    )
}

/// A hidden placeholder button, so the Welcome variant can keep one
/// `Composer` shape.
fn hidden_enhance() -> gtk::Button {
    let button = icon_button("✎", "eb-tool", "Code assist");
    hide_from_show_all(&button);
    button
}

/// Measured: 74.75x32, radius 999px, 13px, with a 2px caret gap.
fn model_selector(
    model: usize,
    wiring: &Wiring,
    contribs: &Contribs,
) -> (gtk::Button, gtk::Label, Rc<Cell<usize>>) {
    // The model selector shows a caret but no leading glyph.
    let button = flat_button("eb-model");
    button.set_valign(gtk::Align::Center);
    button.set_tooltip_text(Some("Choose model"));

    let items = contribs.menu("model").to_vec();
    let choice = Rc::new(Cell::new(model));
    let label = gtk::Label::new(Some(items[model.min(items.len() - 1)]));

    let row = hbox(2);
    row.pack_start(&label, false, false, 0);
    row.pack_start(&glyph("⌄"), false, false, 0);
    button.add(&row);

    // The popover is built once, so the label is repainted here rather than on
    // the next refresh.
    let on_pick: Rc<dyn Fn(usize)> = {
        let choice = choice.clone();
        let label = label.clone();
        let items = items.clone();
        let app_pick = wiring.picker(Picker::Model);
        Rc::new(move |index| {
            choice.set(index);
            if let Some(item) = items.get(index) {
                label.set_text(item);
            }
            app_pick(index);
        })
    };

    attach_menu(&button, &items, choice.clone(), gtk::PositionType::Top, on_pick);
    (button, label, choice)
}

fn workspace_pill(
    selected: usize,
    wiring: &Wiring,
) -> (gtk::Button, gtk::Label, Rc<Cell<usize>>) {
    let button = flat_button("eb-pill");
    button.set_valign(gtk::Align::Center);
    let choice = Rc::new(Cell::new(selected));
    let initial = (wiring.workspace_items)()
        .get(selected)
        .cloned()
        .unwrap_or_else(|| "No workspace".into());
    let label = gtk::Label::new(Some(&initial));

    let row = hbox(4);
    row.pack_start(&glyph("▦"), false, false, 0);
    row.pack_start(&label, false, false, 0);
    row.pack_start(&glyph("⌄"), false, false, 0);
    button.add(&row);

    let on_pick: Rc<dyn Fn(usize)> = {
        let choice = choice.clone();
        let label = label.clone();
        let wiring = wiring.clone();
        Rc::new(move |index| {
            choice.set(index);
            if let Some(name) = (wiring.workspace_items)().get(index) {
                label.set_text(name);
            }
            (wiring.pick)(Picker::Workspace, index);
        })
    };
    attach_dynamic_menu(
        &button,
        wiring.workspace_items.clone(),
        choice.clone(),
        gtk::PositionType::Top,
        on_pick,
    );
    (button, label, choice)
}

/// Measured: 32px tall, radius 999px, `padding: 0 8px`, 4px gap, 13.33px at
/// rgba(0,0,0,.5): icon, label, chevron.
fn pill(
    icon: &str,
    items: &[&'static str],
    selected: usize,
    picker: Picker,
    wiring: &Wiring,
) -> gtk::Button {
    let button = flat_button("eb-pill");
    button.set_valign(gtk::Align::Center);

    let choice = Rc::new(Cell::new(selected));
    let label = gtk::Label::new(Some(items[selected.min(items.len() - 1)]));

    let row = hbox(4);
    row.pack_start(&glyph(icon), false, false, 0);
    row.pack_start(&label, false, false, 0);
    row.pack_start(&glyph("⌄"), false, false, 0);
    button.add(&row);

    let on_pick: Rc<dyn Fn(usize)> = {
        let choice = choice.clone();
        let label = label.clone();
        let items = items.to_vec();
        let app_pick = wiring.picker(picker);
        Rc::new(move |index| {
            choice.set(index);
            if let Some(item) = items.get(index) {
                label.set_text(item);
            }
            app_pick(index);
        })
    };

    attach_menu(&button, items, choice, gtk::PositionType::Top, on_pick);
    button
}

/// A square glyph button that opens a menu with no persistent selection: the
/// "add context" menu performs an action rather than choosing an option.
fn glyph_button(
    value: &str,
    class_name: &str,
    tooltip: &str,
    items: &[&'static str],
    picker: Picker,
    wiring: &Wiring,
) -> gtk::Button {
    let button = icon_button(value, class_name, tooltip);
    attach_menu(
        &button,
        items,
        Rc::new(Cell::new(0)),
        gtk::PositionType::Top,
        wiring.picker(picker),
    );
    button
}
