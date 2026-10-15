//! Embeds the app icon in Windows builds, as icon resource 1: GPUI gives its windows that icon,
//! so the title bar, taskbar and Alt+Tab show it (the installer's shortcuts use the .ico file).

fn main() {
    println!("cargo:rerun-if-changed=../../packaging/jelly-gpui.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../packaging/jelly-gpui.ico");
    if let Err(err) = resource.compile() {
        panic!("couldn't embed the app icon: {err}");
    }
}
