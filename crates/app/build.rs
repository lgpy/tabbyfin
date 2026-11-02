//! Embeds the app icon in Windows builds, as icon resource 1: Explorer uses it for
//! `tabbyfin.exe`, and the installer's shortcuts use the .ico file. The window's own icon comes
//! from `window::viewport` (`packaging/tabbyfin.png`); eframe doesn't read Windows icon
//! resources.

fn main() {
    // The UI tests compile a mockup file in under the `mockup` feature, named by
    // `TABBYFIN_MOCKUP_SRC` (`src/ui_tests/mod.rs`). A build with the feature and no file to
    // render (clippy, a plain `cargo test` with it on) gets an empty one instead of failing.
    println!("cargo:rerun-if-env-changed=TABBYFIN_MOCKUP_SRC");
    if std::env::var_os("TABBYFIN_MOCKUP_SRC").is_none() {
        let empty = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui_tests/no_mockup.rs");
        println!("cargo:rustc-env=TABBYFIN_MOCKUP_SRC={}", empty.display());
    }
    println!("cargo:rerun-if-changed=../../packaging/tabbyfin.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../packaging/tabbyfin.ico");
    if let Err(err) = resource.compile() {
        panic!("couldn't embed the app icon: {err}");
    }
}
