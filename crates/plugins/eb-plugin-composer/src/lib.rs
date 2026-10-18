//! The composer's content copy: dropdown menus, pill labels, editor hints and
//! the landing page's scenario chips.
//!
//! **Placeholders.** The real dropdown lists come from the signed-in account
//! and were not captured, so these are neutral stand-ins whose only job is to
//! make the controls operable (ui-spec §10). The strings are EvoBuddy's own
//! wording — the product's strings are deliberately not reproduced.

use eb_api::contrib::{self, ChipSpec, ComposerCopy};
use eb_api::{EbPlugin, Ui};

pub struct ComposerPlugin;

const COPY: ComposerCopy = ComposerCopy {
    workspace_label: "Select Workspace",
    permission_label: "Default Permissions",
    model_label: "Auto",
    welcome_hint: "Ask anything — @ to add context, / for skills",
    chat_hint: "What can I help you with today? @ add context, / enable skills and commands",
};

const MENUS: &[(&str, &[&str])] = &[
    ("workspace", &["No workspace", "Add workspace…"]),
    ("permission", &["Default Permissions", "Auto-approve", "Read-only"]),
    ("model", &["Auto", "Fast", "Deep"]),
    ("add", &["Add file", "Add folder", "Add screenshot", "Add web page"]),
    ("filter", &["All tasks", "Running", "Finished"]),
    ("account", &["Settings", "Keyboard shortcuts", "Send feedback", "Sign out"]),
];

/// Measured chip widths were 107.5 / 82.5 / 115.2 / 68.6 / 57.7 / 90.4 — i.e.
/// content-sized, not fixed, so the labels drive them. Each chip's prompt is
/// what clicking it drops into the composer.
const CHIPS: [ChipSpec; 6] = [
    ChipSpec {
        label: "Development",
        prompt: "Help me plan and build a small desktop utility.",
    },
    ChipSpec {
        label: "Websites",
        prompt: "Draft a landing page for a new product.",
    },
    ChipSpec {
        label: "Mini Programs",
        prompt: "Sketch an automation that files my weekly notes.",
    },
    ChipSpec { label: "Agents", prompt: "Design an agent that triages my inbox." },
    ChipSpec {
        label: "Skills",
        prompt: "Write a skill that summarises a directory of files.",
    },
    ChipSpec {
        label: "Tech Docs",
        prompt: "Write documentation for the module I just changed.",
    },
];

impl EbPlugin for ComposerPlugin {
    fn id(&self) -> &'static str {
        "composer"
    }

    fn mount(&self, ui: &Ui) -> Result<(), eb_api::CordisError> {
        contrib::composer_copy(ui, COPY);
        for (name, items) in MENUS {
            contrib::menu(ui, name, items);
        }
        for chip in CHIPS {
            contrib::chip(ui, chip);
        }
        Ok(())
    }
}
