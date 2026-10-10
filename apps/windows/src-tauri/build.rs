fn main() {
    tauri_build::build();
    // The plugin loads DLLs beside the executable (or in its lib/ directory).
    // Tauri bundles resources for installers, but `tauri dev` needs this copy too.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let lib =
            std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("lib");
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
        let profile = out.ancestors().nth(3).expect("Cargo profile directory");
        let dest = profile.join("lib");
        std::fs::create_dir_all(&dest).expect("create libmpv runtime directory");
        for name in ["libmpv-2.dll", "libmpv-wrapper.dll"] {
            let source = lib.join(name);
            println!("cargo:rerun-if-changed={}", source.display());
            std::fs::copy(&source, dest.join(name)).unwrap_or_else(|_| {
                panic!("Missing {name}. Run `pnpm setup:mpv` in apps/windows before building.")
            });
        }
    }
}
