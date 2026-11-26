//! Compiles the Slint UI, and for Windows embeds the icon, the version
//! details and the manifest that asks for Administrator when Studio opens.
//!
//! The check is on the target, not the machine doing the build, so a build
//! for Windows from Linux gets them too (with mingw-w64's windres).

use std::env;
use std::path::PathBuf;

fn main() {
    // Fonts and images are compiled into the program, so osflux-studio.exe
    // runs on its own. Debug builds also carry the element information the
    // UI tests use to find buttons by their accessible label.
    let debug = env::var("PROFILE").as_deref() == Ok("debug");
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug);
    slint_build::compile_with_config("ui/app.slint", config).expect("the Studio UI compiles");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by Cargo"));
    let icon = manifest_dir.join("../../assets/brand/icons/windows/osflux.ico");
    let manifest = manifest_dir.join("windows/osflux-studio.manifest");
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-changed={}", manifest.display());

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(icon.to_str().expect("the icon path is valid UTF-8"));
    resource.set_manifest_file(manifest.to_str().expect("the manifest path is valid UTF-8"));
    // Shown in the file's Properties and in Task Manager. Code signing
    // (SignPath) needs the product name to be the project's and the version
    // to be the same in every file of a release, so both programs say
    // "OSflux" and take the version from Cargo.toml.
    let version = env::var("CARGO_PKG_VERSION").expect("set by Cargo");
    resource
        .set("ProductName", "OSflux")
        .set("FileDescription", "OSflux Studio")
        .set("InternalName", "osflux-studio")
        .set("OriginalFilename", "osflux-studio.exe")
        .set("CompanyName", "Joly Doh")
        .set("LegalCopyright", "Copyright (C) 2026 Joly Doh")
        .set("ProductVersion", &version)
        .set("FileVersion", &version);
    if let Err(error) = resource.compile() {
        panic!(
            "could not embed the Windows icon and manifest: {error}. On Windows this needs the \
             Windows SDK's rc.exe, which the Visual Studio C++ build tools install; when \
             building for Windows from Linux it needs mingw-w64's windres."
        );
    }
}
