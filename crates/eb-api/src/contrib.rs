//! What a plugin can contribute to the UI, and where the host collects it.
//!
//! Contributions are *declarations*: plain data the host turns into widgets at
//! render time. A plugin's `mount` fills the registries through [`Ui`]; after
//! all plugins have mounted, the host validates what it requires and renders
//! from what it got. Nothing here mentions GTK.

use crate::Ui;

/// One sidebar navigation row. The capture's five rows are contributed by the
/// built-in nav plugin; the first one ("New Task") is special-cased by the host
/// because it is mutually exclusive with an open task.
#[derive(Clone, Copy)]
pub struct NavRow {
    pub glyph: &'static str,
    pub label: &'static str,
    pub badge: &'static str,
    /// Shown for a row whose screen has not been built. Kept explicit so the UI
    /// can say so rather than silently showing the wrong thing.
    pub unbuilt_route: &'static str,
}

/// One scenario chip on the landing page: the label on the chip and the prompt
/// clicking it drops into the composer. **Inferred behaviour** (ui-spec §10):
/// the capture shows a chip click leaving the composer untouched, which is
/// consistent with either "prefill the composer" or "start a task offline";
/// prefilling is the non-destructive reading, so that is what the clone does.
#[derive(Clone, Copy)]
pub struct ChipSpec {
    pub label: &'static str,
    pub prompt: &'static str,
}

/// Composer copy that is content, not chrome: the pill labels and the editor
/// hints. (Window chrome copy — the hero, the empty state — stays in the host.)
#[derive(Clone, Copy)]
pub struct ComposerCopy {
    pub workspace_label: &'static str,
    pub permission_label: &'static str,
    pub model_label: &'static str,
    pub welcome_hint: &'static str,
    pub chat_hint: &'static str,
}

/// Everything the built-in plugins contribute. The host reads this after the
/// mount and validates the parts it cannot render without.
#[derive(Clone, Default)]
pub struct Contribs {
    pub nav: Vec<NavRow>,
    /// "Not in this build" copy, keyed by an id the host looks up: it owns
    /// *where* a notice appears and *how* it opens; the plugin owns *what it
    /// says*.
    pub notices: Vec<(&'static str, &'static str)>,
    /// Dropdown contents, keyed by a well-known name: `workspace`,
    /// `permission`, `model`, `add`, `filter`, `account`.
    pub menus: Vec<(&'static str, Vec<&'static str>)>,
    pub composer_copy: Option<ComposerCopy>,
    pub chips: Vec<ChipSpec>,
}

impl Contribs {
    pub fn notice(&self, id: &str) -> &'static str {
        let (_, message) = self
            .notices
            .iter()
            .find(|(key, _)| *key == id)
            .unwrap_or_else(|| panic!("no plugin contributed the '{id}' notice"));
        message
    }

    pub fn menu(&self, name: &str) -> &[&'static str] {
        let (_, items) = self
            .menus
            .iter()
            .find(|(key, _)| *key == name)
            .unwrap_or_else(|| panic!("no plugin contributed the '{name}' menu"));
        items
    }

    pub fn composer_copy(&self) -> ComposerCopy {
        self.composer_copy
            .expect("no plugin contributed the composer copy")
    }
}

/// Declare one navigation row.
pub fn nav_row(ui: &Ui, row: NavRow) {
    ui.contribs.lock().unwrap().nav.push(row);
}

/// Declare a "not in this build" notice under an id the host looks up.
pub fn notice(ui: &Ui, id: &'static str, message: &'static str) {
    ui.contribs.lock().unwrap().notices.push((id, message));
}

/// Declare a dropdown's items under its well-known name.
pub fn menu(ui: &Ui, name: &'static str, items: &'static [&'static str]) {
    ui.contribs.lock().unwrap().menus.push((name, items.to_vec()));
}

/// Declare the composer's content copy. Exactly one plugin may.
pub fn composer_copy(ui: &Ui, copy: ComposerCopy) {
    let mut registries = ui.contribs.lock().unwrap();
    if registries.composer_copy.is_some() {
        panic!("two plugins contributed composer copy");
    }
    registries.composer_copy = Some(copy);
}

/// Declare one scenario chip.
pub fn chip(ui: &Ui, spec: ChipSpec) {
    ui.contribs.lock().unwrap().chips.push(spec);
}
