fn main() {
    println!("cargo:rerun-if-changed=assets/windict.rc");
    println!("cargo:rerun-if-changed=assets/windict.ico");
    println!("cargo:rerun-if-changed=assets/windict.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // Embeds the tray/exe icon and the DPI-awareness manifest.
        embed_resource::compile("assets/windict.rc", embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
