fn main() {
    for file in [
        "windict.rc",
        "windict-uiaccess.rc",
        "windict-setup.rc",
        "windict.ico",
        "windict.manifest",
        "windict-uiaccess.manifest",
        "windict-setup.manifest",
    ] {
        println!("cargo:rerun-if-changed=assets/{file}");
    }
    println!("cargo:rerun-if-env-changed=WINDICT_SETUP");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // Embeds the icon and the manifest. The `uiaccess` build asks Windows
        // for UI access, which lets the card appear above Start and Search;
        // it only starts once installed by the setup and signed by
        // installer/uiaccess.ps1.
        // `wd` never asks for it.
        if std::env::var_os("CARGO_FEATURE_UIACCESS").is_some() {
            embed_resource::compile_for("assets/windict.rc", ["wd"], embed_resource::NONE)
                .manifest_optional()
                .unwrap();
            let rc = "assets/windict-uiaccess.rc";
            embed_resource::compile_for(rc, ["windict"], embed_resource::NONE)
                .manifest_optional()
                .unwrap();
        } else {
            let rc = "assets/windict.rc";
            embed_resource::compile_for(rc, ["windict", "wd"], embed_resource::NONE)
                .manifest_optional()
                .unwrap();
        }
        // The setup asks for administrator rights before its window opens.
        if std::env::var_os("CARGO_FEATURE_SETUP").is_some() {
            let rc = "assets/windict-setup.rc";
            embed_resource::compile_for(rc, ["windict-setup"], embed_resource::NONE)
                .manifest_optional()
                .unwrap();
        }
    }
}
