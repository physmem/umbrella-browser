//! Embeds the app icon so Explorer, the taskbar and Alt+Tab show it.

fn main() {
    println!("cargo:rerun-if-changed=assets/umbrella.rc");
    println!("cargo:rerun-if-changed=assets/umbrella.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/umbrella.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap_or_else(|e| panic!("could not embed the app icon: {e}"));
    }
}
