//! Embeds a manifest so common controls use the current visual style (the settings window).

fn main() {
    println!("cargo:rerun-if-changed=pasteroll.manifest");
    println!("cargo:rerun-if-changed=pasteroll.rc");
    println!("cargo:rerun-if-changed=pasteroll.ico");
    // the program's icon (Explorer, taskbar, Alt+Tab, the settings window)
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let numbers = version.split('-').next().unwrap().replace('.', ",");
    let definitions = [
        format!("APP_FILE_VERSION={numbers},0"),
        format!("APP_VERSION=\"{version}\""),
        format!("APP_PUBLISHER=\"{}\"", std::env::var("CARGO_PKG_AUTHORS").unwrap()),
        format!("APP_URL=\"{}\"", std::env::var("CARGO_PKG_HOMEPAGE").unwrap()),
        format!("APP_LICENSE=\"{}\"", std::env::var("CARGO_PKG_LICENSE").unwrap()),
    ];
    embed_resource::compile("pasteroll.rc", &definitions)
        .manifest_optional()
        .expect("could not embed the icon");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default().replace('\\', "/");
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{dir}/pasteroll.manifest");
    }
}
