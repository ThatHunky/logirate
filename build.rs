// Embeds the icon and version info into every logirate .exe.
fn main() {
    println!("cargo:rerun-if-changed=assets/logirate.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/logirate.ico");
        res.set("ProductName", "logirate");
        res.set("FileDescription", "logirate: per-app polling rate for Logitech mice");
        res.set("CompanyName", "ThatHunky");
        res.compile().expect("compiling Windows resources (needs the Windows SDK's rc.exe)");
    }
}
