//! Linux CodeMirror verification switch.
//!
//! Unset, Linux stays on the text view. `EB_PREVIEW=webview` opts in for this
//! process. `EB_PREVIEW=native` forces the text view on every platform.
//! A failed opt-in sticks to the text view until the process exits; the next
//! launch with the same variable tries again.

/// `true` when this process should try the web preview.
pub(crate) fn webview_default_for(host_landed: bool, eb_preview: Option<&str>) -> bool {
    match eb_preview {
        Some("native") => false,
        Some("webview") => true,
        _ => host_landed,
    }
}

/// Environment variables copied into the helper. The bundle loader path and
/// the bundle immodule cache stay behind: a child that inherits them loads
/// the packaged GTK and exits before `main`.
pub(crate) fn forward_helper_env(key: &str) -> bool {
    matches!(
        key,
        "DISPLAY"
            | "XAUTHORITY"
            | "HOME"
            | "USER"
            | "LOGNAME"
            | "LANG"
            | "LC_ALL"
            | "LC_CTYPE"
            | "LC_MESSAGES"
            | "DBUS_SESSION_BUS_ADDRESS"
            | "XMODIFIERS"
            | "XDG_RUNTIME_DIR"
            | "GTK_MODULES"
            | "GDK_SCALE"
            | "GDK_DPI_SCALE"
    )
}

/// fcitx and ibus stay on the system XIM server. Anything else is left unset
/// so the helper does not load an immodule out of the bundle.
pub(crate) fn helper_im_module(gtk_im: Option<&str>) -> Option<&'static str> {
    match gtk_im {
        Some("fcitx" | "fcitx5" | "ibus" | "xim") => Some("xim"),
        _ => None,
    }
}

/// The opt-in accepts the target desktop's WebKitGTK 2.28 only.
/// The helper enforces this in C. The function stays for the policy tests.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn webkit_version_ok(major: u32, minor: u32, _micro: u32) -> bool {
    major == 2 && minor == 28
}

/// Map a helper `REFUSE` line to the stderr reason. The line never carries
/// the preview token.
pub(crate) fn refuse_reason(line: &str) -> String {
    if let Some(version) = line.strip_prefix("REFUSE webkit ") {
        let version = version.trim();
        format!(
            "system WebKitGTK is {version}; Linux CodeMirror verification needs 2.28. Showing the native view"
        )
    } else if line.starts_with("REFUSE webkit-missing") {
        "system WebKitGTK was not found. Showing the native view".to_string()
    } else if line.starts_with("REFUSE gtk_init") {
        "preview helper could not open an X11 display. Showing the native view".to_string()
    } else if line.starts_with("REFUSE plug-id") {
        "preview helper did not create a plug window. Showing the native view".to_string()
    } else {
        "preview helper refused to start. Showing the native view".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_webview_is_opt_in() {
        assert!(webview_default_for(false, Some("webview")));
        assert!(!webview_default_for(false, None));
        assert!(!webview_default_for(false, Some("native")));
        assert!(!webview_default_for(false, Some("1")));
        assert!(!webview_default_for(true, Some("native")));
        assert!(webview_default_for(true, None));
    }

    #[test]
    fn the_helper_does_not_inherit_the_bundle_loader_path() {
        assert!(!forward_helper_env("LD_LIBRARY_PATH"));
        assert!(!forward_helper_env("GTK_IM_MODULE_FILE"));
        assert!(!forward_helper_env("GTK_EXE_PREFIX"));
        assert!(!forward_helper_env("GDK_PIXBUF_MODULE_FILE"));
        assert!(!forward_helper_env("FONTCONFIG_FILE"));
        assert!(forward_helper_env("DISPLAY"));
        assert!(forward_helper_env("XMODIFIERS"));
        assert!(forward_helper_env("DBUS_SESSION_BUS_ADDRESS"));
        assert!(forward_helper_env("GDK_SCALE"));
        assert!(forward_helper_env("GDK_DPI_SCALE"));
        assert!(!forward_helper_env("GSETTINGS_SCHEMA_DIR"));
        assert!(!forward_helper_env("GTK_PATH"));
        assert!(!forward_helper_env("GTK_DATA_PREFIX"));
        assert!(!forward_helper_env("GIO_MODULE_DIR"));
    }

    #[test]
    fn fcitx_and_ibus_use_system_xim() {
        assert_eq!(helper_im_module(Some("fcitx")), Some("xim"));
        assert_eq!(helper_im_module(Some("fcitx5")), Some("xim"));
        assert_eq!(helper_im_module(Some("ibus")), Some("xim"));
        assert_eq!(helper_im_module(Some("xim")), Some("xim"));
        assert_eq!(helper_im_module(None), None);
        assert_eq!(helper_im_module(Some("cedilla")), None);
    }

    #[test]
    fn only_webkit_228_is_accepted() {
        assert!(webkit_version_ok(2, 28, 1));
        assert!(webkit_version_ok(2, 28, 0));
        assert!(!webkit_version_ok(2, 38, 6));
        assert!(!webkit_version_ok(2, 40, 0));
        assert!(!webkit_version_ok(1, 28, 1));
    }

    #[test]
    fn refuse_lines_do_not_need_the_token() {
        let reason = refuse_reason("REFUSE webkit 2.38.6");
        assert!(reason.contains("2.38.6"));
        assert!(reason.contains("2.28"));
        assert!(refuse_reason("REFUSE webkit-missing").contains("not found"));
        assert!(refuse_reason("REFUSE gtk_init").contains("X11"));
    }
}
