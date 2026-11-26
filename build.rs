fn main() {
    println!("cargo:rerun-if-changed=app.manifest");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        let manifest_path = std::path::PathBuf::from(
            std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()),
        )
        .join("app.manifest");
        println!("cargo:rustc-link-arg-bin=mcgm=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=mcgm=/MANIFESTINPUT:{}",
            manifest_path.display()
        );
    }
}
