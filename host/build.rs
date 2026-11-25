//! Puts the Everywhere icon (and version info) into the Windows exe, so
//! Explorer, the taskbar and Alt+Tab show it. Nothing to do elsewhere: the
//! Mac app bundle carries its own icon (packaging/macos/make-app.sh).
fn main() {
    println!("cargo:rerun-if-changed=../assets/icons/everywhere.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../assets/icons/everywhere.ico");
        res.set("ProductName", "Everywhere");
        res.set("FileDescription", "Everywhere: use this PC from any browser");
        if let Err(e) = res.compile() {
            // A missing resource compiler only costs the icon, not the build.
            println!("cargo:warning=no exe icon: {e}");
        }
    }
}
