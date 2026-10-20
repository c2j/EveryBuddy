//! Embed the EvoBuddy icon into the Windows executable. Every other platform
//! carries its icon as a bundle resource (macOS .icns, Linux hicolor), so this
//! is a no-op there.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packaging/evobuddy.ico");
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(icon.to_str().expect("icon path is utf-8"));
    resource.compile().expect("failed to embed the Windows icon");
}
