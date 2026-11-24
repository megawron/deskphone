//! GNU gettext internationalization helpers for Deskphone.
//!
//! English is the canonical source language in code; translations live in `po/*.po`
//! and are compiled into `.mo` catalogs via `build.rs` (for local Cargo runs) and
//! `po/meson.build` (for Meson & Flatpak builds).

use std::path::PathBuf;

pub const GETTEXT_PACKAGE: &str = "deskphone";

/// Resolves the best available locale directory for the current runtime environment
/// (Flatpak `/app/share/locale`, Meson-configured `DESKPHONE_LOCALEDIR`, local Cargo
/// `target/locale`, or system `/usr/share/locale`).
pub fn resolve_localedir() -> PathBuf {
    if let Some(dir) = option_env!("DESKPHONE_LOCALEDIR") {
        let p = PathBuf::from(dir);
        if p.exists() {
            return p;
        }
    }

    let flatpak_dir = PathBuf::from("/app/share/locale");
    if flatpak_dir.exists() {
        return flatpak_dir;
    }

    let dev_dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/locale"));
    if dev_dir.exists() {
        return dev_dir;
    }

    PathBuf::from("/usr/share/locale")
}

/// Initializes GNU gettext for the running process.
/// Must be called once on the main thread during startup before spawning worker threads.
pub fn init_i18n() {
    // SAFETY: Called once at process startup in `main()` before threads are spawned.
    unsafe {
        gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");
    }
    let localedir = resolve_localedir();
    let _ = gettextrs::bindtextdomain(GETTEXT_PACKAGE, &localedir);
    let _ = gettextrs::bind_textdomain_codeset(GETTEXT_PACKAGE, "UTF-8");
    let _ = gettextrs::textdomain(GETTEXT_PACKAGE);
}

/// Translates a static English UI string via gettext.
#[inline]
pub fn tr(msgid: &str) -> String {
    gettextrs::gettext(msgid)
}

/// Translates an English format template via gettext and substitutes `{key}` placeholders.
pub fn trf(msgid: &str, args: &[(&str, &str)]) -> String {
    let mut out = gettextrs::gettext(msgid);
    for (k, v) in args {
        let placeholder = format!("{{{}}}", k);
        out = out.replace(&placeholder, v);
    }
    out
}
