//! Windows only: embed the app icon and version info (VERSIONINFO) into `lumen.exe` (shipped as
//! `LumenVideo.exe`), so it shows in Explorer, the taskbar, the Start menu and Alt-Tab. The version is
//! Lumen Video Editor's own (`VERSION` in `crates/ui-egui/src/brand.rs`).
//!
//! On every other target this does nothing (`winresource` is only a build-dependency on Windows
//! hosts). A missing resource compiler is a warning, unless `LUMEN_REQUIRE_WINRES=1` (for
//! release builds) turns it into an error.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/lumen.ico");
    println!("cargo:rerun-if-changed=../../crates/ui-egui/src/brand.rs");
    println!("cargo:rerun-if-env-changed=LUMEN_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    windows::embed_resources();
}

/// Lumen Video Editor's version (`pub const VERSION: &str = "x.y.z";` in brand.rs), or 0.0.0.
#[cfg_attr(not(windows), allow(dead_code))]
fn brand_version() -> String {
    let text = std::fs::read_to_string("../../crates/ui-egui/src/brand.rs").unwrap_or_default();
    text.lines().find_map(|l| l.trim().strip_prefix("pub const VERSION: &str = \"")?.strip_suffix("\";").map(str::to_string)).unwrap_or_else(|| "0.0.0".into())
}

/// `x.y.z` packed as VERSIONINFO wants it (MAJOR << 48 | MINOR << 32 | PATCH << 16).
#[cfg_attr(not(windows), allow(dead_code))]
fn packed(version: &str) -> u64 {
    let mut parts = version.split('.').map(|p| p.parse::<u64>().unwrap_or(0).min(0xFFFF));
    let mut next = || parts.next().unwrap_or(0);
    (next() << 48) | (next() << 32) | (next() << 16)
}

#[cfg(windows)]
mod windows {
    pub fn embed_resources() {
        let version = super::brand_version();
        let mut res = winresource::WindowsResource::new();
        res.set_version_info(winresource::VersionInfo::FILEVERSION, super::packed(&version))
            .set_version_info(winresource::VersionInfo::PRODUCTVERSION, super::packed(&version))
            .set("FileVersion", &version)
            .set("ProductVersion", &version)
            .set("Comments", "https://sifg.net");
        // The licence notices ship beside the exe (licencias\).
        res.set_icon("../../assets/app-icon/lumen.ico")
            .set("ProductName", "Lumen Video Editor")
            .set("FileDescription", "Lumen Video Editor")
            .set("CompanyName", "SifG22")
            .set("LegalCopyright", "Copyright (c) SifG22 and contributors. MIT License.")
            .set("OriginalFilename", "LumenVideo.exe")
            .set("InternalName", "LumenVideo");
        if let Err(e) = res.compile() {
            if std::env::var_os("LUMEN_REQUIRE_WINRES").is_some() {
                panic!("embedding Windows resources failed: {e}");
            }
            println!("cargo:warning=lumen.exe built without icon/version resources: {e}");
        }
    }
}

/// Cross-compiling for Windows from macOS or Linux: `winresource` is not available on this host.
#[cfg(not(windows))]
mod windows {
    pub fn embed_resources() {
        println!("cargo:warning=lumen.exe built without icon/version resources (cross-compiled from a non-Windows host)");
    }
}
