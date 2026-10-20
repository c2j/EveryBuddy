//! Catalog of dsh client UI plugins that the embedded panel can host.

pub const PLUGINS: &[(&str, &str)] = &[
    ("ui-theme", "Theme"),
    ("ui-message-feedback", "Message feedback"),
];

pub fn url(id: &str) -> Option<&'static str> {
    match id {
        "ui-theme" => Some(crate::preview_server::theme_url()),
        "ui-message-feedback" => Some(crate::preview_server::url()),
        _ => None,
    }
}
