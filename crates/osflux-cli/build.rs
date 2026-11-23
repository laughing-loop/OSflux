//! Embeds the OSflux icon in the Windows executable.
//!
//! The check is on the target, not the machine doing the build, so a build
//! for Windows from Linux gets the icon too (with mingw-w64's windres), and
//! builds for Linux and macOS skip this step.

use std::env;
use std::path::PathBuf;

fn main() {
    let icon = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by Cargo"))
        .join("../../assets/brand/icons/windows/osflux.ico");
    println!("cargo:rerun-if-changed={}", icon.display());

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(icon.to_str().expect("the icon path is valid UTF-8"));
    if let Err(error) = resource.compile() {
        panic!(
            "could not embed the Windows icon from {}: {error}. On Windows this needs the \
             Windows SDK's rc.exe, which the Visual Studio C++ build tools install; when \
             building for Windows from Linux it needs mingw-w64's windres.",
            icon.display()
        );
    }
}
