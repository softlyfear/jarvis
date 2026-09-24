fn main() {
    // link to Vosk lib
    // println!("cargo:rustc-link-lib=libvosk.dll");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let lib_path = std::path::Path::new(&manifest_dir)
        .join("..\\..\\lib\\windows\\amd64");
    
    println!("cargo:rustc-link-search=native={}", lib_path.display());

    // embed the icon (no-op when the target is not Windows)
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=../../resources/icons/icon.ico");
    embed_resource::compile("app.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("failed to embed the application icon");
}
