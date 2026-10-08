fn main() {
    for file in [
        "windict.rc",
        "windict-uiaccess.rc",
        "windict.ico",
        "windict.manifest",
        "windict-uiaccess.manifest",
    ] {
        println!("cargo:rerun-if-changed=assets/{file}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // Embeds the icon and the manifest. The `uiaccess` build asks Windows
        // for UI access, which lets the card appear above Start and Search;
        // it only starts once signed and installed by scripts/install-uiaccess.ps1.
        let rc = if std::env::var_os("CARGO_FEATURE_UIACCESS").is_some() {
            "assets/windict-uiaccess.rc"
        } else {
            "assets/windict.rc"
        };
        embed_resource::compile(rc, embed_resource::NONE)
            .manifest_optional()
            .unwrap();
    }
}
