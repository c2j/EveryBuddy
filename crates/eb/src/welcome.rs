//! The landing page — the product's `wb-home-route`.
//!
//! Measured column, top to bottom (see ui-spec §4.3):
//!
//! ```text
//! y=127  hero h1            421x42, 30px/600, centred
//! y=185  Work | Code        163.74x36 track, radius 100, 2px padding
//! y=285  scenario chips     800x32, 8px gap, radius 100
//! y=333  composer card      800x144, radius 24, 12px top padding
//! y=481  picker row         800x36, transparent, sits *below* the card
//! y=517  236px empty block
//! y=839  AI notice          1160x16.5, centred, 12px from the bottom
//! ```

use std::rc::Rc;

use gtk::prelude::*;

use crate::composer::Composer;
use crate::model::Scene;
use crate::spec;
use crate::widgets::*;
use eb_api::contrib::Contribs;

#[derive(Clone)]
pub struct Wiring {
    /// The Work | Code segmented control.
    pub scene: Rc<dyn Fn(Scene)>,
    /// A scenario chip was clicked, by index into `spec::CHIPS`.
    pub chip: Rc<dyn Fn(usize)>,
    /// The "show sidebar" affordance shown while the sidebar is collapsed.
    pub reveal: Rc<dyn Fn()>,
}

pub struct Welcome {
    pub root: gtk::Box,
    pub topbar: gtk::Box,
    pub hero: gtk::Label,
    pub segmented: gtk::Box,
    pub chips: gtk::Box,
    /// The picker row; owned by the composer but placed in this column.
    pub footer: gtk::Box,
    pub notice: gtk::Label,
    pub segments: Vec<gtk::Button>,
    pub reveal: gtk::Button,
}

pub fn build(composer: &Rc<Composer>, wiring: &Wiring, contribs: &Contribs) -> Welcome {
    let root = vbox(0);
    class(&root, "eb-content");

    let reveal = icon_button("▶", "eb-icon", "Show sidebar");
    hide_from_show_all(&reveal);
    reveal.connect_clicked({
        let reveal = wiring.reveal.clone();
        move |_| reveal()
    });

    let topbar = hbox(0);
    class(&topbar, "eb-content-topbar");
    topbar.pack_start(&reveal, false, false, 0);
    if cfg!(target_os = "macos") {
        if let Ok(url) = std::env::var("EB_DSH_WEB_URL") {
            let dsh = icon_button("▤", "eb-icon", "Open DSH plugins");
            dsh.connect_clicked(move |_| crate::dsh_web::open(&url));
            topbar.pack_end(&dsh, false, false, 0);
        }
    }
    root.pack_start(&topbar, false, false, 0);

    let body = vbox(0);
    body.set_vexpand(true);

    let column = vbox(0);
    class(&column, "eb-column");
    column.set_size_request(spec::COLUMN_WIDTH, -1);
    column.set_halign(gtk::Align::Center);
    column.set_valign(gtk::Align::Start);
    column.set_margin_top(spec::HERO_TOP_GAP);

    // Measured 421x42 at y=127: the h1 needs a 42px min-height, because the
    // natural height of the 30px line box is 37 and everything below it would
    // sit 5px high otherwise.
    let hero = gtk::Label::new(Some(spec::HERO));
    class(&hero, "eb-hero");
    hero.set_halign(gtk::Align::Center);
    column.pack_start(&hero, false, false, 0);

    column.pack_start(&spacer(spec::HERO_TO_SEGMENTED), false, false, 0);

    let (segmented, segments) = segmented_control(wiring);
    column.pack_start(&segmented, false, false, 0);

    column.pack_start(&spacer(spec::SEGMENTED_TO_COMPOSER), false, false, 0);

    let chips = hbox(8);
    class(&chips, "eb-chips");
    for (index, chip_spec) in contribs.chips.iter().enumerate() {
        let chip = flat_button("eb-chip");
        chip.set_valign(gtk::Align::Center);
        chip.add(&gtk::Label::new(Some(chip_spec.label)));
        chip.connect_clicked({
            let chip = wiring.chip.clone();
            move |_| chip(index)
        });
        chips.pack_start(&chip, false, false, 0);
    }
    column.pack_start(&chips, false, false, 0);

    column.pack_start(&spacer(spec::HERO_TO_SEGMENTED), false, false, 0);
    column.pack_start(&composer.root, false, false, 0);
    column.pack_start(&spacer(spec::BELOW_INPUT_HEIGHT), false, false, 0);

    body.pack_start(&column, false, false, 0);

    // A small, original mascot placeholder anchored to the input area's right
    // side. It is an overlay so the measured column/hero geometry is unchanged.
    let stage = gtk::Overlay::new();
    stage.add(&body);
    if let Some(mascot) = crate::icons::image("mascot", 120) {
        mascot.set_halign(gtk::Align::End);
        mascot.set_valign(gtk::Align::Start);
        mascot.set_margin_top(220);
        mascot.set_margin_end(40);
        stage.add_overlay(&mascot);
    }
    root.pack_start(&stage, true, true, 0);

    let notice = gtk::Label::new(Some(spec::NOTICE));
    class(&notice, "eb-notice");
    notice.set_halign(gtk::Align::Center);
    root.pack_end(&notice, false, false, 0);

    // The picker row lives inside the composer's column; the handle is taken
    // here only so the smoke test can measure it in place.
    let footer = composer.footer.clone().unwrap_or_else(|| vbox(0));

    Welcome {
        root,
        topbar,
        hero,
        segmented,
        chips,
        footer,
        notice,
        segments,
        reveal,
    }
}

/// 163.74x36 track (#f2f2f2, radius 100px, 2px padding, 2px gap) holding two
/// 79x32 items.
///
/// The product slides a white thumb behind the active item; a white pill on the
/// active item itself is visually equivalent and far simpler. Captured evidence
/// for the toggle mechanism: the active item carries
/// `wb-segmented__item--active` and the other does not, so a class swap is
/// exactly what the real client does.
fn segmented_control(wiring: &Wiring) -> (gtk::Box, Vec<gtk::Button>) {
    let track = hbox(2);
    class(&track, "eb-segmented");
    track.set_halign(gtk::Align::Center);
    track.set_valign(gtk::Align::Center);

    let mut buttons = Vec::new();
    for (index, (value, label)) in [("▤", "Work"), ("▸", "Code")].iter().enumerate() {
        let button = flat_button("eb-segment");
        let row = hbox(4);
        let icon = gtk::Label::new(Some(value));
        class(&icon, "eb-segment-icon");
        row.pack_start(&icon, false, false, 0);
        row.pack_start(&gtk::Label::new(Some(label)), false, false, 0);
        button.add(&row);

        let scene = if index == 0 { Scene::Work } else { Scene::Code };
        button.connect_clicked({
            let choose = wiring.scene.clone();
            move |_| choose(scene)
        });
        buttons.push(button);
    }
    for button in &buttons {
        track.pack_start(button, false, false, 0);
    }

    (track, buttons)
}
