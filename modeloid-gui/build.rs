//! Puts `assets/favicon.ico` into the Windows server exe, so Explorer, the
//! taskbar and the console window show Modeloid's icon. Skipped for the
//! browser (wasm) build and other systems.

fn main() {
    println!("cargo:rerun-if-changed=assets/favicon.ico");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let server = std::env::var_os("CARGO_FEATURE_SERVER").is_some();
    if windows && server {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/favicon.ico");
        res.compile().expect("couldn't embed the app icon");
    }
}
