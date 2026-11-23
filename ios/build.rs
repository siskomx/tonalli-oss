use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=web.zip");
    println!("cargo:rerun-if-changed=native");
    // The country-map extractor is Go, built as a static archive by the iPhone workflow and linked in,
    // because iPhone apps cannot load a shared library.
    let native = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default()).join("native");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios")
        && native.join("libtonalli_maps.a").is_file()
    {
        println!("cargo:rustc-link-search=native={}", native.display());
        println!("cargo:rustc-link-lib=static=tonalli_maps");
    }
    tauri_build::build();
}
