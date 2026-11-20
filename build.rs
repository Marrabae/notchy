fn main() {
    // Embed icon + info versi ke notchy.exe (butuh rc.exe dari Windows SDK)
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/notchy.ico");
        res.set("FileDescription", "notchy - dynamic notch for Windows");
        res.set("ProductName", "notchy");
        if let Err(e) = res.compile() {
            println!("cargo:warning=gagal embed icon: {e}");
        }
    }
    println!("cargo:rerun-if-changed=assets/notchy.ico");
}
