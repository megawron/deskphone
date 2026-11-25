//! Freedesktop / GNOME Desktop Portals & Secret Service integration:
//! - Encrypted credential storage via `oo7` (GNOME Keyring / Flatpak Secret Portal)
//! - Background execution & status reporting via `ashpd` (`org.freedesktop.portal.Background`)

use std::sync::{Mutex, OnceLock};

const KEYRING_APP_ID: &str = "com.martingawron.Deskphone";
const KEYRING_ITEM_LABEL: &str = "Deskphone SIP / PBX Account";

/// In-memory cache of `(server, user, password)` to avoid repeated D-Bus roundtrips
/// when `load_or_init_data` is called on background sync ticks.
fn password_cache() -> &'static Mutex<Option<(String, String, String)>> {
    static CACHE: OnceLock<Mutex<Option<(String, String, String)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn keyring_disabled() -> bool {
    cfg!(test) || std::env::var("DESKPHONE_DISABLE_KEYRING").as_deref() == Ok("1")
}

/// Runs a short async keyring operation on a dedicated lightweight Tokio runtime.
fn run_keyring_op<F, T>(fut: F) -> Option<T>
where
    F: std::future::Future<Output = Result<T, oo7::Error>>,
{
    if keyring_disabled() {
        return None;
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    rt.block_on(async move {
        match tokio::time::timeout(std::time::Duration::from_secs(5), fut).await {
            Ok(Ok(val)) => Some(val),
            Ok(Err(e)) => {
                eprintln!("[Deskphone] Keyring unavailable: {}", e);
                None
            }
            Err(_) => {
                eprintln!("[Deskphone] Keyring operation timed out");
                None
            }
        }
    })
}

/// Stores the account password encrypted in the GNOME Keyring / Flatpak Secret Portal.
/// Returns `true` if the secret was safely stored in the keyring.
pub fn store_password(server: &str, user: &str, password: &str) -> bool {
    if password.is_empty() {
        if let Ok(mut guard) = password_cache().lock() {
            *guard = Some((server.to_string(), user.to_string(), String::new()));
        }
        return false;
    }

    if let Ok(guard) = password_cache().lock() {
        if let Some((ref cs, ref cu, ref cp)) = *guard {
            if cs == server && cu == user && cp == password {
                return !keyring_disabled();
            }
        }
    }

    let srv = server.to_string();
    let usr = user.to_string();
    let pwd = password.to_string();
    let stored = run_keyring_op(async move {
        let keyring = oo7::Keyring::new().await?;
        let attrs = [
            ("app", KEYRING_APP_ID),
            ("server", srv.as_str()),
            ("user", usr.as_str()),
        ];
        keyring
            .create_item(KEYRING_ITEM_LABEL, &attrs, pwd.as_bytes(), true)
            .await?;
        Ok(())
    })
    .is_some();

    if stored {
        if let Ok(mut guard) = password_cache().lock() {
            *guard = Some((server.to_string(), user.to_string(), password.to_string()));
        }
    }
    stored
}

/// Retrieves the account password from the GNOME Keyring / Flatpak Secret Portal.
pub fn load_password(server: &str, user: &str) -> Option<String> {
    if let Ok(guard) = password_cache().lock() {
        if let Some((ref cs, ref cu, ref cp)) = *guard {
            if cs == server && cu == user && !cp.is_empty() {
                return Some(cp.clone());
            }
        }
    }

    let srv = server.to_string();
    let usr = user.to_string();
    let loaded = run_keyring_op(async move {
        let keyring = oo7::Keyring::new().await?;
        let _ = keyring.unlock().await;
        let mut items = keyring
            .search_items(&[
                ("app", KEYRING_APP_ID),
                ("server", srv.as_str()),
                ("user", usr.as_str()),
            ])
            .await?;
        if items.is_empty() {
            items = keyring.search_items(&[("app", KEYRING_APP_ID)]).await?;
        }
        if let Some(item) = items.into_iter().next() {
            if item.is_locked().await.unwrap_or(false) {
                let _ = item.unlock().await;
            }
            let secret = item.secret().await?;
            let text = String::from_utf8_lossy(secret.as_bytes()).to_string();
            Ok(Some(text))
        } else {
            Ok(None)
        }
    })
    .flatten();

    if let Some(ref pwd) = loaded {
        if let Ok(mut guard) = password_cache().lock() {
            *guard = Some((server.to_string(), user.to_string(), pwd.clone()));
        }
    }
    loaded
}

/// Requests native GNOME background execution permission via `org.freedesktop.portal.Background`
/// when running inside a Flatpak sandbox so GNOME Shell shows Deskphone in Quick Settings.
pub fn request_background_portal(runtime: &tokio::runtime::Runtime, reason: &str) {
    let reason_str = reason.to_string();
    runtime.spawn(async move {
        if !ashpd::is_sandboxed() {
            return;
        }
        let res = ashpd::desktop::background::Background::request()
            .reason(reason_str.as_str())
            .auto_start(false)
            .dbus_activatable(false)
            .send()
            .await;
        match res {
            Ok(req) => match req.response() {
                Ok(bg) => tracing::info!(
                    "XDG Background Portal: allowed={}, autostart={}",
                    bg.run_in_background(),
                    bg.auto_start()
                ),
                Err(e) => tracing::debug!("XDG Background Portal response note: {}", e),
            },
            Err(e) => tracing::debug!("XDG Background Portal unavailable: {}", e),
        }
    });
}

/// Updates the status text shown for Deskphone in GNOME Shell's Quick Settings → Background Apps.
pub fn set_background_status(runtime: &tokio::runtime::Runtime, status: &str) {
    let msg = status.to_string();
    runtime.spawn(async move {
        if !ashpd::is_sandboxed() {
            return;
        }
        if let Ok(proxy) = ashpd::desktop::background::BackgroundProxy::new().await {
            let opts = ashpd::desktop::background::SetStatusOptions::default().set_message(msg.as_str());
            if let Err(e) = proxy.set_status(opts).await {
                tracing::debug!("XDG Background Portal SetStatus note: {}", e);
            }
        }
    });
}
