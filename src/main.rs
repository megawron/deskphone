mod engine_bridge;
mod gnome_contacts;
mod i18n;
mod models;
mod portals;
mod safety;
mod swyx_bridge;
pub mod tray;
mod ui;

use std::sync::atomic::{AtomicBool, Ordering};
use libadwaita::prelude::*;
use libadwaita as adw;

pub const APP_ID: &str = "com.martingawron.Deskphone";

static DEMO_MODE: AtomicBool = AtomicBool::new(false);

/// Returns `true` when Deskphone was launched with `--demo` or `DESKPHONE_DEMO=1`
/// for offline showcase/review without touching real user configuration or networks.
pub fn is_demo_mode() -> bool {
    DEMO_MODE.load(Ordering::Relaxed) || std::env::var_os("DESKPHONE_DEMO").is_some()
}

/// Extracts a clean dialable number or user target from `tel:`, `callto:`, or `sip:` URIs.
pub fn parse_dial_uri(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let lower = trimmed.to_ascii_lowercase();
    let body = if lower.starts_with("tel:") {
        &trimmed[4..]
    } else if lower.starts_with("callto:") {
        &trimmed[7..]
    } else if lower.starts_with("sip:") {
        &trimmed[4..]
    } else if lower.starts_with("sips:") {
        &trimmed[5..]
    } else {
        return None;
    };

    // Strip URI parameters (e.g. ";phone-context=..." or ";transport=udp") and leading slashes
    let body = body.trim_start_matches('/');
    let without_params = body.split(';').next().unwrap_or(body);
    let user_part = without_params.split('@').next().unwrap_or(without_params);

    // Lightweight percent-decoding for common sequences (%20, %2B, %23, %2A)
    let mut decoded = String::with_capacity(user_part.len());
    let bytes = user_part.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex_str) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex_str, 16) {
                    decoded.push(byte as char);
                    i += 3;
                    continue;
                }
            }
        }
        decoded.push(bytes[i] as char);
        i += 1;
    }

    let cleaned: String = decoded
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | ' ' | '*' | '#' | '.' | '_'))
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn main() {
    let raw_args: Vec<String> = std::env::args().collect();
    let demo_requested = raw_args.iter().any(|a| a == "--demo")
        || std::env::var_os("DESKPHONE_DEMO").is_some();
    if demo_requested {
        DEMO_MODE.store(true, Ordering::Relaxed);
    }
    let filtered_args: Vec<String> = raw_args.into_iter().filter(|a| a != "--demo").collect();

    init_logging();
    i18n::init_i18n();

    adw::glib::set_prgname(Some(APP_ID));
    adw::glib::set_application_name("Deskphone");

    let mut flags = adw::gio::ApplicationFlags::HANDLES_OPEN;
    if demo_requested {
        // Allow running `--demo` alongside an active production Deskphone instance
        flags |= adw::gio::ApplicationFlags::NON_UNIQUE;
    }

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        .build();

    app.connect_activate(build_ui);
    app.connect_open(|app, files, _hint| {
        build_ui(app);
        for file in files {
            let uri = file.uri();
            if let Some(number) = parse_dial_uri(&uri) {
                if let Some(win) = app.active_window().or_else(|| app.windows().into_iter().next()) {
                    win.set_visible(true);
                    win.present();
                    let _ = gtk4::prelude::WidgetExt::activate_action(
                        &win,
                        "win.prefill-number",
                        Some(&number.to_variant()),
                    );
                }
                break;
            }
        }
    });
    app.run_with_args(&filtered_args);
}

/// Logging: schreibt immer in `~/.cache/deskphone/deskphone.log` (und bei Start im Terminal
/// zusätzlich nach stdout), damit SIP-/RTP-Pakete zuverlässig protokolliert werden.
fn init_logging() {
    use std::io::{IsTerminal, Write};
    use std::sync::{Arc, Mutex};

    struct TeeWriter {
        file: Option<Arc<Mutex<std::fs::File>>>,
        to_stdout: bool,
    }

    impl Write for TeeWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if let Some(ref f) = self.file {
                if let Ok(mut guard) = f.lock() {
                    let _ = guard.write_all(buf);
                }
            }
            if self.to_stdout {
                let _ = std::io::stdout().write_all(buf);
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            if let Some(ref f) = self.file {
                if let Ok(mut guard) = f.lock() {
                    let _ = guard.flush();
                }
            }
            if self.to_stdout {
                let _ = std::io::stdout().flush();
            }
            Ok(())
        }
    }

    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new("info,deskphone_core=debug,deskphone=debug")
    });

    let file = {
        let dir = adw::glib::user_cache_dir().join("deskphone");
        std::fs::create_dir_all(&dir)
            .ok()
            .and_then(|_| std::fs::File::create(dir.join("deskphone.log")).ok())
            .map(|f| Arc::new(Mutex::new(f)))
    };

    let to_stdout = std::io::stdout().is_terminal();
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(move || TeeWriter {
            file: file.clone(),
            to_stdout,
        })
        .try_init();
}

fn build_ui(app: &adw::Application) {
    if let Some(win) = app.active_window().or_else(|| app.windows().into_iter().next()) {
        win.set_visible(true);
        win.present();
        return;
    }

    app.set_accels_for_action("win.quit", &["<Primary>q"]);

    let style_manager = adw::StyleManager::default();
    style_manager.set_color_scheme(adw::ColorScheme::Default);

    let main_window = ui::window::MainWindow::new(app);
    main_window.window.present();

    // Automated deterministic Light & Dark showcase screenshot capture (only when `--demo` + `DESKPHONE_CAPTURE_DIR`)
    if is_demo_mode() {
        if let Ok(capture_dir) = std::env::var("DESKPHONE_CAPTURE_DIR") {
            let win = main_window.window.clone();
            let app_cap = app.clone();
            let sm = style_manager.clone();
            win.set_default_size(680, 530);
            if let Some(child) = win.child() {
                child.set_can_target(false);
            }
            main_window.dial_entry.set_text("+49 30 23125 100");
            sm.set_color_scheme(adw::ColorScheme::ForceLight);

            let save_snapshot = |w: &adw::ApplicationWindow, path: &std::path::Path| {
                use gtk4::prelude::*;
                let paintable = gtk4::WidgetPaintable::new(Some(w));
                let width = paintable.intrinsic_width().max(w.width()) as f64;
                let height = paintable.intrinsic_height().max(w.height()) as f64;
                if width > 0.0 && height > 0.0 {
                    let snapshot = gtk4::Snapshot::new();
                    // Clip cleanly to Libadwaita 1.8's 15px window radius so outer corners
                    // are 100% transparent without clipped drop-shadow wedges.
                    let bounds = gtk4::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
                    let rounded = gtk4::gsk::RoundedRect::from_rect(bounds, 15.0);
                    snapshot.push_rounded_clip(&rounded);
                    paintable.snapshot(&snapshot, width, height);
                    snapshot.pop();
                    if let Some(node) = snapshot.to_node() {
                        if let Some(renderer) = w.renderer() {
                            let tex = renderer.render_texture(&node, Some(&bounds));
                            let _ = tex.save_to_png(path);
                        }
                    }
                }
            };

            let dir = std::path::PathBuf::from(capture_dir);
            let _ = std::fs::create_dir_all(&dir);
            adw::glib::timeout_add_local_once(std::time::Duration::from_millis(550), move || {
                save_snapshot(&win, &dir.join("deskphone-light.png"));
                sm.set_color_scheme(adw::ColorScheme::ForceDark);
                let win2 = win.clone();
                let dir2 = dir.clone();
                let app2 = app_cap.clone();
                adw::glib::timeout_add_local_once(std::time::Duration::from_millis(450), move || {
                    save_snapshot(&win2, &dir2.join("deskphone-dark.png"));
                    app2.quit();
                });
            });
        }
    }
}
