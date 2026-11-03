//! One-shot Mac verification for the Read-row click fix.
//!
//! GTK must be initialised on the main thread, which `cargo test` cannot
//! guarantee, so this runs as a binary: it compares the dead no-window `Box`
//! wiring with the fixed `EventBox` wiring and proves the click handler fires.
use std::rc::Rc;
use std::cell::RefCell;

use gtk::prelude::*;

fn pump_until(deadline_ms: u32, ready: impl Fn() -> bool) {
    let start = std::time::Instant::now();
    let context = gtk::glib::MainContext::default();
    while !ready() && start.elapsed().as_millis() < deadline_ms as u128 {
        context.iteration(true);
    }
}

fn main() {
    gtk::init().expect("gtk init on the main thread");

    let pressed = Rc::new(RefCell::new(Vec::<String>::new()));

    // --- The old (broken) wiring: a plain GtkBox, NO_WINDOW, handler attached.
    let old = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let hits = pressed.clone();
    old.add_events(gtk::gdk::EventMask::BUTTON_PRESS_MASK);
    old.connect_button_press_event(move |_, _| {
        hits.borrow_mut().push("old".into());
        gtk::glib::Propagation::Stop
    });

    // --- The fixed wiring: the row inside an EventBox, handler on the EventBox.
    let outer = gtk::EventBox::new();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.pack_start(&gtk::Label::new(Some("Read src/main.rs")), true, true, 0);
    outer.add(&row);
    let hits = pressed.clone();
    outer.connect_button_press_event(move |_, event| {
        if event.button() == 1 {
            hits.borrow_mut().push("new".into());
            return gtk::glib::Propagation::Stop;
        }
        gtk::glib::Propagation::Proceed
    });

    // Realise both under a toplevel, like the packed conversation does.
    let win = gtk::Window::new(gtk::WindowType::Toplevel);
    let stack = gtk::Box::new(gtk::Orientation::Vertical, 0);
    stack.pack_start(&old, false, false, 0);
    stack.pack_start(&outer, false, false, 0);
    win.add(&stack);
    win.show_all();
    pump_until(3000, || old.window().is_some() && outer.window().is_some());

    // Window ownership: the whole point of the fix. `window()` inherits the
    // parent's window on no-window widgets, so `has_window()` is the check.
    assert!(
        outer.has_window(),
        "EventBox owns a GdkWindow after realisation"
    );
    assert!(
        !old.has_window(),
        "plain GtkBox stays NO_WINDOW — events never reached the old handler"
    );

    // Deliver a button press the way GDK does to the fixed row. GTK3's
    // button-press-event signal takes the generic boxed GdkEvent; the gdk-rs
    // wrapper is read-only, so set the primary button through the FFI struct.
    let mut press = gtk::gdk::Event::new(gtk::gdk::EventType::ButtonPress);
    unsafe {
        // gdk-rs exposes the union through its GdkEventAny first member; cast
        // back to the union to reach the button variant.
        let event: &mut gtk::gdk::ffi::GdkEventAny = press.as_mut();
        let event = &mut *(event as *mut gtk::gdk::ffi::GdkEventAny
            as *mut gtk::gdk::ffi::GdkEvent);
        event.button.button = 1;
    }
    let handled: bool = outer.emit_by_name("button-press-event", &[&press]);
    assert!(handled, "the EventBox handles the button press");

    // The press-on-idle hop: the callback runs on the next main-loop pass.
    pump_until(2000, || !pressed.borrow().is_empty());
    assert_eq!(
        pressed.borrow().as_slice(),
        ["new"],
        "click routed through idle to the open callback"
    );

    // And the old wiring stays inert under the same delivery.
    let handled: bool = old.emit_by_name::<bool>("button-press-event", &[&press]);
    println!(
        "verified: EventBox owns a window and fires the click handler; \
         the plain Box had no window{}",
        if handled { " (handler attached but undeliverable)" } else { "" }
    );
}
