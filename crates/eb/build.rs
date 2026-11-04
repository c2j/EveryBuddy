//! Embed the EvoBuddy icon into the Windows executable. Every other platform
//! carries its icon as a bundle resource (macOS .icns, Linux hicolor), so this
//! is a no-op there.

fn main() {
    build_linux_helper();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let icon =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packaging/evobuddy.ico");
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(icon.to_str().expect("icon path is utf-8"));
    resource
        .compile()
        .expect("failed to embed the Windows icon");
}

fn build_linux_helper() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux") {
        return;
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = manifest.join("src/preview_helper_linux.c");
    println!("cargo:rerun-if-changed={}", src.display());
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let out = std::path::Path::new(&out_dir).join("eb-preview-helper");
    let cflags = pkg_config("--cflags");
    let libs = pkg_config("--libs");
    let status = std::process::Command::new("cc")
        .args(["-O2", "-Wall", "-Wextra"])
        .args(&cflags)
        .arg("-o")
        .arg(&out)
        .arg(&src)
        .args(&libs)
        .arg("-ldl")
        .status()
        .unwrap_or_else(|error| panic!("cc failed to start: {error}"));
    if !status.success() {
        panic!("preview helper failed to compile");
    }
}

fn pkg_config(flag: &str) -> Vec<String> {
    let output = std::process::Command::new("pkg-config")
        .args([flag, "gtk+-3.0"])
        .output()
        .unwrap_or_else(|error| panic!("pkg-config gtk+-3.0: {error}"));
    if !output.status.success() {
        panic!(
            "pkg-config gtk+-3.0 {flag} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .map(str::to_string)
        .collect()
}
