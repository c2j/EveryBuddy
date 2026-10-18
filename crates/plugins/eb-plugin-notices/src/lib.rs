//! The "not in this build" copy.
//!
//! Several controls in the capture do things this build does not do (global
//! search is its own window; mini programs, code assist and a second artifact
//! tab were never captured). The host decides *where* such an affordance
//! appears and *how* it responds — a popover that says so instead of failing
//! silently; this plugin owns *what it says*, keyed by the host's surface ids.

use eb_api::contrib;
use eb_api::{EbPlugin, Ui};

pub struct NoticesPlugin;

/// The surfaces the host can ask for, and EvoBuddy's own wording for each.
/// (The product's strings are not reproduced — ui-spec §9.)
const NOTICES: &[(&str, &str)] = &[
    (
        "sidebar.search",
        "Global search is a separate window in the product, and this build only \
         covers the landing page and a task's conversation.",
    ),
    (
        "sidebar.mini_programs",
        "Mini programs are not part of this UI-only build.",
    ),
    (
        "detail.new_tab",
        "Only the one artifact a task produced is shown in this UI-only build.",
    ),
    (
        "composer.code_assist",
        "Code assist is not part of this UI-only build.",
    ),
];

impl EbPlugin for NoticesPlugin {
    fn id(&self) -> &'static str {
        "notices"
    }

    fn mount(&self, ui: &Ui) -> Result<(), eb_api::CordisError> {
        for (id, message) in NOTICES {
            contrib::notice(ui, id, message);
        }
        Ok(())
    }
}
