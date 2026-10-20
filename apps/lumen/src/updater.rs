//! Lumen Video Editor's automatic updates (Windows). At most once a day, a few seconds after
//! start, a worker thread reads [`MANIFEST_URL`]; when it names a newer version than this build
//! ([`lumen_ui_egui::brand::VERSION`]) it asks, in the interface language, whether to download
//! and install it. On yes it downloads that architecture's installer, checks its SHA-256 and starts
//! it; the installer closes the app (the unsaved-changes prompt still runs), updates it in place and
//! reopens it.
//!
//! The manifest, attached to every GitHub release next to its installers:
//!
//! ```json
//! {"version": "0.0.2", "notes": "Novedades…",
//!  "x64": {"url": "https://…/LumenVideo-Setup-0.0.2-x64.exe", "sha256": "…"},
//!  "arm64": {"url": "https://…/LumenVideo-Setup-0.0.2-arm64.exe", "sha256": "…"}}
//! ```
//!
//! Downloading and hashing use Windows' own `curl.exe` and `certutil.exe` (by full path, never
//! through `PATH`), so no network code is linked in. Every failure is logged and ignored: an
//! update check must never get in the way. `LUMEN_VIDEO_NO_UPDATE=1` turns it off.
//!
//! Nothing piles up on disk: the manifest is read into memory (never saved), the only file kept is
//! the tiny `lumen-video-update.json` state, and the downloaded installer is removed from the temp folder
//! once that version (or a newer one) is running, or when its download or checksum fails.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lumen_ui_egui::brand::{PRODUCT_NAME, VERSION};
use lumen_ui_egui::i18n;
use serde_json::Value;

/// Where the latest version is announced.
/// GitHub serves `releases/latest/download/<file>` from the newest release, and every release
/// carries its own `latest.json`, so publishing a release is all an update needs.
pub const MANIFEST_URL: &str = "https://github.com/SifG22/lumen_video_editor/releases/latest/download/latest.json";

/// How often to look.
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// Wait after start, so the question never interrupts the window opening.
const START_DELAY: Duration = Duration::from_secs(8);

/// Start the background check (Windows release builds only; never under automation).
pub fn spawn(config_dir: Option<PathBuf>, automation: bool) {
    if !cfg!(windows) || cfg!(debug_assertions) || automation || std::env::var_os("LUMEN_VIDEO_NO_UPDATE").is_some() {
        return;
    }
    let spawned = std::thread::Builder::new().name("lumen-video-update".into()).spawn(move || {
        std::thread::sleep(START_DELAY);
        remove_old_installers(&std::env::temp_dir());
        if let Err(e) = check(config_dir.as_deref()) {
            log::info!("update check: {e}");
        }
    });
    if let Err(e) = spawned {
        log::warn!("update check not started: {e}");
    }
}

/// The saved state: when we last looked, and a version the user said no to.
fn state_path(dir: Option<&Path>) -> Option<PathBuf> {
    dir.map(|d| d.join("lumen-video-update.json"))
}

fn read_state(dir: Option<&Path>) -> Value {
    state_path(dir).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

fn write_state(dir: Option<&Path>, state: &Value) {
    if let Some(p) = state_path(dir)
        && let Err(e) = std::fs::write(&p, state.to_string())
    {
        log::info!("update state not saved to {}: {e}", p.display());
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn check(dir: Option<&Path>) -> Result<(), String> {
    let mut state = read_state(dir);
    let last = state.get("lastCheck").and_then(Value::as_u64).unwrap_or(0);
    if now_secs().saturating_sub(last) < CHECK_EVERY.as_secs() {
        return Ok(());
    }
    let skipped = state.get("skipped").and_then(Value::as_str).map(str::to_owned);
    state = serde_json::json!({"lastCheck": now_secs(), "skipped": skipped});
    write_state(dir, &state);

    let body = curl(&["--max-time", "20", MANIFEST_URL])?;
    let manifest: Value =
        serde_json::from_slice(body.strip_prefix([0xEF, 0xBB, 0xBF].as_slice()).unwrap_or(&body)).map_err(|e| format!("bad manifest: {e}"))?;
    let latest = manifest.get("version").and_then(Value::as_str).ok_or("manifest has no version")?;
    if !is_newer(latest, VERSION) || skipped.as_deref() == Some(latest) {
        return Ok(());
    }
    let arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x64" };
    let asset = manifest.get(arch).ok_or_else(|| format!("no {arch} installer in the manifest"))?;
    let url = asset.get("url").and_then(Value::as_str).filter(|u| u.starts_with("https://")).ok_or("installer url missing or not https")?;
    let sha = asset.get("sha256").and_then(Value::as_str).map(str::to_ascii_lowercase).filter(|s| s.len() == 64).ok_or("installer sha256 missing")?;
    let notes = manifest.get("notes").and_then(Value::as_str).unwrap_or("").chars().take(600).collect::<String>();

    // the question in the language the interface is drawn in
    i18n::set_current(i18n::interface_language());
    let args = [("app", PRODUCT_NAME), ("latest", latest), ("current", VERSION)];
    let mut question = i18n::fmt(i18n::t("A new version of {app} is available: {latest} (you have {current})."), &args);
    if !notes.is_empty() {
        question.push_str("\n\n");
        question.push_str(&notes);
    }
    question.push_str("\n\n");
    question.push_str(&i18n::fmt(i18n::t("Download and install it now? {app} will close to update; if you have unsaved changes it asks first."), &args));
    let answer = rfd::MessageDialog::new()
        .set_title(i18n::fmt(i18n::t("Update {app}"), &args))
        .set_description(question)
        .set_level(rfd::MessageLevel::Info)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();
    if !matches!(answer, rfd::MessageDialogResult::Yes) {
        if let Some(o) = state.as_object_mut() {
            o.insert("skipped".into(), Value::String(latest.to_string()));
        }
        write_state(dir, &state);
        return Ok(());
    }

    let file = std::env::temp_dir().join(installer_name(latest, arch));
    let path = file.to_str().ok_or("temp path is not UTF-8")?;
    if let Err(e) = curl(&["--max-time", "900", "-o", path, url]) {
        let _ = std::fs::remove_file(&file);
        return Err(e);
    }
    let got = sha256(&file).inspect_err(|_| {
        let _ = std::fs::remove_file(&file);
    })?;
    if got != sha {
        let _ = std::fs::remove_file(&file);
        return Err(format!("installer checksum mismatch: expected {sha}, got {got}"));
    }
    Command::new(&file)
        .args(["/SILENT", "/NORESTART", "/CLOSEAPPLICATIONS", "/RESTARTAPPLICATIONS"])
        .spawn()
        .map_err(|e| format!("could not start the installer: {e}"))?;
    Ok(())
}

/// The downloaded installer's file name in the temp folder.
fn installer_name(version: &str, arch: &str) -> String {
    format!("{INSTALLER_PREFIX}{}-{arch}.exe", sanitize(version))
}

const INSTALLER_PREFIX: &str = "LumenVideo-Setup-";

/// Delete the installers this updater downloaded into `temp` for this version or older ones: once
/// that version runs they are only taking space. One still running (Windows locks it) stays and
/// goes on a later start.
fn remove_old_installers(temp: &Path) {
    let Ok(entries) = std::fs::read_dir(temp) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(version) = name.to_str().and_then(installer_version) else { continue };
        if !is_newer(version, VERSION) {
            match std::fs::remove_file(entry.path()) {
                Ok(()) => log::info!("removed old installer {}", entry.path().display()),
                Err(e) => log::info!("old installer {} not removed yet: {e}", entry.path().display()),
            }
        }
    }
}

/// The version in a name made by [`installer_name`], or `None` for any other file.
fn installer_version(name: &str) -> Option<&str> {
    let rest = name.strip_prefix(INSTALLER_PREFIX)?;
    let rest = rest.strip_suffix("-x64.exe").or_else(|| rest.strip_suffix("-arm64.exe"))?;
    (!rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')).then_some(rest)
}

/// `a.b.c` newer than `current`? Missing or non-numeric parts count as 0.
fn is_newer(candidate: &str, current: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> { v.trim().trim_start_matches('v').split(['.', '-']).take(3).map(|p| p.parse().unwrap_or(0)).collect() };
    let (a, b) = (parts(candidate), parts(current));
    (0..3).map(|i| (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0))).find(|(x, y)| x != y).is_some_and(|(x, y)| x > y)
}

/// A version string safe in a file name.
fn sanitize(v: &str) -> String {
    v.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-').take(32).collect()
}

fn system32(exe: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    root.join("System32").join(exe)
}

/// Run Windows' curl (`-fsSL`, so HTTP errors fail) and return what it printed.
fn curl(args: &[&str]) -> Result<Vec<u8>, String> {
    let agent = format!("LumenVideoEditor/{VERSION}");
    let out = Command::new(system32("curl.exe")).args(["-fsSL", "-A", agent.as_str()]).args(args).output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl failed ({}): {}", out.status, String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

/// SHA-256 of `file` (lower-case hex) through `certutil -hashfile`.
fn sha256(file: &Path) -> Result<String, String> {
    let out = Command::new(system32("certutil.exe")).arg("-hashfile").arg(file).arg("SHA256").output().map_err(|e| format!("certutil: {e}"))?;
    if !out.status.success() {
        return Err(format!("certutil failed ({})", out.status));
    }
    parse_certutil(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| "no hash in certutil's output".into())
}

/// The hash line of `certutil -hashfile` (older Windows separates the bytes with spaces).
fn parse_certutil(text: &str) -> Option<String> {
    text.lines().map(|l| l.replace(' ', "").to_ascii_lowercase()).find(|l| l.len() == 64 && l.chars().all(|c| c.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.0.2", "0.0.1"));
        assert!(is_newer("0.1.0", "0.0.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(is_newer("0.0.10", "0.0.9"));
        assert!(!is_newer("0.0.1", "0.0.1"));
        assert!(!is_newer("0.0.1", "0.0.2"));
        assert!(!is_newer("nonsense", "0.0.1"));
        assert!(!is_newer("", ""));
    }

    #[test]
    fn certutil_output_parses_with_or_without_spaces() {
        let hex = "e34e8e0c16119c453cd7b11a88aeda8e9fc0cf6d389f54ca56b0dbaadb9557f6";
        let new = format!("SHA256 hash of x.exe:\r\n{hex}\r\nCertUtil: -hashfile command completed successfully.\r\n");
        assert_eq!(parse_certutil(&new).as_deref(), Some(hex));
        let spaced: String = hex.as_bytes().chunks(2).map(|c| String::from_utf8_lossy(c).to_uppercase()).collect::<Vec<_>>().join(" ");
        assert_eq!(parse_certutil(&format!("SHA256 hash:\n{spaced}\n")).as_deref(), Some(hex));
        assert_eq!(parse_certutil("CertUtil: error"), None);
    }

    #[test]
    fn only_our_installers_up_to_this_version_are_removed() {
        assert_eq!(installer_version(&installer_name("0.0.2", "x64")), Some("0.0.2"));
        assert_eq!(installer_version("LumenVideo-Setup-1.2.3-arm64.exe"), Some("1.2.3"));
        assert_eq!(installer_version("LumenVideo-Setup--x64.exe"), None);
        assert_eq!(installer_version("LumenVideo-Setup-0.0.2-x64.exe.part"), None);
        assert_eq!(installer_version("Other-Setup-0.0.2-x64.exe"), None);

        let dir = std::env::temp_dir().join(format!("lumen-video-updater-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join(installer_name("0.0.0", "x64"));
        let current = dir.join(installer_name(VERSION, "arm64"));
        let newer = dir.join(installer_name("999.0.0", "x64"));
        let other = dir.join("notes.txt");
        for f in [&old, &current, &newer, &other] {
            std::fs::write(f, b"x").unwrap();
        }
        remove_old_installers(&dir);
        assert!(!old.exists() && !current.exists(), "installed versions go");
        assert!(newer.exists() && other.exists(), "a pending update and other files stay");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn file_names_keep_only_safe_characters() {
        assert_eq!(sanitize("0.0.2"), "0.0.2");
        assert_eq!(sanitize("../../evil\\name"), "....evilname");
    }
}
