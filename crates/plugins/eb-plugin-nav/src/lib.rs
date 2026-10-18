//! The sidebar's navigation rows.
//!
//! Measured: five 255x32 buttons, the first one active. The rows are a plugin's
//! declaration, not the host's: a build that adds a screen contributes a row
//! and the sidebar grows, with no host change.

use eb_api::contrib::{self, NavRow};
use eb_api::{EbPlugin, Ui};

pub struct NavPlugin;

/// The capture's rows, in capture order. `unbuilt_route` is the name the UI
/// shows for a row whose screen has not been built (the host derives its
/// "not in this build" behaviour from the route, not from the row).
const ROWS: [NavRow; 5] = [
    NavRow { glyph: "+", label: "New Task", badge: "", unbuilt_route: "" },
    NavRow { glyph: "◆", label: "Assistant", badge: "BETA", unbuilt_route: "Assistant" },
    NavRow {
        glyph: "▦",
        label: "Experts · Skills · Connectors",
        badge: "",
        unbuilt_route: "Experts · Skills · Connectors",
    },
    NavRow { glyph: "◻", label: "Scheduled", badge: "", unbuilt_route: "Scheduled" },
    NavRow { glyph: "▤", label: "Library", badge: "", unbuilt_route: "Library" },
];

impl EbPlugin for NavPlugin {
    fn id(&self) -> &'static str {
        "nav"
    }

    fn mount(&self, ui: &Ui) -> Result<(), eb_api::CordisError> {
        for row in ROWS {
            contrib::nav_row(ui, row);
        }
        Ok(())
    }
}
