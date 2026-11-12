//! The TMDB key in the Secret Service (libsecret's secret-tool), not in settings.toml.
//! Read once, then kept in memory.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Mutex;

const ATTRS: [&str; 4] = ["application", "nyx-media-player", "key", "tmdb"];

static KEY: Mutex<Option<String>> = Mutex::new(None);

fn lookup() -> String {
    Command::new("secret-tool")
        .arg("lookup")
        .args(ATTRS)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn store(value: &str) -> Result<(), String> {
    let failed = || "Unlock the desktop keyring and make sure libsecret (secret-tool) is installed.".to_string();
    if value.is_empty() {
        let ok = Command::new("secret-tool").arg("clear").args(ATTRS).status().map_err(|_| failed())?.success();
        return if ok { Ok(()) } else { Err(failed()) };
    }
    let mut child = Command::new("secret-tool")
        .args(["store", "--label=Media Player TMDB key"])
        .args(ATTRS)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| failed())?;
    child.stdin.take().ok_or_else(failed)?.write_all(value.as_bytes()).map_err(|_| failed())?;
    if child.wait().map_err(|_| failed())?.success() { Ok(()) } else { Err(failed()) }
}

/// The TMDB key. A key older versions left in settings.toml moves to the keyring here.
pub fn tmdb_key() -> String {
    let mut cached = KEY.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(key) = cached.as_ref() {
        return key.clone();
    }
    let old = crate::prefs::get().tmdb_key.trim().to_string();
    let key = if !old.is_empty() && store(&old).is_ok() {
        crate::prefs::update(|p| p.tmdb_key.clear());
        old
    } else if !old.is_empty() {
        old
    } else {
        lookup()
    };
    *cached = Some(key.clone());
    key
}

/// Save the TMDB key (empty removes it).
pub fn set_tmdb_key(value: &str) -> Result<(), String> {
    store(value)?;
    *KEY.lock().unwrap_or_else(|e| e.into_inner()) = Some(value.to_string());
    Ok(())
}
