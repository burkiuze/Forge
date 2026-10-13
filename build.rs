fn main() {
    // Build identity for the self-updater (CI sets FORGE_BUILD to the workflow run number).
    let build = std::env::var("FORGE_BUILD").unwrap_or_else(|_| "0".into());
    let commit = std::env::var("FORGE_COMMIT")
        .ok()
        .or_else(|| {
            std::process::Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        })
        .unwrap_or_default();
    println!("cargo:rustc-env=FORGE_BUILD={build}");
    println!("cargo:rustc-env=FORGE_COMMIT={commit}");
    println!("cargo:rerun-if-env-changed=FORGE_BUILD");
    println!("cargo:rerun-if-env-changed=FORGE_COMMIT");

    // Embed the Forge icon into the Windows executable.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/forge.ico");
        res.set("ProductName", "Forge");
        res.set("FileDescription", "Forge — AI-native game engine");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon embed failed: {e}");
        }
    }
}
