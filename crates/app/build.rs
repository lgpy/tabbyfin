//! Embeds the app icon in Windows builds, as icon resource 1: Explorer uses it for
//! `tabbyfin.exe`, and the installer's shortcuts use the .ico file. The window's own icon comes
//! from `window::viewport` (`packaging/tabbyfin.png`); eframe doesn't read Windows icon
//! resources.

fn main() {
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
