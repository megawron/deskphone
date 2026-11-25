use gtk4 as gtk;
use gtk::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use gtk::glib;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone)]
struct RingtonePlayer {
    is_ringing: Arc<AtomicBool>,
}

impl RingtonePlayer {
    fn new() -> Self {
        Self {
            is_ringing: Arc::new(AtomicBool::new(false)),
        }
    }

    fn start(&self, device_id: &str) {
        if self.is_ringing.swap(true, Ordering::SeqCst) {
            return;
        }
        let ringing = self.is_ringing.clone();
        let dev = device_id.trim().to_string();
        std::thread::spawn(move || {
            let sound_path = "/usr/share/sounds/freedesktop/stereo/phone-incoming-call.oga";
            while ringing.load(Ordering::SeqCst) {
                let played_external = if std::path::Path::new(sound_path).exists() {
                    let spawn_result = if !dev.is_empty() && dev != "default" {
                        std::process::Command::new("paplay")
                            .arg(format!("--device={}", dev))
                            .arg(sound_path)
                            .spawn()
                            .or_else(|_| {
                                std::process::Command::new("pw-play")
                                    .arg(format!("--target={}", dev))
                                    .arg(sound_path)
                                    .spawn()
                            })
                    } else {
                        std::process::Command::new("pw-play")
                            .arg(sound_path)
                            .spawn()
                            .or_else(|_| {
                                std::process::Command::new("paplay")
                                    .arg(sound_path)
                                    .spawn()
                            })
                    };

                    if let Ok(mut child) = spawn_result {
                        loop {
                            if !ringing.load(Ordering::SeqCst) {
                                let _ = child.kill();
                                let _ = child.wait();
                                return;
                            }
                            match child.try_wait() {
                                Ok(Some(_)) => break,
                                Ok(None) => std::thread::sleep(Duration::from_millis(80)),
                                Err(_) => break,
                            }
                        }
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };

                if !played_external {
                    if !deskphone_core::rtp::AudioDeviceManager::play_ringtone_burst(
                        Some(&ringing),
                        0.28,
                    ) {
                        break;
                    }
                }

                // Pause between rings (approx 1.2s) unless stopped
                for _ in 0..12 {
                    if !ringing.load(Ordering::SeqCst) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        });
    }

    fn stop(&self) {
        self.is_ringing.store(false, Ordering::SeqCst);
    }

    /// Spielt einmalig einen extrem leisen Anklopfton (10 % Lautstärke) ab, damit ein laufendes
    /// Gespräch niemals vom Klingelton übertönt wird.
    fn play_call_waiting_beep(&self, device_id: &str) {
        let dev = device_id.trim().to_string();
        std::thread::spawn(move || {
            let sound_path = "/usr/share/sounds/freedesktop/stereo/message-new-instant.oga";
            let played_external = if std::path::Path::new(sound_path).exists() {
                let spawn_result = if !dev.is_empty() && dev != "default" {
                    std::process::Command::new("paplay")
                        .arg(format!("--device={}", dev))
                        .arg("--volume=6554")
                        .arg(sound_path)
                        .spawn()
                        .or_else(|_| {
                            std::process::Command::new("pw-play")
                                .env("LC_ALL", "C")
                                .arg(format!("--target={}", dev))
                                .arg("--volume=0.10")
                                .arg(sound_path)
                                .spawn()
                        })
                } else {
                    std::process::Command::new("pw-play")
                        .env("LC_ALL", "C")
                        .arg("--volume=0.10")
                        .arg(sound_path)
                        .spawn()
                        .or_else(|_| {
                            std::process::Command::new("paplay")
                                .arg("--volume=6554")
                                .arg(sound_path)
                                .spawn()
                        })
                };
                if let Ok(mut child) = spawn_result {
                    let _ = child.wait();
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if !played_external {
                deskphone_core::rtp::AudioDeviceManager::play_call_waiting_tone(0.10);
            }
        });
    }
}

/// Formats a caller label without repeating the phone number when name and number
/// represent the same digits (e.g. `"+49 7164 14950-17"` and `"+4971641495017"`).
fn format_caller_display(name: &str, number: &str) -> String {
    let n = name.trim();
    let num = number.trim();
    if num.is_empty() || n == num {
        return n.to_string();
    }
    let n_digits = crate::gnome_contacts::normalize_phone_digits(n);
    let num_digits = crate::gnome_contacts::normalize_phone_digits(num);
    if !n_digits.is_empty() && n_digits == num_digits {
        return n.to_string();
    }
    if n.is_empty() {
        num.to_string()
    } else {
        format!("{} ({})", n, num)
    }
}

/// Löst eine angerufene Nebenstelle / Zielnummer (z. B. bei Gruppen-Signalisierung) über das
/// Telefonbuch bzw. die Kurzwahlen zum lesbaren Namen des angerufenen Kollegen auf.
pub fn resolve_called_colleague_name(
    target_num: &str,
    contacts: &[crate::models::Contact],
    speed_dials: &[crate::models::SpeedDialEntry],
) -> Option<String> {
    let norm = |s: &str| -> String {
        s.trim()
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '+')
            .collect()
    };
    let tgt_raw = target_num.trim();
    let tgt = norm(tgt_raw);
    if tgt.is_empty() {
        return None;
    }

    // 1. Exakter Treffer im Telefonbuch (Durchwahl oder Rufnummer)
    if let Some(c) = contacts.iter().find(|c| {
        let ext = norm(&c.extension);
        let phone = norm(&c.phone);
        (!ext.is_empty() && ext == tgt) || (!phone.is_empty() && phone == tgt)
    }) {
        let name = crate::ui::sidebar::format_hotkey_display_name(&c.name);
        if !name.trim().is_empty() {
            return Some(name);
        }
    }

    // 2. Durchwahl-Suffix-Treffer (z. B. Zielnummer mit Ortsvorwahl endet auf 3+ stellige Durchwahl)
    if tgt.len() >= 3 {
        if let Some(c) = contacts.iter().find(|c| {
            let ext = norm(&c.extension);
            let phone = norm(&c.phone);
            (!ext.is_empty() && ext.len() >= 3 && tgt.ends_with(&ext))
                || (!phone.is_empty() && phone.len() > tgt.len() && phone.ends_with(&tgt))
        }) {
            let name = crate::ui::sidebar::format_hotkey_display_name(&c.name);
            if !name.trim().is_empty() {
                return Some(name);
            }
        }
    }

    // 3. Fallback über Kurzwahlen / Favoriten
    if let Some(sd) = speed_dials.iter().find(|sd| {
        let num = norm(&sd.number);
        !num.is_empty() && (num == tgt || (num.len() >= 3 && tgt.ends_with(&num)))
    }) {
        let name = crate::ui::sidebar::format_hotkey_display_name(&sd.name);
        if !name.trim().is_empty() && name != tgt_raw {
            return Some(name);
        }
    }

    None
}

use crate::engine_bridge::{EngineBridge, SoftphoneEvent};
use crate::models::{
    ActiveCall, AudioMode, HeldCall, PresenceState, TelephonyMode,
};
use crate::ui::chat_window::{open_chat_window, ChatWindowHandle};
use crate::ui::contacts_view::create_contacts_view;
use crate::ui::dialer_view::create_dialer_view;
use crate::ui::incall_view::create_incall_view;
use crate::ui::journal_view::create_journal_view;
use crate::ui::sidebar::create_sidebar;

pub struct MainWindow {
    pub window: adw::ApplicationWindow,
    pub dial_entry: gtk::SearchEntry,
    /// Shows the in-call page with sample data (used for `--demo` screenshots).
    pub show_demo_call: Rc<dyn Fn()>,
}

impl MainWindow {
    pub fn new(app: &adw::Application) -> Self {
        // Load custom CSS
        let css_provider = gtk::CssProvider::new();
        css_provider.load_from_data(include_str!("../style.css"));
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &css_provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            let icon_theme = gtk::IconTheme::for_display(&display);
            let manifest_icons = concat!(env!("CARGO_MANIFEST_DIR"), "/data/icons");
            icon_theme.add_search_path(manifest_icons);
            icon_theme.add_search_path("data/icons");
            icon_theme.add_search_path(adw::glib::user_data_dir().join("icons"));
        }

        // Data store & SwyxBridge (in `--demo` mode, load purely in-memory fictional showcase data)
        let is_demo = crate::is_demo_mode();
        let bridge = Rc::new(crate::swyx_bridge::SwyxBridge::new());
        let store = if is_demo {
            crate::swyx_bridge::build_demo_store()
        } else {
            bridge.load_or_init_data()
        };
        let is_unconfigured =
            !is_demo && (store.server_ip.trim().is_empty() || store.user_name.trim().is_empty());
        let ringtone_player = RingtonePlayer::new();

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Deskphone")
            .icon_name(crate::APP_ID)
            .default_width(680)
            .default_height(530)
            .width_request(300)
            .height_request(520)
            .build();

        // Application State
        let current_audio = Rc::new(RefCell::new(AudioMode::Headset));
        let current_presence = Rc::new(RefCell::new(PresenceState::Available));
        let journal_entries = Rc::new(RefCell::new(store.journal.clone()));

        // Kontakte & Kurzwahlen bereinigen (Duplikate in Kurzwahlen entfernen, Live-Präsenz nicht mit veraltetem sd.state überschreiben)
        let mut initial_contacts = store.contacts.clone();
        let mut seen_emp_ids = std::collections::HashSet::new();
        initial_contacts.retain(|c| {
            if c.id.starts_with("swyx-") {
                seen_emp_ids.insert(c.id.clone())
            } else {
                true
            }
        });
        // Bereits zwischengespeicherte Profilbilder direkt vor dem ersten Rendern aus dem Disk-Cache laden
        for c in &initial_contacts {
            crate::ui::preload_cached_contact_photo(c);
        }
        let contacts_list = Rc::new(RefCell::new(initial_contacts));

        let mut dedup_speed_dials: Vec<crate::models::SpeedDialEntry> = Vec::new();
        for sd in &store.speed_dials {
            let already = dedup_speed_dials.iter().any(|existing| {
                (!sd.number.is_empty() && existing.number == sd.number)
                    || (sd.entity_id > 0 && existing.entity_id == sd.entity_id)
            });
            if !already {
                dedup_speed_dials.push(sd.clone());
            }
        }
        if dedup_speed_dials.len() != store.speed_dials.len() {
            let mut st_clean = store.clone();
            st_clean.speed_dials = dedup_speed_dials.clone();
            bridge.save_data(&st_clean);
        }
        let speed_dials = Rc::new(RefCell::new(dedup_speed_dials));
        let current_telephony_mode = Rc::new(RefCell::new(TelephonyMode::Softphone));

        let active_call: Rc<RefCell<Option<ActiveCall>>> = Rc::new(RefCell::new(None));
        let held_call: Rc<RefCell<Option<HeldCall>>> = Rc::new(RefCell::new(None));

        // Toast Overlay for sleek Adwaita notifications (available across whole window)
        let toast_overlay = adw::ToastOverlay::new();

        // 1. Navigation Split View (Konzept A: Adwaita SplitView, schlanke Seitenleiste 215-275px)
        let split_view = adw::NavigationSplitView::new();
        split_view.set_min_sidebar_width(215.0);
        split_view.set_max_sidebar_width(275.0);
        split_view.set_sidebar_width_fraction(0.40);

        // 2. Right Content Area: AdwToolbarView (flache HeaderBar mit automatischem Scroll-Schatten)
        let content_toolbar = adw::ToolbarView::new();
        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content_toolbar.set_content(Some(&content_box));
        let content_nav_page = adw::NavigationPage::builder()
            .child(&content_toolbar)
            .title(&crate::i18n::tr("Call Journal"))
            .build();

        let content_header = adw::HeaderBar::new();
        content_header.add_css_class("content-header");
        content_header.set_centering_policy(adw::CenteringPolicy::Loose);

        // Search button at start of content header
        let search_button = gtk::ToggleButton::new();
        search_button.set_icon_name("edit-find-symbolic");
        search_button.add_css_class("flat");
        search_button.set_tooltip_text(Some(&crate::i18n::tr("Search (Ctrl+F)")));
        search_button.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr("Search"))]);
        search_button.set_visible(true);
        content_header.pack_start(&search_button);

        let window_title = adw::WindowTitle::new(&crate::i18n::tr("Call Journal"), "");
        content_header.set_title_widget(Some(&window_title));

        let conn_spinner = gtk::Spinner::new();
        conn_spinner.set_tooltip_text(Some(&crate::i18n::tr("Connecting to phone server…")));
        conn_spinner.start();
        conn_spinner.set_visible(true);

        let passive_badge = gtk::Label::new(Some(&crate::i18n::tr("Read-only")));
        passive_badge.add_css_class("sidebar-badge");
        passive_badge.set_visible(false);

        // GNOME HIG Primary Menu
        let menu = gtk::gio::Menu::new();
        menu.append(Some(&crate::i18n::tr("Preferences")), Some("win.preferences"));
        menu.append(Some(&crate::i18n::tr("Keyboard Shortcuts")), Some("win.shortcuts"));
        menu.append(Some(&crate::i18n::tr("About Deskphone")), Some("win.about"));
        menu.append(Some(&crate::i18n::tr("Quit")), Some("win.quit"));

        let primary_menu_btn = gtk::MenuButton::new();
        primary_menu_btn.set_icon_name("open-menu-symbolic");
        primary_menu_btn.set_menu_model(Some(&menu));
        primary_menu_btn.set_primary(true); // GNOME HIG: automatically binds F10 to open the primary menu
        primary_menu_btn.add_css_class("flat");
        primary_menu_btn.set_tooltip_text(Some(&crate::i18n::tr("Main Menu")));
        primary_menu_btn.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr("Main Menu"))]);

        content_header.pack_end(&primary_menu_btn);
        content_toolbar.add_top_bar(&content_header);

        // 2a. Connection / Onboarding Banner (AdwBanner)
        let conn_banner = adw::Banner::builder()
            .title(&if is_unconfigured {
                crate::i18n::tr("Set up your SIP or PBX account to start making calls")
            } else {
                crate::i18n::tr("Connecting to phone system…")
            })
            .revealed(!is_demo)
            .build();
        if is_unconfigured {
            conn_banner.set_button_label(Some(&crate::i18n::tr("Preferences")));
            conn_spinner.stop();
            conn_spinner.set_visible(false);
        } else if is_demo {
            conn_spinner.stop();
            conn_spinner.set_visible(false);
        } else {
            conn_banner.add_css_class("conn-banner-connecting");
        }
        let win_banner_pref = window.clone();
        conn_banner.connect_button_clicked(move |_| {
            let _ = gtk::prelude::WidgetExt::activate_action(&win_banner_pref, "win.preferences", None);
        });
        content_box.append(&conn_banner);

        // 2b. In-App Call Signalisierungs-Banner (minimalistisches Libadwaita-Layout ohne Überlauf)
        let call_banner = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        call_banner.add_css_class("incoming-call-banner");
        call_banner.set_visible(false);

        let banner_avatar = adw::Avatar::new(34, Some("?"), true);
        banner_avatar.set_valign(gtk::Align::Center);
        call_banner.append(&banner_avatar);

        let banner_info_box = gtk::Box::new(gtk::Orientation::Vertical, 1);
        banner_info_box.set_hexpand(true);
        banner_info_box.set_valign(gtk::Align::Center);

        let banner_name = gtk::Label::new(None);
        banner_name.add_css_class("heading");
        banner_name.set_halign(gtk::Align::Start);
        banner_name.set_hexpand(true);
        banner_name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        banner_info_box.append(&banner_name);

        let banner_details = gtk::Label::new(None);
        banner_details.add_css_class("caption");
        banner_details.add_css_class("dim-label");
        banner_details.set_halign(gtk::Align::Start);
        banner_details.set_hexpand(true);
        banner_details.set_ellipsize(gtk::pango::EllipsizeMode::End);
        banner_info_box.append(&banner_details);

        call_banner.append(&banner_info_box);

        let banner_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        banner_actions.set_valign(gtk::Align::Center);

        let current_incoming_call_id = Rc::new(RefCell::new(String::new()));
        let current_incoming_caller_name = Rc::new(RefCell::new(String::new()));
        let current_caller_number = Rc::new(RefCell::new(String::new()));

        let answer_btn = gtk::Button::builder()
            .icon_name("call-start-symbolic")
            .css_classes(["circular", "call-banner-answer"])
            .tooltip_text("Anruf annehmen")
            .build();
        banner_actions.append(&answer_btn);

        let reject_btn = gtk::Button::builder()
            .icon_name("call-stop-symbolic")
            .css_classes(["circular", "call-banner-reject"])
            .tooltip_text("Anruf ablehnen")
            .build();
        banner_actions.append(&reject_btn);

        let cb_close = call_banner.clone();
        let close_btn = gtk::Button::from_icon_name("window-close-symbolic");
        close_btn.add_css_class("flat");
        close_btn.add_css_class("circular");
        close_btn.add_css_class("call-banner-close");
        close_btn.set_tooltip_text(Some("Banner ausblenden"));
        close_btn.connect_clicked(move |_| {
            cb_close.set_visible(false);
        });
        banner_actions.append(&close_btn);

        call_banner.append(&banner_actions);
        crate::ui::attach_call_drag_source(&call_banner);
        content_box.append(&call_banner);

        // 2c. In-App Call Pickup / Gruppenruf-Signalisierungs-Banner (minimalistisch)
        let pickup_revealer = gtk::Revealer::new();
        pickup_revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        pickup_revealer.set_reveal_child(false);

        let pickup_banner = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        pickup_banner.add_css_class("pickup-call-banner");

        let pickup_avatar = adw::Avatar::new(34, Some("?"), true);
        pickup_avatar.set_valign(gtk::Align::Center);
        pickup_banner.append(&pickup_avatar);

        let pickup_info_box = gtk::Box::new(gtk::Orientation::Vertical, 1);
        pickup_info_box.set_hexpand(true);
        pickup_info_box.set_valign(gtk::Align::Center);

        let pickup_name = gtk::Label::new(None);
        pickup_name.add_css_class("heading");
        pickup_name.set_halign(gtk::Align::Start);
        pickup_name.set_hexpand(true);
        pickup_name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        pickup_info_box.append(&pickup_name);

        let pickup_details = gtk::Label::new(None);
        pickup_details.add_css_class("caption");
        pickup_details.add_css_class("dim-label");
        pickup_details.set_halign(gtk::Align::Start);
        pickup_details.set_hexpand(true);
        pickup_details.set_ellipsize(gtk::pango::EllipsizeMode::End);
        pickup_info_box.append(&pickup_details);

        pickup_banner.append(&pickup_info_box);

        let pickup_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        pickup_actions.set_valign(gtk::Align::Center);

        let current_pickup_ext = Rc::new(RefCell::new(String::new()));
        let current_pickup_name = Rc::new(RefCell::new(String::new()));

        let pickup_btn = gtk::Button::builder()
            .icon_name("call-start-symbolic")
            .css_classes(["circular", "call-banner-answer"])
            .tooltip_text(&crate::i18n::tr("Pick up colleague's call"))
            .build();

        let trigger_call_holder: Rc<RefCell<Option<Rc<dyn Fn(String, String)>>>> = Rc::new(RefCell::new(None));
        let transfer_call_holder: Rc<RefCell<Option<Rc<dyn Fn(String, String)>>>> = Rc::new(RefCell::new(None));
        let select_number_holder: Rc<RefCell<Option<Rc<dyn Fn(String)>>>> = Rc::new(RefCell::new(None));
        let fav_changed_holder: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let sidebar_reload_hotkeys_holder: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let open_chat_holder: Rc<RefCell<Option<Rc<dyn Fn(String, String)>>>> = Rc::new(RefCell::new(None));
        let tc_pickup = trigger_call_holder.clone();
        let p_ext_btn = current_pickup_ext.clone();
        let p_name_btn = current_pickup_name.clone();
        let p_rev_btn = pickup_revealer.clone();
        let bridge_pickup = bridge.clone();

        pickup_btn.connect_clicked(move |_| {
            p_rev_btn.set_reveal_child(false);
            let ext = p_ext_btn.borrow().clone();
            let cname = p_name_btn.borrow().clone();
            if !ext.is_empty() {
                let st = bridge_pickup.load_or_init_data();
                let dial_target = crate::models::format_pickup_number(&st.pickup_prefix, &ext);
                if let Some(ref tc) = *tc_pickup.borrow() {
                    tc(crate::i18n::trf("Pickup {name}", &[("name", &cname)]), dial_target);
                }
            }
        });
        pickup_actions.append(&pickup_btn);

        let p_rev_close = pickup_revealer.clone();
        let pickup_close_btn = gtk::Button::from_icon_name("window-close-symbolic");
        pickup_close_btn.add_css_class("flat");
        pickup_close_btn.add_css_class("circular");
        pickup_close_btn.add_css_class("call-banner-close");
        pickup_close_btn.set_tooltip_text(Some(&crate::i18n::tr("Dismiss notification")));
        pickup_close_btn.connect_clicked(move |_| {
            p_rev_close.set_reveal_child(false);
        });
        pickup_actions.append(&pickup_close_btn);

        pickup_banner.append(&pickup_actions);
        pickup_revealer.set_child(Some(&pickup_banner));
        content_box.append(&pickup_revealer);

        let content_stack = adw::ViewStack::new();
        content_stack.set_hhomogeneous(false);
        content_stack.set_vhomogeneous(false);
        content_stack.set_vexpand(true);
        content_stack.set_hexpand(true);

        // Initialize Native Softphone Engine Bridge
        let engine_bridge = Rc::new(
            EngineBridge::new().expect("Failed to initialize EngineBridge")
        );

        // Wire In-Call DTMF action
        let eb_dtmf = engine_bridge.clone();
        let active_c_dtmf = active_call.clone();
        let toast_dtmf = toast_overlay.clone();
        let on_send_dtmf = Rc::new(move |digit: char| {
            let cid = active_c_dtmf
                .borrow()
                .as_ref()
                .map(|c| c.id.clone())
                .unwrap_or_else(|| "active-call".into());
            let eb = eb_dtmf.clone();
            let toast_o = toast_dtmf.clone();
            glib::MainContext::default().spawn_local(async move {
                if let Err(e) = eb.send_dtmf(&cid, digit).await {
                    let t = adw::Toast::new(&format!("DTMF: {}", e));
                    t.set_timeout(2);
                    toast_o.add_toast(t);
                }
            });
        });

        // 3. Full In-Call Components (Available for detailed in-call screen with live DTMF)
        let dtmf_cb_incall = on_send_dtmf.clone();
        let incall_comps = create_incall_view(move |d| dtmf_cb_incall(d));

        // 3b. Dedicated Dialer View
        let tc_dialer_holder: Rc<RefCell<Option<Rc<dyn Fn(String, String)>>>> = Rc::new(RefCell::new(None));
        let tc_for_dialer = tc_dialer_holder.clone();
        let dialer_comps = create_dialer_view(move |name, num| {
            if let Some(tc) = tc_for_dialer.borrow().as_ref() {
                tc(name, num);
            }
        });

        // 4. Sidebar Setup
        let cs_select = content_stack.clone();
        let win_title_sel = window_title.clone();
        let sb_btn = search_button.clone();
        let journal_search_bar_holder: Rc<RefCell<Option<gtk::SearchBar>>> = Rc::new(RefCell::new(None));
        let contacts_search_bar_holder: Rc<RefCell<Option<gtk::SearchBar>>> = Rc::new(RefCell::new(None));
        let journal_mark_seen_holder: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let badge_journal_holder: Rc<RefCell<Option<gtk::Label>>> = Rc::new(RefCell::new(None));
        let j_sb_holder = journal_search_bar_holder.clone();
        let c_sb_holder = contacts_search_bar_holder.clone();
        let j_seen_holder = journal_mark_seen_holder.clone();
        let b_holder = badge_journal_holder.clone();
        let win_clone = window.clone();
        let cur_page = Rc::new(RefCell::new(String::from("journal")));
        let cur_page_sel = cur_page.clone();
        let auto_popover_holder: Rc<RefCell<Option<gtk::Popover>>> = Rc::new(RefCell::new(None));
        let auto_pop_sel = auto_popover_holder.clone();
        let sv_select = split_view.clone();
        let c_page_sel = content_nav_page.clone();

        let on_select = Rc::new(move |page: &str| {
            if let Some(p) = auto_pop_sel.borrow().as_ref() {
                p.popdown();
            }
            let prev = cur_page_sel.borrow().clone();
            *cur_page_sel.borrow_mut() = page.to_string();

            // Beim Verlassen des Rufjournals: Ungesehene Anrufe als gesehen markieren & Badge ausblenden
            if prev == "journal" && page != "journal" {
                if let Some(mark_seen) = j_seen_holder.borrow().as_ref() {
                    mark_seen();
                }
                if let Some(b) = b_holder.borrow().as_ref() {
                    b.set_visible(false);
                }
            }

            // In mobiler / schmaler Ansicht automatisch zur Inhaltsseite wechseln
            sv_select.set_show_content(true);

            match page {
                "journal" => {
                    let t = crate::i18n::tr("Call Journal");
                    c_page_sel.set_title(&t);
                    win_title_sel.set_title(&t);
                    cs_select.set_visible_child_name("journal");
                    sb_btn.set_visible(true);
                    if let Some(j_sbar) = j_sb_holder.borrow().as_ref() {
                        sb_btn.set_active(j_sbar.is_search_mode());
                        j_sbar.set_key_capture_widget(Some(&win_clone));
                    }
                    if let Some(c_sbar) = c_sb_holder.borrow().as_ref() {
                        c_sbar.set_search_mode(false);
                        c_sbar.set_key_capture_widget(gtk::Widget::NONE);
                    }
                    // Nach kurzem Hinsehen (2 Sekunden) ungesehene Anrufe automatisch als gesehen markieren
                    let j_seen_cb = j_seen_holder.clone();
                    let b_cb = b_holder.clone();
                    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(2000), move || {
                        if let Some(mark_seen) = j_seen_cb.borrow().as_ref() {
                            mark_seen();
                        }
                        if let Some(b) = b_cb.borrow().as_ref() {
                            b.set_visible(false);
                        }
                    });
                }
                "contacts" => {
                    let t = crate::i18n::tr("Contacts");
                    c_page_sel.set_title(&t);
                    win_title_sel.set_title(&t);
                    cs_select.set_visible_child_name("contacts");
                    sb_btn.set_visible(true);
                    if let Some(c_sbar) = c_sb_holder.borrow().as_ref() {
                        sb_btn.set_active(c_sbar.is_search_mode());
                        c_sbar.set_key_capture_widget(Some(&win_clone));
                    }
                    if let Some(j_sbar) = j_sb_holder.borrow().as_ref() {
                        j_sbar.set_search_mode(false);
                        j_sbar.set_key_capture_widget(gtk::Widget::NONE);
                    }
                }
                "voicebox" => {
                    let t = crate::i18n::tr("Voicemail");
                    c_page_sel.set_title(&t);
                    win_title_sel.set_title(&t);
                    cs_select.set_visible_child_name("voicebox");
                    sb_btn.set_visible(false);
                    if let Some(j_sbar) = j_sb_holder.borrow().as_ref() {
                        j_sbar.set_search_mode(false);
                        j_sbar.set_key_capture_widget(gtk::Widget::NONE);
                    }
                    if let Some(c_sbar) = c_sb_holder.borrow().as_ref() {
                        c_sbar.set_search_mode(false);
                        c_sbar.set_key_capture_widget(gtk::Widget::NONE);
                    }
                }
                "incall" => {
                    let t = crate::i18n::tr("Active Call");
                    c_page_sel.set_title(&t);
                    win_title_sel.set_title(&t);
                    cs_select.set_visible_child_name("incall");
                    sb_btn.set_visible(false);
                    if let Some(j_sbar) = j_sb_holder.borrow().as_ref() {
                        j_sbar.set_search_mode(false);
                        j_sbar.set_key_capture_widget(gtk::Widget::NONE);
                    }
                    if let Some(c_sbar) = c_sb_holder.borrow().as_ref() {
                        c_sbar.set_search_mode(false);
                        c_sbar.set_key_capture_widget(gtk::Widget::NONE);
                    }
                }
                "dialer" => {
                    let t = crate::i18n::tr("Keypad");
                    c_page_sel.set_title(&t);
                    win_title_sel.set_title(&t);
                    cs_select.set_visible_child_name("dialer");
                    sb_btn.set_visible(false);
                    if let Some(j_sbar) = j_sb_holder.borrow().as_ref() {
                        j_sbar.set_search_mode(false);
                        j_sbar.set_key_capture_widget(gtk::Widget::NONE);
                    }
                    if let Some(c_sbar) = c_sb_holder.borrow().as_ref() {
                        c_sbar.set_search_mode(false);
                        c_sbar.set_key_capture_widget(gtk::Widget::NONE);
                    }
                }
                _ => (),
            }
        });

        // Call action placeholders to be connected
        let active_c_swap = active_call.clone();
        let held_c_swap = held_call.clone();
        let active_c_hang = active_call.clone();
        let held_c_hang = held_call.clone();
        let active_c_trig = active_call.clone();
        let held_c_trig = held_call.clone();

        let toast_overlay_cti = toast_overlay.clone();
        let on_toggle_cti = move |mode: TelephonyMode| {
            let msg = match mode {
                TelephonyMode::Cti => "CTI-Modus aktiviert (Tischtelefon-Steuerung aktiv)",
                TelephonyMode::Softphone => "Softphone-Modus aktiviert (PC-Audio aktiv)",
            };
            let toast = adw::Toast::new(msg);
            toast.set_timeout(2);
            toast_overlay_cti.add_toast(toast);
        };

        // Wire forward persistence & active PBX sync callback
        let bridge_fwd = bridge.clone();
        let eb_fwd = engine_bridge.clone();
        let toast_fwd = toast_overlay.clone();
        let on_forward_changed = move |enabled: bool, number: String| {
            let mut st = bridge_fwd.load_or_init_data();
            let prev_enabled = st.forward_enabled;
            let prev_num = st.forward_number.clone();
            st.forward_enabled = enabled;
            st.forward_number = number.clone();
            bridge_fwd.save_data(&st);

            if !st.passive_mode && (prev_enabled != enabled || (enabled && prev_num != number)) {
                eb_fwd.set_call_forwarding(enabled, number.clone());
                let msg = if enabled && !number.is_empty() {
                    format!("Rufumleitung an {} aktiv", number)
                } else {
                    "Rufumleitung deaktiviert".to_string()
                };
                let t = adw::Toast::new(&msg);
                t.set_timeout(2);
                toast_fwd.add_toast(t);
            }
        };

        // Initialize StatusNotifierItem (System-Tray) icon for GNOME Top-Bar / Statusleiste
        let (tray_h_opt, tray_rx) = crate::tray::DeskphoneTray::spawn_service(*current_presence.borrow());
        let tray_handle = Rc::new(RefCell::new(tray_h_opt));

        // Wire presence change callback to sync user status with Swyx PBX / SIP
        let bridge_pres = bridge.clone();
        let eb_pres = engine_bridge.clone();
        let toast_pres = toast_overlay.clone();
        let h_pres = tray_handle.clone();
        let on_presence_changed = move |state: PresenceState| {
            let st = bridge_pres.load_or_init_data();
            if !st.passive_mode {
                eb_pres.set_user_presence(state);
                let t = adw::Toast::new(&crate::i18n::trf("Presence status: {status}", &[("status", &state.label())]));
                t.set_timeout(2);
                toast_pres.add_toast(t);
            }
            if let Some(ref h) = *h_pres.borrow() {
                h.update(move |t| {
                    t.presence = state;
                });
            }
        };

        // We will wire closures with refs to sidebar and incall components below
        let on_sel_sidebar = on_select.clone();
        let tc_hk = trigger_call_holder.clone();
        let bridge_hk = bridge.clone();
        let on_hotkey_click = move |name: String, number: String, presence: PresenceState| {
            if presence == PresenceState::Ringing {
                let st = bridge_hk.load_or_init_data();
                let dial_target = crate::models::format_pickup_number(&st.pickup_prefix, &number);
                if let Some(ref tc) = *tc_hk.borrow() {
                    tc(crate::i18n::trf("Pickup {name}", &[("name", &name)]), dial_target);
                }
            } else if let Some(ref tc) = *tc_hk.borrow() {
                tc(name, number);
            }
        };

        let tr_hk = transfer_call_holder.clone();
        let on_sidebar_transfer = move |name: String, number: String| {
            if let Some(ref tr) = *tr_hk.borrow() {
                tr(name, number);
            }
        };

        let sn_hk = select_number_holder.clone();
        let on_sidebar_copy = move |number: String| {
            if let Some(ref sn) = *sn_hk.borrow() {
                sn(number);
            }
        };

        let fav_hk = fav_changed_holder.clone();
        let on_sidebar_fav = move || {
            if let Some(ref f) = *fav_hk.borrow() {
                f();
            }
        };

        let ch_hk = open_chat_holder.clone();
        let on_sidebar_chat = move |name: String, number: String| {
            if let Some(ref ch) = *ch_hk.borrow() {
                ch(name, number);
            }
        };

        let br_dp_save = bridge.clone();
        let on_dialpad_toggle = move |visible: bool| {
            let mut st = br_dp_save.load_or_init_data();
            if st.show_dialpad != visible {
                st.show_dialpad = visible;
                br_dp_save.save_data(&st);
            }
        };

        let tc_sb_dial = trigger_call_holder.clone();
        let sidebar_comps = create_sidebar(
            contacts_list.clone(),
            speed_dials.clone(),
            toast_overlay.clone(),
            current_audio.clone(),
            current_presence.clone(),
            current_telephony_mode.clone(),
            active_call.clone(),
            held_call.clone(),
            store.user_name.clone(),
            store.extension.clone(),
            store.forward_number.clone(),
            store.forward_enabled,
            store.show_dialpad
                || (is_demo && std::env::var_os("DESKPHONE_CAPTURE_DIALPAD").is_some()),
            move |name, number| {
                if let Some(ref tc) = *tc_sb_dial.borrow() {
                    tc(name, number);
                }
            },
            move |page| on_sel_sidebar(page),
            on_toggle_cti,
            on_forward_changed,
            on_presence_changed,
            on_hotkey_click,
            on_sidebar_transfer,
            on_sidebar_copy,
            on_sidebar_fav,
            on_sidebar_chat,
            on_dialpad_toggle,
        );
        window.add_css_class("call-blocked");
        (sidebar_comps.set_backend_ready)(false);
        *badge_journal_holder.borrow_mut() = Some(sidebar_comps.journal_badge.clone());
        *auto_popover_holder.borrow_mut() = Some(sidebar_comps.autocomplete_popover.clone());
        *sidebar_reload_hotkeys_holder.borrow_mut() = Some(sidebar_comps.reload_hotkeys.clone());

        sidebar_comps.hotkeys_toggle_btn.set_active(store.show_hotkeys_sidebar);
        sidebar_comps.hotkeys_revealer.set_reveal_child(store.show_hotkeys_sidebar);
        let br_hk_save = bridge.clone();
        sidebar_comps.hotkeys_toggle_btn.connect_toggled(move |btn| {
            let show = btn.is_active();
            let mut st = br_hk_save.load_or_init_data();
            if st.show_hotkeys_sidebar != show {
                st.show_hotkeys_sidebar = show;
                br_hk_save.save_data(&st);
            }
        });

        content_header.pack_end(&conn_spinner);

        // Kompaktes DTMF-Tastenfeld-Popover für die untere Anrufleiste in der Sidebar
        let dtmf_popover = gtk::Popover::new();
        dtmf_popover.set_parent(&sidebar_comps.docked_dtmf_btn);
        let dtmf_pop_grid = gtk::Grid::new();
        dtmf_pop_grid.set_row_spacing(6);
        dtmf_pop_grid.set_column_spacing(8);
        dtmf_pop_grid.set_margin_top(8);
        dtmf_pop_grid.set_margin_bottom(8);
        dtmf_pop_grid.set_margin_start(8);
        dtmf_pop_grid.set_margin_end(8);
        dtmf_pop_grid.set_halign(gtk::Align::Center);

        let dtmf_dock_keys = [
            ("1", 0, 0), ("2", 1, 0), ("3", 2, 0),
            ("4", 0, 1), ("5", 1, 1), ("6", 2, 1),
            ("7", 0, 2), ("8", 1, 2), ("9", 2, 2),
            ("*", 0, 3), ("0", 1, 3), ("#", 2, 3),
        ];

        for (k, col, row) in dtmf_dock_keys {
            let b = gtk::Button::with_label(k);
            b.add_css_class("flat");
            b.add_css_class("circular");
            b.set_size_request(40, 40);
            let ch = k.chars().next().unwrap_or(' ');
            let dtmf_send = on_send_dtmf.clone();
            b.connect_clicked(move |_| {
                dtmf_send(ch);
            });
            dtmf_pop_grid.attach(&b, col, row, 1, 1);
        }
        dtmf_popover.set_child(Some(&dtmf_pop_grid));
        let pop_btn = dtmf_popover.clone();
        sidebar_comps.docked_dtmf_btn.connect_clicked(move |_| {
            pop_btn.popup();
        });

        let journal_reload_holder: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let contacts_reload_holder: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
        let gnome_contacts_sync_holder: Rc<RefCell<Option<Rc<dyn Fn(bool)>>>> = Rc::new(RefCell::new(None));
        let reconnect_holder: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

        // GNOME HIG Menü-Aktionen: win.preferences, win.about und win.quit
        let act_pref = gtk::gio::SimpleAction::new("preferences", None);
        let win_set = window.clone();
        let br_set = bridge.clone();
        let eb_set = engine_bridge.clone();
        let st_set = Rc::new(RefCell::new(store.clone()));
        let toast_set = toast_overlay.clone();
        let u_lbl_set = sidebar_comps.user_label.clone();
        let u_av_set = sidebar_comps.user_avatar.clone();
        let j_rel_set = journal_reload_holder.clone();
        let c_rel_set = contacts_reload_holder.clone();
        let gc_sync_set = gnome_contacts_sync_holder.clone();
        let reconn_set = reconnect_holder.clone();
        act_pref.connect_activate(move |_, _| {
            let latest_st = br_set.load_or_init_data();
            *st_set.borrow_mut() = latest_st;
            let j_cb = j_rel_set.clone();
            let c_cb = c_rel_set.clone();
            let gc_cb = gc_sync_set.clone();
            let rc_cb = reconn_set.clone();
            crate::ui::settings_dialog::open_settings_dialog(
                &win_set,
                br_set.clone(),
                eb_set.clone(),
                st_set.clone(),
                toast_set.clone(),
                u_lbl_set.clone(),
                u_av_set.clone(),
                Rc::new(move || {
                    if let Some(r) = c_cb.borrow().as_ref() {
                        r();
                    }
                    if let Some(r) = j_cb.borrow().as_ref() {
                        r();
                    }
                }),
                Rc::new(move |enabled| {
                    if let Some(sync_fn) = gc_cb.borrow().as_ref() {
                        sync_fn(enabled);
                    }
                }),
                Rc::new(move || {
                    if let Some(reconn) = rc_cb.borrow().as_ref() {
                        reconn();
                    }
                }),
            );
        });
        window.add_action(&act_pref);

        let act_shortcuts = gtk::gio::SimpleAction::new("shortcuts", None);
        let win_short = window.clone();
        act_shortcuts.connect_activate(move |_, _| {
            use crate::i18n::tr;
            let dlg = adw::ShortcutsDialog::new();

            let sec_nav = adw::ShortcutsSection::new(Some(&tr("General")));
            sec_nav.add(adw::ShortcutsItem::new(&tr("Search"), "<Control>f"));
            sec_nav.add(adw::ShortcutsItem::new(&tr("Main Menu"), "F10"));
            sec_nav.add(adw::ShortcutsItem::new(&tr("Open Preferences"), "<Control>comma"));
            sec_nav.add(adw::ShortcutsItem::new(&tr("Show Keyboard Shortcuts"), "<Control>question"));
            sec_nav.add(adw::ShortcutsItem::new(&tr("Close Window"), "<Control>w"));
            sec_nav.add(adw::ShortcutsItem::new(&tr("Quit"), "<Control>q"));
            dlg.add(sec_nav);

            let sec_call = adw::ShortcutsSection::new(Some(&tr("Telephony")));
            sec_call.add(adw::ShortcutsItem::new(&tr("Dial number directly (in dial field)"), "Return"));
            sec_call.add(adw::ShortcutsItem::new(&tr("Close search"), "Escape"));
            dlg.add(sec_call);

            dlg.present(Some(&win_short));
        });
        window.add_action(&act_shortcuts);
        app.set_accels_for_action("win.preferences", &["<Control>comma"]);
        app.set_accels_for_action("win.shortcuts", &["<Control>question"]);
        app.set_accels_for_action("win.close", &["<Control>w"]);
        app.set_accels_for_action("win.quit", &["<Control>q"]);

        let act_close = gtk::gio::SimpleAction::new("close", None);
        let win_close = window.clone();
        act_close.connect_activate(move |_, _| {
            win_close.close();
        });
        window.add_action(&act_close);

        let act_about = gtk::gio::SimpleAction::new("about", None);
        let win_about = window.clone();
        act_about.connect_activate(move |_, _| {
            let about = adw::AboutDialog::builder()
                .application_name("Deskphone")
                .application_icon(crate::APP_ID)
                .version(env!("CARGO_PKG_VERSION"))
                .developer_name("Martin Gawron")
                .developers(["Martin Gawron"])
                .copyright("© 2026 Martin Gawron")
                .website("https://deskphone.martingawron.com")
                .issue_url("https://github.com/megawron/deskphone/issues")
                .comments(&crate::i18n::tr("Modern native SIP and PBX softphone for GNOME"))
                .license_type(gtk::License::Gpl30)
                .build();
            about.present(Some(&win_about));
        });
        window.add_action(&act_about);

        let force_quit = Rc::new(std::cell::Cell::new(false));
        let fq_quit = force_quit.clone();
        let ring_quit = ringtone_player.clone();
        let app_quit = app.clone();
        let act_quit = gtk::gio::SimpleAction::new("quit", None);
        act_quit.connect_activate(move |_, _| {
            fq_quit.set(true);
            ring_quit.stop();
            app_quit.quit();
        });
        window.add_action(&act_quit);

        // Safe URI handler action (tel:/sip:): fills the dial field without auto-calling
        let act_prefill = gtk::gio::SimpleAction::new(
            "prefill-number",
            Some(gtk::glib::VariantTy::STRING),
        );
        let sel_prefill = select_number_holder.clone();
        let win_prefill = window.clone();
        act_prefill.connect_activate(move |_, param| {
            if let Some(num) = param.and_then(|v| v.get::<String>()) {
                if !num.trim().is_empty() {
                    if let Some(ref sel) = *sel_prefill.borrow() {
                        sel(num);
                    }
                    win_prefill.set_visible(true);
                    win_prefill.present();
                }
            }
        });
        window.add_action(&act_prefill);

        // Process Tray Actions (Click on status bar icon or context menu items)
        let win_tray = window.clone();
        let ans_btn_tray = answer_btn.clone();
        let rej_btn_tray = reject_btn.clone();
        let hang_btn_tray = sidebar_comps.docked_hangup_btn.clone();
        let fq_tray = force_quit.clone();
        let app_tray = app.clone();
        let ring_tray = ringtone_player.clone();
        let pres_tray = current_presence.clone();
        let u_dot_tray = sidebar_comps.user_dot.clone();
        let eb_tray = engine_bridge.clone();
        let br_tray = bridge.clone();
        let toast_tray = toast_overlay.clone();
        let h_tray_action = tray_handle.clone();

        glib::spawn_future_local(async move {
            while let Ok(action) = tray_rx.recv().await {
                match action {
                    crate::tray::TrayAction::ShowWindow => {
                        win_tray.set_visible(true);
                        win_tray.present();
                    }
                    crate::tray::TrayAction::ToggleWindow => {
                        if win_tray.is_visible() {
                            win_tray.set_visible(false);
                        } else {
                            win_tray.set_visible(true);
                            win_tray.present();
                        }
                    }
                    crate::tray::TrayAction::AnswerIncomingCall => {
                        win_tray.set_visible(true);
                        win_tray.present();
                        ans_btn_tray.emit_clicked();
                    }
                    crate::tray::TrayAction::RejectIncomingCall => {
                        rej_btn_tray.emit_clicked();
                    }
                    crate::tray::TrayAction::HangupCall => {
                        hang_btn_tray.emit_clicked();
                    }
                    crate::tray::TrayAction::SetPresence(new_pres) => {
                        *pres_tray.borrow_mut() = new_pres;
                        u_dot_tray.remove_css_class("presence-available");
                        u_dot_tray.remove_css_class("presence-busy");
                        u_dot_tray.remove_css_class("presence-away");
                        u_dot_tray.remove_css_class("presence-offline");
                        u_dot_tray.remove_css_class("presence-ringing");
                        u_dot_tray.add_css_class(new_pres.css_class());

                        let st = br_tray.load_or_init_data();
                        if !st.passive_mode {
                            eb_tray.set_user_presence(new_pres);
                            let t = adw::Toast::new(&crate::i18n::trf("Presence status: {status}", &[("status", &new_pres.label())]));
                            t.set_timeout(2);
                            toast_tray.add_toast(t);
                        }

                        if let Some(ref h) = *h_tray_action.borrow() {
                            h.update(move |t| {
                                t.presence = new_pres;
                            });
                        }
                    }
                    crate::tray::TrayAction::Quit => {
                        fq_tray.set(true);
                        ring_tray.stop();
                        app_tray.quit();
                    }
                }
            }
        });

        // Zero-Config Onboarding: the AdwBanner directly provides a non-blocking "Preferences" button
        // instead of popping up a blocking modal dialog on first launch.

        // Wire Swap Action (Makeln zwischen Leitung 1 & Leitung 2)
        let s_caller = sidebar_comps.docked_caller_label.clone();
        let s_timer = sidebar_comps.docked_timer_label.clone();
        let s_held_box = sidebar_comps.docked_held_box.clone();
        let s_held_lbl = sidebar_comps.docked_held_label.clone();
        let s_b1_swap = sidebar_comps.line1_badge.clone();
        let s_b2_swap = sidebar_comps.line2_badge.clone();
        let s_i2_swap = sidebar_comps.line2_icon.clone();
        let s_vol1_swap = sidebar_comps.line1_volume_scale.clone();
        let s_vol2_swap = sidebar_comps.line2_volume_scale.clone();
        let s_mute_swap = sidebar_comps.docked_mute_btn.clone();
        let i_caller = incall_comps.caller_name_label.clone();
        let i_ext = incall_comps.caller_ext_label.clone();
        let i_timer = incall_comps.timer_label.clone();
        let i_held_banner = incall_comps.held_banner.clone();
        let i_held_lbl = incall_comps.held_pill_label.clone();
        let i_hold_btn_swap = incall_comps.hold_button.clone();
        let i_mute_swap = incall_comps.mute_button.clone();

        let eb_swap = engine_bridge.clone();
        let do_swap = move || {
            let mut act = active_c_swap.borrow_mut();
            let mut hld = held_c_swap.borrow_mut();

            if let (Some(cur_act), Some(cur_hld)) = (act.clone(), hld.clone()) {
                let eb = eb_swap.clone();
                let act_id = cur_act.id.clone();
                let act_was_held = cur_act.is_held;
                let hld_id = cur_hld.id.clone();
                glib::MainContext::default().spawn_local(async move {
                    if !act_was_held {
                        let _ = eb.hold(&act_id).await;
                    }
                    let _ = eb.unhold(&hld_id).await;
                    eb.set_mute(false).await;
                });
                *act = Some(ActiveCall {
                    id: cur_hld.id,
                    caller_name: cur_hld.caller_name.clone(),
                    extension: cur_hld.extension.clone(),
                    full_number: format!("DW {}", cur_hld.extension),
                    duration_secs: cur_hld.duration_secs,
                    is_muted: false,
                    is_held: false,
                });
                *hld = Some(HeldCall {
                    id: cur_act.id,
                    caller_name: cur_act.caller_name.clone(),
                    extension: cur_act.extension.clone(),
                    duration_secs: cur_act.duration_secs,
                });

                // Reset hold & mute buttons for newly active line
                i_hold_btn_swap.remove_css_class("suggested-action");
                i_hold_btn_swap.set_icon_name("media-playback-pause-symbolic");
                i_hold_btn_swap.set_tooltip_text(Some("Makeln / Halten"));
                s_b1_swap.set_text(&crate::i18n::tr("Active"));
                s_b1_swap.remove_css_class("warning");
                s_b1_swap.remove_css_class("dim-label");
                s_b1_swap.add_css_class("success");
                s_mute_swap.remove_css_class("call-btn-active-mute");
                s_mute_swap.set_icon_name("audio-input-microphone-symbolic");
                s_mute_swap.set_tooltip_text(Some(&crate::i18n::tr("Mute")));
                i_mute_swap.remove_css_class("call-btn-active-mute");
                i_mute_swap.set_icon_name("audio-input-microphone-symbolic");
                i_mute_swap.set_tooltip_text(Some(&crate::i18n::tr("Mute")));

                // Update Sidebar Dock & Lines
                s_caller.set_text(&format_caller_display(&cur_hld.caller_name, &cur_hld.extension));
                s_timer.set_text(&format!(
                    "{:02}:{:02}",
                    cur_hld.duration_secs / 60,
                    cur_hld.duration_secs % 60
                ));
                s_held_lbl.set_text(&format!("Halten: {} ({})", cur_act.caller_name, cur_act.extension));
                s_held_box.set_visible(true);
                s_b2_swap.set_text("Halten");
                s_i2_swap.set_icon_name(Some("audio-volume-high-symbolic"));
                s_vol1_swap.set_visible(true);
                s_vol2_swap.set_visible(true);

                // Update InCall View
                i_caller.set_text(&cur_hld.caller_name);
                i_ext.set_text(&crate::i18n::trf("{number} • Line 1 • Connected", &[("number", &cur_hld.extension)]));
                i_timer.set_text(&format!(
                    "{:02}:{:02}",
                    cur_hld.duration_secs / 60,
                    cur_hld.duration_secs % 60
                ));
                i_held_lbl.set_text(&format!(
                    "{} ({}) • In Warteschleife {:02}:{:02}",
                    cur_act.caller_name,
                    cur_act.extension,
                    cur_act.duration_secs / 60,
                    cur_act.duration_secs % 60
                ));
                i_held_banner.set_visible(true);
            }
        };

        let swap_c1 = do_swap.clone();
        sidebar_comps.docked_swap_btn.connect_clicked(move |_| {
            swap_c1();
        });

        let swap_c2 = do_swap.clone();
        incall_comps.held_pill_button.connect_clicked(move |_| {
            swap_c2();
        });

        // Klick auf Leitung 2 in der Seitenleiste wechselt direkt zur gehaltenen Leitung (Makeln)
        let swap_line2_click = do_swap.clone();
        let hld_line2_check = held_call.clone();
        let line2_swap_gesture = gtk::GestureClick::new();
        line2_swap_gesture.connect_released(move |_, _, _, _| {
            if hld_line2_check.borrow().is_some() {
                swap_line2_click();
            }
        });
        sidebar_comps.line2_chip.add_controller(line2_swap_gesture);

        // Wire Hangup & Active-Call Cleanup Action
        let eb_hang = engine_bridge.clone();
        let eb_promote = engine_bridge.clone();
        let s_dock_box = sidebar_comps.docked_call_box.clone();
        let s_held_box_hang = sidebar_comps.docked_held_box.clone();
        let s_caller_hang = sidebar_comps.docked_caller_label.clone();
        let s_timer_hang = sidebar_comps.docked_timer_label.clone();
        let s_chip1_hang = sidebar_comps.line1_chip.clone();
        let s_b1_hang = sidebar_comps.line1_badge.clone();
        let s_i1_hang = sidebar_comps.line1_icon.clone();
        let s_vol1_hang = sidebar_comps.line1_volume_scale.clone();
        let s_vol2_hang = sidebar_comps.line2_volume_scale.clone();
        let s_b2_hang = sidebar_comps.line2_badge.clone();
        let s_i2_hang = sidebar_comps.line2_icon.clone();
        let s_mute_hang = sidebar_comps.docked_mute_btn.clone();
        let i_caller_hang = incall_comps.caller_name_label.clone();
        let i_ext_hang = incall_comps.caller_ext_label.clone();
        let i_timer_hang = incall_comps.timer_label.clone();
        let i_banner_hang = incall_comps.held_banner.clone();
        let i_mute_hang = incall_comps.mute_button.clone();
        let i_hold_hang = incall_comps.hold_button.clone();
        let on_sel_hang = on_select.clone();
        let ring_hang = ringtone_player.clone();
        let h_hang = tray_handle.clone();
        let pres_hang = current_presence.clone();

        let finish_active_call_ui = move || {
            ring_hang.stop();
            let mut act = active_c_hang.borrow_mut();
            let mut hld = held_c_hang.borrow_mut();

            // Reset mute & hold indicators
            s_mute_hang.remove_css_class("call-btn-active-mute");
            s_mute_hang.set_icon_name("audio-input-microphone-symbolic");
            s_mute_hang.set_tooltip_text(Some(&crate::i18n::tr("Mute")));
            i_mute_hang.remove_css_class("call-btn-active-mute");
            i_mute_hang.set_icon_name("audio-input-microphone-symbolic");
            i_mute_hang.set_tooltip_text(Some(&crate::i18n::tr("Mute")));
            i_hold_hang.remove_css_class("suggested-action");
            i_hold_hang.set_icon_name("media-playback-pause-symbolic");
            i_hold_hang.set_tooltip_text(Some(&crate::i18n::tr("Hold")));

            if let Some(held) = hld.take() {
                // Promote held call to active AND unhold on SIP engine!
                let held_id_unhold = held.id.clone();
                let eb_u = eb_promote.clone();
                glib::MainContext::default().spawn_local(async move {
                    let _ = eb_u.unhold(&held_id_unhold).await;
                    eb_u.set_mute(false).await;
                });

                *act = Some(ActiveCall {
                    id: held.id,
                    caller_name: held.caller_name.clone(),
                    extension: held.extension.clone(),
                    full_number: format!("DW {}", held.extension),
                    duration_secs: held.duration_secs,
                    is_muted: false,
                    is_held: false,
                });

                s_caller_hang.set_text(&format_caller_display(&held.caller_name, &held.extension));
                s_timer_hang.set_text(&format!(
                    "{:02}:{:02}",
                    held.duration_secs / 60,
                    held.duration_secs % 60
                ));
                s_held_box_hang.set_visible(false);

                // Leitung 1 active, Leitung 2 freed
                s_chip1_hang.add_css_class("line-chip-active");
                s_b1_hang.set_text(&crate::i18n::tr("Active"));
                s_b1_hang.remove_css_class("dim-label");
                s_b1_hang.remove_css_class("warning");
                s_b1_hang.add_css_class("success");
                s_i1_hang.set_icon_name(Some("audio-volume-high-symbolic"));
                s_i1_hang.remove_css_class("dim-label");
                s_i1_hang.add_css_class("speaker-green-icon");
                s_vol1_hang.set_visible(true);
                s_vol2_hang.set_visible(false);
                s_b2_hang.set_text(&crate::i18n::tr("Free"));
                s_i2_hang.set_icon_name(Some("call-start-symbolic"));

                i_caller_hang.set_text(&held.caller_name);
                i_ext_hang.set_text(&crate::i18n::trf("{number} • Line 1 • Connected", &[("number", &held.extension)]));
                i_timer_hang.set_text(&format!(
                    "{:02}:{:02}",
                    held.duration_secs / 60,
                    held.duration_secs % 60
                ));
                i_banner_hang.set_visible(false);

                if let Some(ref h) = *h_hang.borrow() {
                    let held_name = held.caller_name.clone();
                    h.update(move |t| {
                        t.call_info = Some(held_name);
                        t.has_incoming_call = false;
                        t.presence = PresenceState::Busy;
                    });
                }
            } else {
                // Call ended completely
                *act = None;
                s_dock_box.set_visible(false);
                s_chip1_hang.remove_css_class("line-chip-active");
                s_b1_hang.set_text(&crate::i18n::tr("Ready"));
                s_b1_hang.remove_css_class("success");
                s_b1_hang.remove_css_class("warning");
                s_b1_hang.add_css_class("dim-label");
                s_i1_hang.set_icon_name(Some("call-start-symbolic"));
                s_i1_hang.remove_css_class("speaker-green-icon");
                s_i1_hang.add_css_class("dim-label");
                s_vol1_hang.set_visible(false);
                s_vol2_hang.set_visible(false);
                s_b2_hang.set_text(&crate::i18n::tr("Free"));
                s_i2_hang.set_icon_name(Some("call-start-symbolic"));
                on_sel_hang("journal");

                if let Some(ref h) = *h_hang.borrow() {
                    let cur_p = *pres_hang.borrow();
                    h.update(move |t| {
                        t.call_info = None;
                        t.has_incoming_call = false;
                        t.presence = cur_p;
                    });
                }
            }
        };

        let active_c_for_hang = active_call.clone();
        let finish_for_hang = finish_active_call_ui.clone();
        let do_hangup = move || {
            let active_cid = active_c_for_hang.borrow().as_ref().map(|c| c.id.clone());
            if let Some(cid) = active_cid {
                let eb = eb_hang.clone();
                glib::MainContext::default().spawn_local(async move {
                    let _ = eb.hangup(&cid).await;
                });
            }
            finish_for_hang();
        };

        let hang_c1 = do_hangup.clone();
        sidebar_comps.docked_hangup_btn.connect_clicked(move |_| {
            hang_c1();
        });

        let hang_c2 = do_hangup.clone();
        incall_comps.hangup_button.connect_clicked(move |_| {
            hang_c2();
        });

        // Wire Mute Action (Synchronized between in-call view and sidebar docked bar)
        let eb_mute = engine_bridge.clone();
        let active_c_mute = active_call.clone();
        let i_mute_click = incall_comps.mute_button.clone();
        let s_mute_click = sidebar_comps.docked_mute_btn.clone();
        let toast_mute = toast_overlay.clone();

        let do_toggle_mute = {
            let active_c = active_c_mute.clone();
            let i_btn = i_mute_click.clone();
            let s_btn = s_mute_click.clone();
            let toast_o = toast_mute.clone();
            let eb = eb_mute.clone();

            move || {
                let mut act = active_c.borrow_mut();
                if let Some(ref mut call) = *act {
                    call.is_muted = !call.is_muted;
                    let is_muted = call.is_muted;

                    let eb_call = eb.clone();
                    glib::MainContext::default().spawn_local(async move {
                        eb_call.set_mute(is_muted).await;
                    });

                    if is_muted {
                        i_btn.add_css_class("call-btn-active-mute");
                        i_btn.set_icon_name("microphone-disabled-symbolic");
                        i_btn.set_tooltip_text(Some(&crate::i18n::tr("Unmute")));

                        s_btn.add_css_class("call-btn-active-mute");
                        s_btn.set_icon_name("microphone-disabled-symbolic");
                        s_btn.set_tooltip_text(Some(&crate::i18n::tr("Unmute")));

                        let toast = adw::Toast::new(&crate::i18n::tr("Microphone muted"));
                        toast.set_timeout(2);
                        toast_o.add_toast(toast);
                    } else {
                        i_btn.remove_css_class("call-btn-active-mute");
                        i_btn.set_icon_name("audio-input-microphone-symbolic");
                        i_btn.set_tooltip_text(Some(&crate::i18n::tr("Mute")));

                        s_btn.remove_css_class("call-btn-active-mute");
                        s_btn.set_icon_name("audio-input-microphone-symbolic");
                        s_btn.set_tooltip_text(Some(&crate::i18n::tr("Mute")));

                        let toast = adw::Toast::new(&crate::i18n::tr("Microphone active"));
                        toast.set_timeout(2);
                        toast_o.add_toast(toast);
                    }
                }
            }
        };

        let mute_c1 = do_toggle_mute.clone();
        incall_comps.mute_button.connect_clicked(move |_| {
            mute_c1();
        });

        let mute_c2 = do_toggle_mute.clone();
        sidebar_comps.docked_mute_btn.connect_clicked(move |_| {
            mute_c2();
        });

        // Wire Hold button in in-call view
        let eb_single_hold = engine_bridge.clone();
        let act_single_hold = active_call.clone();
        let hld_single_hold = held_call.clone();
        let toast_hold = toast_overlay.clone();
        let h_btn_icon = incall_comps.hold_button.clone();
        let s_b1_hold = sidebar_comps.line1_badge.clone();
        let swap_cb = do_swap.clone();

        incall_comps.hold_button.connect_clicked(move |_| {
            if hld_single_hold.borrow().is_some() {
                swap_cb();
            } else {
                let mut act = act_single_hold.borrow_mut();
                if let Some(ref mut c) = *act {
                    c.is_held = !c.is_held;
                    let is_h = c.is_held;
                    let cid = c.id.clone();
                    let eb = eb_single_hold.clone();
                    glib::MainContext::default().spawn_local(async move {
                        if is_h {
                            let _ = eb.hold(&cid).await;
                        } else {
                            let _ = eb.unhold(&cid).await;
                        }
                    });

                    if is_h {
                        h_btn_icon.add_css_class("suggested-action");
                        h_btn_icon.set_icon_name("media-playback-start-symbolic");
                        h_btn_icon.set_tooltip_text(Some(&crate::i18n::tr("Resume call")));
                        s_b1_hold.set_text("Halten");
                        s_b1_hold.remove_css_class("success");
                        s_b1_hold.add_css_class("warning");
                        let t = adw::Toast::new(&crate::i18n::tr("Call on hold"));
                        t.set_timeout(2);
                        toast_hold.add_toast(t);
                    } else {
                        h_btn_icon.remove_css_class("suggested-action");
                        h_btn_icon.set_icon_name("media-playback-pause-symbolic");
                        h_btn_icon.set_tooltip_text(Some(&crate::i18n::tr("Hold")));
                        s_b1_hold.set_text(&crate::i18n::tr("Active"));
                        s_b1_hold.remove_css_class("warning");
                        s_b1_hold.add_css_class("success");
                        let t = adw::Toast::new(&crate::i18n::tr("Call resumed"));
                        t.set_timeout(2);
                        toast_hold.add_toast(t);
                    }
                }
            }
        });

        // Modal-freies Weiterleiten / Abgeben (über bestehende Views per Drag & Drop oder Rechtsklick)
        let eb_trans = engine_bridge.clone();
        let bridge_trans = bridge.clone();
        let act_trans = active_call.clone();
        let hld_trans = held_call.clone();
        let in_cid_trans = current_incoming_call_id.clone();
        let cb_trans = call_banner.clone();
        let app_trans = app.clone();
        let ring_trans = ringtone_player.clone();
        let toast_trans = toast_overlay.clone();
        let finish_ui_trans = finish_active_call_ui.clone();
        let s_held_box_trans = sidebar_comps.docked_held_box.clone();
        let i_banner_trans = incall_comps.held_banner.clone();
        let s_b2_trans = sidebar_comps.line2_badge.clone();
        let s_i2_trans = sidebar_comps.line2_icon.clone();
        let s_vol2_trans = sidebar_comps.line2_volume_scale.clone();

        let clear_held_line_ui = {
            let hld_ref = held_call.clone();
            let s_hb = s_held_box_trans.clone();
            let i_hb = i_banner_trans.clone();
            let sb2 = s_b2_trans.clone();
            let si2 = s_i2_trans.clone();
            let sv2 = s_vol2_trans.clone();
            Rc::new(move || {
                hld_ref.borrow_mut().take();
                s_hb.set_visible(false);
                i_hb.set_visible(false);
                sb2.set_text(&crate::i18n::tr("Free"));
                si2.set_icon_name(Some("call-start-symbolic"));
                sv2.set_visible(false);
            })
        };

        let clear_held_for_trans = clear_held_line_ui.clone();
        let transfer_call: Rc<dyn Fn(String, String)> = Rc::new(move |target_name: String, target_number: String| {
            let dropped_tag = crate::ui::take_last_dropped_call_tag();
            let target: String = target_number
                .trim()
                .chars()
                .filter(|ch| !matches!(ch, ' ' | '-' | '/' | '(' | ')' | '.'))
                .collect();
            if target.is_empty() {
                return;
            }
            let disp = if !target_name.is_empty() && target_name != target {
                format!("{} ({})", target_name, target)
            } else {
                target.clone()
            };

            let cur_st = bridge_trans.load_or_init_data();
            let is_passive = cur_st.passive_mode;

            // 0. Gehaltene Leitung 2 wurde per Drag & Drop auf Kontakt gezogen -> gehaltenes Gespräch übergeben
            if dropped_tag == "CALL_TRANSFER_HELD" {
                let held_cid = hld_trans.borrow().as_ref().map(|h| h.id.clone());
                if let Some(hcid) = held_cid {
                    if is_passive {
                        clear_held_for_trans();
                        let t = adw::Toast::new(&crate::i18n::trf(
                            "Held line 2 transferred to {name}",
                            &[("name", &disp)],
                        ));
                        t.set_timeout(3);
                        toast_trans.add_toast(t);
                    } else {
                        let eb = eb_trans.clone();
                        let t_toast = toast_trans.clone();
                        let d_name = disp.clone();
                        let t_num = target.clone();
                        let clr_h = clear_held_for_trans.clone();
                        glib::MainContext::default().spawn_local(async move {
                            match eb.transfer(&hcid, &t_num).await {
                                Ok(_) => {
                                    clr_h();
                                    let t = adw::Toast::new(&crate::i18n::trf(
                                        "Held line 2 transferred to {name}",
                                        &[("name", &d_name)],
                                    ));
                                    t.set_timeout(3);
                                    t_toast.add_toast(t);
                                }
                                Err(e) => {
                                    let err_s = e.to_string();
                                    let t = adw::Toast::new(&crate::i18n::trf(
                                        "Transfer failed: {error}",
                                        &[("error", &err_s)],
                                    ));
                                    t.set_timeout(3);
                                    t_toast.add_toast(t);
                                }
                            }
                        });
                    }
                    return;
                }
            }

            // 1. Eingehender Anruf klingelt gerade -> Sofort umleiten (Deflect / Blind Transfer)
            let incoming_cid = in_cid_trans.borrow().clone();
            if !incoming_cid.is_empty() && act_trans.borrow().is_none() {
                ring_trans.stop();
                cb_trans.set_visible(false);
                app_trans.withdraw_notification("incoming-call");
                in_cid_trans.borrow_mut().clear();

                if is_passive {
                    let t = adw::Toast::new(&crate::i18n::trf(
                        "Incoming call deflected to {name}",
                        &[("name", &disp)],
                    ));
                    t.set_timeout(3);
                    toast_trans.add_toast(t);
                } else {
                    let eb = eb_trans.clone();
                    let t_toast = toast_trans.clone();
                    let d_name = disp.clone();
                    let t_num = target.clone();
                    glib::MainContext::default().spawn_local(async move {
                        match eb.transfer(&incoming_cid, &t_num).await {
                            Ok(_) => {
                                let t = adw::Toast::new(&crate::i18n::trf(
                                    "Incoming call deflected to {name}",
                                    &[("name", &d_name)],
                                ));
                                t.set_timeout(3);
                                t_toast.add_toast(t);
                            }
                            Err(e) => {
                                let err_s = e.to_string();
                                let t = adw::Toast::new(&crate::i18n::trf(
                                    "Transfer failed: {error}",
                                    &[("error", &err_s)],
                                ));
                                t.set_timeout(3);
                                t_toast.add_toast(t);
                            }
                        }
                    });
                }
                return;
            }

            // 2. Aktives Gespräch läuft -> Direkt übergeben (Blind Transfer RFC 3515)
            let active_cid = act_trans.borrow().as_ref().map(|c| c.id.clone());
            if let Some(cid) = active_cid {
                if is_passive {
                    let t = adw::Toast::new(&crate::i18n::trf(
                        "Call transferred to {name}",
                        &[("name", &disp)],
                    ));
                    t.set_timeout(3);
                    toast_trans.add_toast(t);
                    finish_ui_trans();
                } else {
                    let eb = eb_trans.clone();
                    let t_toast = toast_trans.clone();
                    let d_name = disp.clone();
                    let t_num = target.clone();
                    let fin_ui = finish_ui_trans.clone();
                    glib::MainContext::default().spawn_local(async move {
                        match eb.transfer(&cid, &t_num).await {
                            Ok(_) => {
                                let t = adw::Toast::new(&crate::i18n::trf(
                                    "Call transferred to {name}",
                                    &[("name", &d_name)],
                                ));
                                t.set_timeout(3);
                                t_toast.add_toast(t);
                                fin_ui();
                            }
                            Err(e) => {
                                let err_s = e.to_string();
                                let t = adw::Toast::new(&crate::i18n::trf(
                                    "Transfer failed: {error}",
                                    &[("error", &err_s)],
                                ));
                                t.set_timeout(3);
                                t_toast.add_toast(t);
                            }
                        }
                    });
                }
            } else {
                let t = adw::Toast::new(&crate::i18n::trf(
                    "No active call to transfer to {name}",
                    &[("name", &disp)],
                ));
                t.set_timeout(2);
                toast_trans.add_toast(t);
            }
        });
        *transfer_call_holder.borrow_mut() = Some(transfer_call.clone());

        // Zusammenschalten bei Rückfrage / Makeln (Echter RFC 3891 Attended Transfer: Leitung 2 ↔ Leitung 1)
        let connect_two_calls = {
            let act_c = active_call.clone();
            let hld_c = held_call.clone();
            let eb_att = engine_bridge.clone();
            let bridge_att = bridge.clone();
            let toast_att = toast_overlay.clone();
            let clr_held = clear_held_line_ui.clone();
            let fin_act = finish_active_call_ui.clone();
            Rc::new(move || {
                let pair = {
                    let act = act_c.borrow();
                    let hld = hld_c.borrow();
                    if let (Some(cur_act), Some(cur_hld)) = (act.as_ref(), hld.as_ref()) {
                        Some((
                            cur_hld.id.clone(),
                            cur_hld.caller_name.clone(),
                            cur_act.id.clone(),
                            cur_act.caller_name.clone(),
                        ))
                    } else {
                        None
                    }
                };
                if let Some((held_id, held_name, act_id, act_name)) = pair {
                    let is_passive = bridge_att.load_or_init_data().passive_mode;
                    if is_passive {
                        clr_held();
                        fin_act();
                        let t = adw::Toast::new(&crate::i18n::trf(
                            "Calls merged: {first} ↔ {second}",
                            &[("first", &held_name), ("second", &act_name)],
                        ));
                        t.set_timeout(3);
                        toast_att.add_toast(t);
                    } else {
                        let eb = eb_att.clone();
                        let t_toast = toast_att.clone();
                        let clr_h = clr_held.clone();
                        let fin_a = fin_act.clone();
                        glib::MainContext::default().spawn_local(async move {
                            match eb.attended_transfer(&held_id, &act_id).await {
                                Ok(_) => {
                                    clr_h();
                                    fin_a();
                                    let t = adw::Toast::new(&crate::i18n::trf(
                                        "Calls merged: {first} ↔ {second}",
                                        &[("first", &held_name), ("second", &act_name)],
                                    ));
                                    t.set_timeout(3);
                                    t_toast.add_toast(t);
                                }
                                Err(e) => {
                                    let err_s = e.to_string();
                                    let t = adw::Toast::new(&crate::i18n::trf(
                                        "Merge failed: {error}",
                                        &[("error", &err_s)],
                                    ));
                                    t.set_timeout(3);
                                    t_toast.add_toast(t);
                                }
                            }
                        });
                    }
                }
            })
        };

        let conn_held1 = connect_two_calls.clone();
        incall_comps.held_transfer_button.connect_clicked(move |_| {
            conn_held1();
        });

        let conn_held2 = connect_two_calls.clone();
        sidebar_comps.docked_transfer_held_btn.connect_clicked(move |_| {
            conn_held2();
        });

        // Leitung 1 auf Leitung 2 (oder Leitung 2 auf Leitung 1) ziehen -> beide Gespräche verbinden
        let conn_drop1 = connect_two_calls.clone();
        crate::ui::attach_line_connect_drop_target(
            &sidebar_comps.line1_chip,
            "CALL_TRANSFER_HELD",
            move || conn_drop1(),
        );
        let conn_drop2 = connect_two_calls.clone();
        crate::ui::attach_line_connect_drop_target(
            &sidebar_comps.line2_chip,
            "CALL_TRANSFER",
            move || conn_drop2(),
        );

        // Klick auf Weiterleiten-Button in InCall-Ansicht: Kein Modal, sondern bestehendes Telefonbuch nutzen
        let hld_check = held_call.clone();
        let conn_held3 = connect_two_calls.clone();
        let on_sel_trans = on_select.clone();
        let c_sb_trans = contacts_search_bar_holder.clone();
        let toast_btn_trans = toast_overlay.clone();
        incall_comps.transfer_button.connect_clicked(move |_| {
            if hld_check.borrow().is_some() {
                conn_held3();
            } else {
                on_sel_trans("contacts");
                if let Some(c_sbar) = c_sb_trans.borrow().as_ref() {
                    c_sbar.set_search_mode(true);
                }
                let t = adw::Toast::new(&crate::i18n::tr("Drag line onto a contact or right-click → Transfer Call"));
                t.set_timeout(3);
                toast_btn_trans.add_toast(t);
            }
        });

        // Wire Answer and Reject Buttons on Call Banner
        let eb_ans = engine_bridge.clone();
        let eb_rej = engine_bridge.clone();
        let in_cid_ans = current_incoming_call_id.clone();
        let in_name_ans = current_incoming_caller_name.clone();
        let in_num_ans = current_caller_number.clone();
        let cb_ans = call_banner.clone();
        let app_ans = app.clone();
        let toast_ans = toast_overlay.clone();
        let act_ans = active_call.clone();
        let hld_ans = held_call.clone();
        let s_caller_ans = sidebar_comps.docked_caller_label.clone();
        let s_timer_ans = sidebar_comps.docked_timer_label.clone();
        let s_dock_ans = sidebar_comps.docked_call_box.clone();
        let s_chip1_ans = sidebar_comps.line1_chip.clone();
        let s_b1_ans = sidebar_comps.line1_badge.clone();
        let s_i1_ans = sidebar_comps.line1_icon.clone();
        let s_vol1_ans = sidebar_comps.line1_volume_scale.clone();
        let s_b2_ans = sidebar_comps.line2_badge.clone();
        let s_i2_ans = sidebar_comps.line2_icon.clone();
        let s_vol2_ans = sidebar_comps.line2_volume_scale.clone();
        let s_held_box_ans = sidebar_comps.docked_held_box.clone();
        let s_held_lbl_ans = sidebar_comps.docked_held_label.clone();
        let s_mute_ans = sidebar_comps.docked_mute_btn.clone();
        let i_caller_ans = incall_comps.caller_name_label.clone();
        let i_ext_ans = incall_comps.caller_ext_label.clone();
        let i_timer_ans = incall_comps.timer_label.clone();
        let i_banner_ans = incall_comps.held_banner.clone();
        let i_held_lbl_ans = incall_comps.held_pill_label.clone();
        let i_mute_ans = incall_comps.mute_button.clone();
        let i_hold_ans = incall_comps.hold_button.clone();
        let on_sel_ans = on_select.clone();
        let win_ans = window.clone();
        let ring_ans = ringtone_player.clone();
        let hang_ans = do_hangup.clone();

        answer_btn.connect_clicked(move |_| {
            ring_ans.stop();
            let cid = in_cid_ans.borrow().clone();
            // Angenommener Ruf ist kein "klingelnder" Ruf mehr (verhindert falsche
            // "Verpasster Anruf"-Meldung, wenn das Gespräch später endet)
            in_cid_ans.borrow_mut().clear();
            let name = in_name_ans.borrow().clone();
            let num = in_num_ans.borrow().clone();
            let eb = eb_ans.clone();
            let toast_o = toast_ans.clone();
            cb_ans.set_visible(false);
            app_ans.withdraw_notification("incoming-call");

            let disp_name = if !name.is_empty() { name.clone() } else { num.clone() };

            // Wenn bereits ein Gespräch auf Leitung 1 aktiv ist -> auf Leitung 2 parken (Halten)
            let mut prev_to_hold: Option<String> = None;
            {
                let mut act = act_ans.borrow_mut();
                let mut hld = hld_ans.borrow_mut();
                if let Some(prev) = act.take() {
                    if !prev.is_held {
                        prev_to_hold = Some(prev.id.clone());
                    }
                    *hld = Some(HeldCall {
                        id: prev.id,
                        caller_name: prev.caller_name.clone(),
                        extension: prev.extension.clone(),
                        duration_secs: prev.duration_secs,
                    });
                    s_held_lbl_ans.set_text(&format!("Halten: {} ({})", prev.caller_name, prev.extension));
                    s_held_box_ans.set_visible(true);
                    s_b2_ans.set_text("Halten");
                    s_i2_ans.set_icon_name(Some("audio-volume-high-symbolic"));
                    s_vol2_ans.set_visible(true);
                    i_held_lbl_ans.set_text(&format!(
                        "{} ({}) • In Warteschleife {:02}:{:02}",
                        prev.caller_name,
                        prev.extension,
                        prev.duration_secs / 60,
                        prev.duration_secs % 60
                    ));
                    i_banner_ans.set_visible(true);
                }
                *act = Some(ActiveCall {
                    id: if !cid.is_empty() { cid.clone() } else { "call-in".into() },
                    caller_name: disp_name.clone(),
                    extension: num.clone(),
                    full_number: num.clone(),
                    duration_secs: 0,
                    is_muted: false,
                    is_held: false,
                });
            }

            // Reset mute & hold buttons for newly answered call
            s_mute_ans.remove_css_class("call-btn-active-mute");
            s_mute_ans.set_icon_name("audio-input-microphone-symbolic");
            s_mute_ans.set_tooltip_text(Some(&crate::i18n::tr("Mute")));
            i_mute_ans.remove_css_class("call-btn-active-mute");
            i_mute_ans.set_icon_name("audio-input-microphone-symbolic");
            i_mute_ans.set_tooltip_text(Some(&crate::i18n::tr("Mute")));
            i_hold_ans.remove_css_class("suggested-action");
            i_hold_ans.set_icon_name("media-playback-pause-symbolic");
            i_hold_ans.set_tooltip_text(Some(&crate::i18n::tr("Hold")));

            s_caller_ans.set_text(&format_caller_display(&disp_name, &num));
            s_timer_ans.set_text("00:00");
            s_dock_ans.set_visible(true);
            s_chip1_ans.add_css_class("line-chip-active");
            s_b1_ans.set_text(&crate::i18n::tr("Active"));
            s_b1_ans.remove_css_class("dim-label");
            s_b1_ans.remove_css_class("warning");
            s_b1_ans.add_css_class("success");
            s_i1_ans.set_icon_name(Some("audio-volume-high-symbolic"));
            s_i1_ans.remove_css_class("dim-label");
            s_i1_ans.add_css_class("speaker-green-icon");
            s_vol1_ans.set_visible(true);

            i_caller_ans.set_text(&disp_name);
            i_ext_ans.set_text(&crate::i18n::trf("{number} • Line 1 • HD Voice", &[("number", &num)]));
            i_timer_ans.set_text("00:00");

            on_sel_ans("incall");
            win_ans.present();

            if !cid.is_empty() {
                let hang_fail = hang_ans.clone();
                glib::MainContext::default().spawn_local(async move {
                    if let Some(pid) = prev_to_hold {
                        let _ = eb.hold(&pid).await;
                    }
                    eb.set_mute(false).await;
                    if let Err(e) = eb.answer(&cid).await {
                        let t = adw::Toast::new(&format!("Annehmen fehlgeschlagen: {}", e));
                        t.set_timeout(3);
                        toast_o.add_toast(t);
                        hang_fail();
                    }
                });
            }
        });

        let in_cid_rej = current_incoming_call_id.clone();
        let in_name_rej = current_incoming_caller_name.clone();
        let cb_rej = call_banner.clone();
        let app_rej = app.clone();
        let toast_rej = toast_overlay.clone();
        let ring_rej = ringtone_player.clone();

        reject_btn.connect_clicked(move |_| {
            ring_rej.stop();
            let cid = in_cid_rej.borrow().clone();
            in_cid_rej.borrow_mut().clear();
            let name = in_name_rej.borrow().clone();
            let eb = eb_rej.clone();
            let toast_o = toast_rej.clone();
            cb_rej.set_visible(false);
            app_rej.withdraw_notification("incoming-call");

            let t = adw::Toast::new(&crate::i18n::trf("Call from {name} declined", &[("name", &name)]));
            t.set_timeout(2);
            toast_o.add_toast(t);

            if !cid.is_empty() {
                glib::MainContext::default().spawn_local(async move {
                    let _ = eb.reject(&cid, 486).await;
                });
            }
        });

        // Desktop signaling: GAction handlers for GNOME Shell interactive notifications
        let action_answer = gtk::gio::SimpleAction::new("answer-incoming-call", None);
        let ans_btn_act = answer_btn.clone();
        let win_act = window.clone();
        action_answer.connect_activate(move |_, _| {
            win_act.set_visible(true);
            win_act.present();
            ans_btn_act.emit_clicked();
        });
        app.add_action(&action_answer);

        let action_reject = gtk::gio::SimpleAction::new("reject-incoming-call", None);
        let rej_btn_act = reject_btn.clone();
        action_reject.connect_activate(move |_, _| {
            rej_btn_act.emit_clicked();
        });
        app.add_action(&action_reject);

        let action_show = gtk::gio::SimpleAction::new("show-window", None);
        let win_act2 = window.clone();
        action_show.connect_activate(move |_, _| {
            win_act2.set_visible(true);
            win_act2.present();
        });
        app.add_action(&action_show);

        // Wire Trigger Call Action
        let s_dock_call = sidebar_comps.docked_call_box.clone();
        let s_caller_trig = sidebar_comps.docked_caller_label.clone();
        let s_timer_trig = sidebar_comps.docked_timer_label.clone();
        let s_chip1_trig = sidebar_comps.line1_chip.clone();
        let s_b1_trig = sidebar_comps.line1_badge.clone();
        let s_i1_trig = sidebar_comps.line1_icon.clone();
        let s_mute_trig = sidebar_comps.docked_mute_btn.clone();
        let s_vol1_trig = sidebar_comps.line1_volume_scale.clone();
        let s_vol2_trig = sidebar_comps.line2_volume_scale.clone();
        let s_b2_trig = sidebar_comps.line2_badge.clone();
        let s_i2_trig = sidebar_comps.line2_icon.clone();
        let s_held_box_trig = sidebar_comps.docked_held_box.clone();
        let s_held_lbl_trig = sidebar_comps.docked_held_label.clone();

        let i_caller_trig = incall_comps.caller_name_label.clone();
        let i_ext_trig = incall_comps.caller_ext_label.clone();
        let i_timer_trig = incall_comps.timer_label.clone();
        let i_banner_trig = incall_comps.held_banner.clone();
        let i_held_lbl_trig = incall_comps.held_pill_label.clone();
        let i_mute_trig = incall_comps.mute_button.clone();

        let is_backend_ready = Rc::new(RefCell::new(false));
        let is_backend_ready_call = is_backend_ready.clone();
        let cti_trig = current_telephony_mode.clone();
        let toast_trig = toast_overlay.clone();
        let eb_trig = engine_bridge.clone();
        let bridge_trig = bridge.clone();
        let active_c_trig_callid = active_call.clone();
        let on_sel_trig = on_select.clone();
        let h_trig = tray_handle.clone();
        let do_hangup_trig = do_hangup.clone();

        let trigger_call = move |name: String, raw_number: String| {
            let number: String = raw_number
                .trim()
                .chars()
                .filter(|ch| !matches!(ch, ' ' | '-' | '/' | '(' | ')' | '.'))
                .collect();
            if number.is_empty() {
                return;
            }
            if !*is_backend_ready_call.borrow() {
                let toast = adw::Toast::new(&crate::i18n::tr("Connecting to phone server… Please wait a moment."));
                toast.set_timeout(3);
                toast_trig.add_toast(toast);
                return;
            }

            let cur_st = bridge_trig.load_or_init_data();
            let is_passive = cur_st.passive_mode;

            // Call-ID vorab erzeugen: alle Engine-Events (Klingeln, Verbunden, Beendet)
            // tragen dieselbe ID, Auflegen während des Klingelns bricht den Ruf ab (CANCEL).
            let call_id = crate::engine_bridge::EngineBridge::new_call_id();

            // Falls bereits ein aktives Gespräch auf Leitung 1 läuft, dieses im SIP-Stack halten (Rückfrage / Zweitanruf)
            let prev_to_hold = active_c_trig
                .borrow()
                .as_ref()
                .filter(|p| !p.is_held)
                .map(|p| p.id.clone());

            if is_passive {
                let toast = adw::Toast::new(&format!(
                    "Passiver Modus aktiv: Anruf an {} ({}) simuliert (kein PBX-Netzwerkverkehr)",
                    name, number
                ));
                toast.set_timeout(3);
                toast_trig.add_toast(toast);
            } else {
                let eb = eb_trig.clone();
                let num_dial = number.clone();
                let cid_dial = call_id.clone();
                let toast_dial = toast_trig.clone();
                let hang_dial = do_hangup_trig.clone();
                let act_dial = active_c_trig_callid.clone();
                glib::MainContext::default().spawn_local(async move {
                    if let Some(pid) = prev_to_hold {
                        let _ = eb.hold(&pid).await;
                    }
                    eb.set_mute(false).await;
                    if let Err(e) = eb.dial(&cid_dial, &num_dial).await {
                        let toast = adw::Toast::new(&format!("Rufaufbau fehlgeschlagen: {}", e));
                        toast.set_timeout(4);
                        toast_dial.add_toast(toast);
                        let still_active = act_dial
                            .borrow()
                            .as_ref()
                            .map(|c| c.id == cid_dial)
                            .unwrap_or(false);
                        if still_active {
                            hang_dial();
                        }
                    }
                });
            }

            let mut act = active_c_trig.borrow_mut();
            let mut hld = held_c_trig.borrow_mut();

            if act.is_some() {
                if let Some(prev) = act.take() {
                    *hld = Some(HeldCall {
                        id: prev.id,
                        caller_name: prev.caller_name.clone(),
                        extension: prev.extension.clone(),
                        duration_secs: prev.duration_secs,
                    });
                    s_held_lbl_trig.set_text(&format!("Halten: {} ({})", prev.caller_name, prev.extension));
                    s_held_box_trig.set_visible(true);

                    // Previous call moves to Leitung 2 (on hold)
                    s_b2_trig.set_text("Halten");
                    s_i2_trig.set_icon_name(Some("audio-volume-high-symbolic"));
                    s_vol2_trig.set_visible(true);

                    i_held_lbl_trig.set_text(&format!(
                        "{} ({}) • In Warteschleife {:02}:{:02}",
                        prev.caller_name,
                        prev.extension,
                        prev.duration_secs / 60,
                        prev.duration_secs % 60
                    ));
                    i_banner_trig.set_visible(true);
                }
            } else {
                s_held_box_trig.set_visible(false);
                i_banner_trig.set_visible(false);
                s_b2_trig.set_text(&crate::i18n::tr("Free"));
                s_i2_trig.set_icon_name(Some("call-start-symbolic"));
                s_vol2_trig.set_visible(false);
            }

            *act = Some(ActiveCall {
                id: call_id.clone(),
                caller_name: name.clone(),
                extension: number.clone(),
                full_number: number.clone(),
                duration_secs: 0,
                is_muted: false,
                is_held: false,
            });

            let is_cti = cti_trig.borrow().is_cti();
            let mode_tag = if is_cti {
                crate::i18n::tr("CTI (Desk Phone)")
            } else {
                crate::i18n::tr("Dialing…")
            };

            // Update Sidebar Dock & show it
            s_caller_trig.set_text(&format_caller_display(&name, &number));
            s_timer_trig.set_text("00:00");
            s_dock_call.set_visible(true);
            s_chip1_trig.add_css_class("line-chip-active");
            s_b1_trig.set_text(&crate::i18n::tr("Active"));
            s_b1_trig.remove_css_class("dim-label");
            s_b1_trig.add_css_class("success");
            s_i1_trig.set_icon_name(Some("audio-volume-high-symbolic"));
            s_i1_trig.remove_css_class("dim-label");
            s_i1_trig.add_css_class("speaker-green-icon");
            s_vol1_trig.set_visible(true);

            // Reset mute indicators
            s_mute_trig.remove_css_class("call-btn-active-mute");
            s_mute_trig.set_icon_name("audio-input-microphone-symbolic");
            s_mute_trig.set_tooltip_text(Some(&crate::i18n::tr("Mute")));
            i_mute_trig.remove_css_class("call-btn-active-mute");
            i_mute_trig.set_icon_name("audio-input-microphone-symbolic");
            i_mute_trig.set_tooltip_text(Some(&crate::i18n::tr("Mute")));

            // Update InCall View
            i_caller_trig.set_text(&name);
            i_ext_trig.set_text(&crate::i18n::trf("{number} • Line 1 • {status}", &[("number", &number), ("status", &mode_tag)]));
            i_timer_trig.set_text("00:00");
            on_sel_trig("incall");

            if let Some(ref h) = *h_trig.borrow() {
                let call_desc = format!("{} ({})", name, number);
                h.update(move |t| {
                    t.call_info = Some(call_desc);
                    t.has_incoming_call = false;
                    t.presence = PresenceState::Busy;
                });
            }
        };

        // Connect dedicated dialer view & sidebar quick-dial callbacks to trigger_call
        let tc_shared = Rc::new(trigger_call.clone());
        *tc_dialer_holder.borrow_mut() = Some(tc_shared.clone());
        *trigger_call_holder.borrow_mut() = Some(tc_shared);

        // Action when clicking/activating an entry in Journal or Contacts:
        // 1) Pre-fill sidebar dialer entry and focus it (safe: no accidental instant call)
        // 2) Copy number to system clipboard
        // 3) Show subtle GNOME Adwaita toast notification
        let s_entry_sel = sidebar_comps.search_entry.clone();
        let toast_sel = toast_overlay.clone();
        let auto_pop_num = auto_popover_holder.clone();
        let on_select_number = move |number: String| {
            if let Some(p) = auto_pop_num.borrow().as_ref() {
                p.popdown();
            }
            s_entry_sel.set_text(&number);
            s_entry_sel.grab_focus();
            s_entry_sel.set_position(-1);

            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&number);
            }

            let toast = adw::Toast::new(&format!("Nummer {} in Zwischenablage kopiert", number));
            toast.set_timeout(2);
            toast_sel.add_toast(toast);
        };
        *select_number_holder.borrow_mut() = Some(Rc::new(on_select_number.clone()));

        // 5. Build Content Pages
        // 5. Standalone Ephemeral Chat-Fenster (analog Swyx)
        let win_chat_parent = window.clone();
        let trig_for_chat = trigger_call.clone();
        let eb_for_chat = engine_bridge.clone();
        let open_chat_windows: Rc<RefCell<std::collections::HashMap<String, ChatWindowHandle>>> =
            Rc::new(RefCell::new(std::collections::HashMap::new()));

        let chat_wins_fn = open_chat_windows.clone();
        let parent_fn = win_chat_parent.clone();
        let trig_fn = trig_for_chat.clone();
        let eb_fn = eb_for_chat.clone();
        let launch_chat = Rc::new(move |contact: crate::models::Contact| {
            let ext_key = if !contact.extension.trim().is_empty() {
                contact.extension.trim().to_string()
            } else {
                contact.phone.trim().to_string()
            };
            let mut wins = chat_wins_fn.borrow_mut();
            if let Some(existing_handle) = wins.get(&ext_key) {
                if existing_handle.window.is_visible() {
                    existing_handle.window.present();
                    return;
                }
            }

            let new_handle = open_chat_window(
                &parent_fn,
                &contact,
                (*eb_fn).clone(),
                trig_fn.clone(),
            );

            let wins_cleanup = chat_wins_fn.clone();
            let ext_cleanup = ext_key.clone();
            new_handle.window.connect_destroy(move |_| {
                wins_cleanup.borrow_mut().remove(&ext_cleanup);
            });

            wins.insert(ext_key, new_handle);
        });

        let lc_for_sidebar = launch_chat.clone();
        let contacts_for_sidebar_chat = contacts_list.clone();
        *open_chat_holder.borrow_mut() = Some(Rc::new(move |name: String, number: String| {
            let contact = contacts_for_sidebar_chat
                .borrow()
                .iter()
                .find(|c| c.extension == number || c.phone == number || c.name == name)
                .cloned()
                .unwrap_or_else(|| crate::models::Contact {
                    id: format!("ext-{}", number),
                    name: if name.is_empty() { number.clone() } else { name },
                    extension: number.clone(),
                    department: "Kurzwahl-Kontakt".into(),
                    presence: crate::models::PresenceState::Available,
                    phone: number,
                    avatar_file_id: None,
                });
            lc_for_sidebar(contact);
        }));

        // 5.1 Page: Rufjournal (mit Rückruf- und Chat-Button)
        let launch_for_journal = launch_chat.clone();
        let contacts_for_journal = contacts_list.clone();

        let save_speed_dials = {
            let br_sd = bridge.clone();
            let sd_ref = speed_dials.clone();
            Rc::new(move || {
                let mut st = br_sd.load_or_init_data();
                st.speed_dials = sd_ref.borrow().clone();
                br_sd.save_data(&st);
            })
        };

        let c_rel_cb = contacts_reload_holder.clone();
        let s_hk_rel1 = sidebar_reload_hotkeys_holder.clone();
        let save_sd1 = save_speed_dials.clone();
        let on_fav_in_journal = move || {
            save_sd1();
            if let Some(r) = c_rel_cb.borrow().as_ref() {
                r();
            }
            if let Some(r) = s_hk_rel1.borrow().as_ref() {
                r();
            }
        };

        let bridge_seen = bridge.clone();
        let on_seen_changed = move |seen_ids: Vec<String>| {
            let mut st = bridge_seen.load_or_init_data();
            st.seen_call_ids = seen_ids;
            bridge_seen.save_data(&st);
        };

        let tc_j = transfer_call.clone();
        let journal_view_comps = create_journal_view(
            journal_entries.clone(),
            contacts_list.clone(),
            speed_dials.clone(),
            toast_overlay.clone(),
            store.seen_call_ids.clone(),
            trigger_call.clone(),
            move |name, num| tc_j(name, num),
            on_select_number.clone(),
            move |name: String, number: String| {
                let contact = contacts_for_journal
                    .borrow()
                    .iter()
                    .find(|c| c.extension == number || c.phone == number || c.name == name)
                    .cloned()
                    .unwrap_or_else(|| crate::models::Contact {
                        id: format!("ext-{}", number),
                        name: if name.is_empty() { number.clone() } else { name },
                        extension: number.clone(),
                        department: "Rufjournal-Kontakt".into(),
                        presence: crate::models::PresenceState::Available,
                        phone: number,
                        avatar_file_id: None,
                    });
                launch_for_journal(contact);
            },
            on_fav_in_journal,
            on_seen_changed,
        );
        *journal_mark_seen_holder.borrow_mut() = Some(journal_view_comps.mark_all_seen.clone());
        *journal_search_bar_holder.borrow_mut() = Some(journal_view_comps.search_bar.clone());
        *journal_reload_holder.borrow_mut() = Some(journal_view_comps.reload.clone());
        journal_view_comps.search_bar.set_key_capture_widget(Some(&window));
        content_stack.add_named(&journal_view_comps.container, Some("journal"));

        // 5.2 Page: Telefonbuch
        let launch_for_contacts = launch_chat.clone();
        let j_rel_cb = journal_reload_holder.clone();
        let s_hk_rel2 = sidebar_reload_hotkeys_holder.clone();
        let save_sd2 = save_speed_dials.clone();
        let on_fav_in_contacts = move || {
            save_sd2();
            if let Some(r) = j_rel_cb.borrow().as_ref() {
                r();
            }
            if let Some(r) = s_hk_rel2.borrow().as_ref() {
                r();
            }
        };

        let j_rel_sb = journal_reload_holder.clone();
        let c_rel_sb = contacts_reload_holder.clone();
        let save_sd3 = save_speed_dials.clone();
        *fav_changed_holder.borrow_mut() = Some(Rc::new(move || {
            save_sd3();
            if let Some(r) = j_rel_sb.borrow().as_ref() {
                r();
            }
            if let Some(r) = c_rel_sb.borrow().as_ref() {
                r();
            }
        }));

        let tc_c = transfer_call.clone();
        let contacts_view_comps = create_contacts_view(
            contacts_list.clone(),
            speed_dials.clone(),
            toast_overlay.clone(),
            trigger_call.clone(),
            move |name, num| tc_c(name, num),
            on_select_number,
            move |contact| {
                launch_for_contacts(contact);
            },
            on_fav_in_contacts,
        );
        *contacts_search_bar_holder.borrow_mut() = Some(contacts_view_comps.search_bar.clone());
        *contacts_reload_holder.borrow_mut() = Some(contacts_view_comps.reload.clone());
        content_stack.add_named(&contacts_view_comps.container, Some("contacts"));

        // Wire Search Button toggle to active search bar
        let cur_page_click = cur_page.clone();
        let j_sb_click = journal_search_bar_holder.clone();
        let c_sb_click = contacts_search_bar_holder.clone();

        search_button.connect_toggled(move |btn| {
            let page = cur_page_click.borrow().clone();
            let is_act = btn.is_active();
            if page == "journal" {
                if let Some(sbar) = j_sb_click.borrow().as_ref() {
                    if sbar.is_search_mode() != is_act {
                        sbar.set_search_mode(is_act);
                    }
                }
            } else if page == "contacts" {
                if let Some(sbar) = c_sb_click.borrow().as_ref() {
                    if sbar.is_search_mode() != is_act {
                        sbar.set_search_mode(is_act);
                    }
                }
            }
        });

        // Sync Search Bar search-mode changes back to header search button
        let sb_btn_sync1 = search_button.clone();
        let cp_sync1 = cur_page.clone();
        journal_view_comps.search_bar.connect_search_mode_enabled_notify(move |sb| {
            if *cp_sync1.borrow() == "journal" {
                if sb_btn_sync1.is_active() != sb.is_search_mode() {
                    sb_btn_sync1.set_active(sb.is_search_mode());
                }
            }
        });

        let sb_btn_sync2 = search_button.clone();
        let cp_sync2 = cur_page.clone();
        contacts_view_comps.search_bar.connect_search_mode_enabled_notify(move |sb| {
            if *cp_sync2.borrow() == "contacts" {
                if sb_btn_sync2.is_active() != sb.is_search_mode() {
                    sb_btn_sync2.set_active(sb.is_search_mode());
                }
            }
        });

        // 5.3 Page: Voice Box (Interaktive Swyx Voicemail & SIP MWI Ansicht)
        let vb_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .build();
        let vb_clamp = adw::Clamp::builder()
            .maximum_size(560)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(16)
            .margin_end(16)
            .build();

        let voicebox_view = gtk::Box::new(gtk::Orientation::Vertical, 16);
        voicebox_view.set_valign(gtk::Align::Center);
        voicebox_view.set_halign(gtk::Align::Fill);

        let vb_status = adw::StatusPage::builder()
            .icon_name("media-tape-symbolic")
            .title(&crate::i18n::tr("No New Voicemails"))
            .description(&crate::i18n::tr("Your voicemail inbox is currently empty."))
            .build();

        let vb_call_btn = gtk::Button::builder()
            .margin_top(4)
            .halign(gtk::Align::Center)
            .css_classes(["suggested-action", "pill"])
            .tooltip_text(&crate::i18n::tr("Call voicemail to listen to messages"))
            .build();
        let vb_btn_content = adw::ButtonContent::builder()
            .icon_name("call-start-symbolic")
            .label(&crate::i18n::tr("Call Voicemail"))
            .build();
        vb_call_btn.set_child(Some(&vb_btn_content));

        let mwi_account_holder: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let mwi_acc_click = mwi_account_holder.clone();
        let bridge_vb_call = bridge.clone();
        let tc_vb = trigger_call.clone();
        vb_call_btn.connect_clicked(move |_| {
            let st = bridge_vb_call.load_or_init_data();
            let target = if !st.voicemail_number.trim().is_empty() {
                st.voicemail_number.trim().to_string()
            } else if let Some(acc) = mwi_acc_click.borrow().clone().filter(|s| !s.trim().is_empty()) {
                acc
            } else if !st.extension.trim().is_empty() {
                st.extension.trim().to_string()
            } else {
                "VoiceBox".to_string()
            };
            tc_vb("Voice Box".to_string(), target);
        });

        vb_status.set_child(Some(&vb_call_btn));
        voicebox_view.append(&vb_status);

        let vb_list_box = gtk::ListBox::new();
        vb_list_box.set_selection_mode(gtk::SelectionMode::None);
        vb_list_box.add_css_class("boxed-list");
        vb_list_box.set_visible(false);
        voicebox_view.append(&vb_list_box);

        vb_clamp.set_child(Some(&voicebox_view));
        vb_scroll.set_child(Some(&vb_clamp));
        content_stack.add_named(&vb_scroll, Some("voicebox"));

        // 5.4 Page: In-Call (Full Flush View)
        content_stack.add_named(&incall_comps.container, Some("incall"));

        // 5.5 Page: Dialer View (Tastenfeld)
        content_stack.add_named(&dialer_comps.container, Some("dialer"));

        content_box.append(&content_stack);

        // 6. NavigationSplitView Pages
        let sidebar_nav_page = adw::NavigationPage::builder()
            .child(&sidebar_comps.container)
            .title("Deskphone")
            .build();

        let content_click = gtk::GestureClick::new();
        content_click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let auto_pop_content = auto_popover_holder.clone();
        content_click.connect_pressed(move |_, _, _, _| {
            if let Some(p) = auto_pop_content.borrow().as_ref() {
                p.popdown();
            }
        });
        content_box.add_controller(content_click);

        split_view.set_sidebar(Some(&sidebar_nav_page));
        split_view.set_content(Some(&content_nav_page));
        split_view.set_hexpand(true);
        split_view.set_vexpand(true);

        // 7. Live 1-Second Timer Tick (aktive Leitung 1 & gehaltene Leitung 2)
        let timer_lbl_i = incall_comps.timer_label.clone();
        let timer_lbl_s = sidebar_comps.docked_timer_label.clone();
        let held_lbl_i_tick = incall_comps.held_pill_label.clone();
        let act_timer = active_call.clone();
        let hld_timer = held_call.clone();

        glib::timeout_add_local(Duration::from_secs(1), move || {
            {
                let mut act = act_timer.borrow_mut();
                if let Some(call) = act.as_mut() {
                    call.duration_secs += 1;
                    let mins = call.duration_secs / 60;
                    let secs = call.duration_secs % 60;
                    let t_str = format!("{:02}:{:02}", mins, secs);
                    timer_lbl_i.set_text(&t_str);
                    timer_lbl_s.set_text(&t_str);
                }
            }
            {
                let mut hld = hld_timer.borrow_mut();
                if let Some(held) = hld.as_mut() {
                    held.duration_secs += 1;
                    held_lbl_i_tick.set_text(&format!(
                        "{} ({}) • In Warteschleife {:02}:{:02}",
                        held.caller_name,
                        held.extension,
                        held.duration_secs / 60,
                        held.duration_secs % 60
                    ));
                }
            }
            glib::ControlFlow::Continue
        });

        toast_overlay.set_child(Some(&split_view));

        // Avoid duplicate call controls: hide the sidebar call card while the in-call page is on screen
        // (in the collapsed/mobile layout only when the content page is actually shown).
        {
            let refresh_dock: Rc<dyn Fn()> = {
                let cs = content_stack.clone();
                let sv = split_view.clone();
                let rev = sidebar_comps.docked_call_revealer.clone();
                Rc::new(move || {
                    let incall_visible = cs.visible_child_name().as_deref() == Some("incall")
                        && (!sv.is_collapsed() || sv.shows_content());
                    rev.set_reveal_child(!incall_visible);
                })
            };
            let r1 = refresh_dock.clone();
            content_stack.connect_visible_child_name_notify(move |_| r1());
            let r2 = refresh_dock.clone();
            split_view.connect_collapsed_notify(move |_| r2());
            let r3 = refresh_dock.clone();
            split_view.connect_show_content_notify(move |_| r3());
            refresh_dock();
        }
        window.set_content(Some(&toast_overlay));

        // 8. Responsive Breakpoints (Adaptive Dual-Pane / Mobile Stack)
        // Breakpoint 1: Below 580px collapse into single-pane mobile navigation so
        // the right content pane always stays >= 365px wide in dual-pane mode.
        let bp_mobile = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 580px").unwrap());
        bp_mobile.add_setter(&split_view, "collapsed", Some(&true.to_value()));
        bp_mobile.add_setter(&sidebar_comps.header_bar, "show-end-title-buttons", Some(&true.to_value()));
        bp_mobile.add_setter(&passive_badge, "visible", Some(&false.to_value()));
        window.add_breakpoint(bp_mobile);

        // Breakpoint 2: Very narrow window (<= 350px, e.g. slim desktop phone strip):
        // switch journal filter pills to crisp symbolic icons with tooltips so nothing ellipsizes.
        let bp_narrow = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 350px").unwrap());
        bp_narrow.add_setter(&split_view, "collapsed", Some(&true.to_value()));
        bp_narrow.add_setter(&sidebar_comps.header_bar, "show-end-title-buttons", Some(&true.to_value()));
        bp_narrow.add_setter(&passive_badge, "visible", Some(&false.to_value()));
        let set_compact_apply = journal_view_comps.set_compact_filters.clone();
        bp_narrow.connect_apply(move |_| set_compact_apply(true));
        let set_compact_unapply = journal_view_comps.set_compact_filters.clone();
        bp_narrow.connect_unapply(move |_| set_compact_unapply(false));
        window.add_breakpoint(bp_narrow);

        if is_demo && std::env::var("DESKPHONE_CAPTURE_VIEW").as_deref() == Ok("journal") {
            split_view.set_show_content(true);
        }

        // Application Window Close Request (unterstützt optionalen Hintergrundmodus)
        let ring_close = ringtone_player.clone();
        let bridge_close = bridge.clone();
        let fq_close = force_quit.clone();
        window.connect_close_request(move |w| {
            if !fq_close.get() && bridge_close.load_or_init_data().background_on_close {
                w.set_visible(false);
                println!("[Deskphone] Fenster ausgeblendet (Hintergrundmodus aktiv).");
                return glib::Propagation::Stop;
            }
            ring_close.stop();
            println!("[Deskphone] Beende native Deskphone-Anwendung...");
            glib::Propagation::Proceed
        });

        let badge_journal = sidebar_comps.journal_badge.clone();

        // Calculate initial missed badge (nur tatsächlich ungesehene verpasste Anrufe zählen)
        let initial_missed = journal_entries
            .borrow()
            .iter()
            .filter(|e| e.direction == crate::models::CallDirection::Missed && !store.seen_call_ids.contains(&e.id))
            .count();
        if initial_missed > 0 {
            badge_journal.set_text(&initial_missed.to_string());
            badge_journal.set_visible(true);
        } else {
            badge_journal.set_visible(false);
        }

        // Nach 2 Sekunden auf dem Rufjournal: Ungesehene Anrufe als gesehen markieren & Badge ausblenden
        let j_timer = journal_view_comps.mark_all_seen.clone();
        let b_timer = badge_journal.clone();
        gtk::glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
            j_timer();
            b_timer.set_visible(false);
        });


        // 9. Connect Native Softphone Engine Bridge & Event Loop (skipped in offline `--demo` or unconfigured mode)
        let (engine_tx, engine_rx) = async_channel::unbounded();
        {
            let eb_rc = engine_bridge.clone();
            let br_rc = bridge.clone();
            let tx_rc = engine_tx.clone();
            let banner_rc = conn_banner.clone();
            let spinner_rc = conn_spinner.clone();
            *reconnect_holder.borrow_mut() = Some(Rc::new(move || {
                if is_demo {
                    return;
                }
                let latest = br_rc.load_or_init_data();
                if latest.server_ip.trim().is_empty() || latest.user_name.trim().is_empty() {
                    return;
                }
                banner_rc.set_button_label(None);
                banner_rc.set_title(&crate::i18n::tr("Connecting to phone system…"));
                banner_rc.add_css_class("conn-banner-connecting");
                banner_rc.set_revealed(true);
                spinner_rc.set_visible(true);
                spinner_rc.start();
                eb_rc.connect(
                    latest.provider,
                    latest.server_ip,
                    latest.sip_port,
                    latest.sip_realm,
                    latest.user_name,
                    latest.password,
                    tx_rc.clone(),
                );
            }));
        }
        if !is_demo {
            crate::portals::request_background_portal(
                engine_bridge.runtime(),
                &crate::i18n::tr("Stay reachable for incoming phone calls in the background"),
            );
            if !is_unconfigured {
                engine_bridge.connect(
                    store.provider,
                    store.server_ip.clone(),
                    store.sip_port,
                    store.sip_realm.clone(),
                    store.user_name.clone(),
                    store.password.clone(),
                    engine_tx.clone(),
                );
            }
        } else {
            window.remove_css_class("call-blocked");
            (sidebar_comps.set_backend_ready)(true);
            (sidebar_comps.refresh_dial_sensitivity)();
        }

        // Beim Beenden SIP-Registrierung sauber entfernen
        let eb_shutdown = engine_bridge.clone();
        app.connect_shutdown(move |_| {
            eb_shutdown.shutdown_blocking();
        });

        let is_backend_ready_eng = is_backend_ready.clone();
        let win_eng = window.clone();
        let set_br_eng = sidebar_comps.set_backend_ready.clone();
        let toast_eng = toast_overlay.clone();
        let in_cid_eng = current_incoming_call_id.clone();
        let in_name_eng = current_incoming_caller_name.clone();
        let b_num_eng = current_caller_number.clone();
        let b_av_eng = banner_avatar.clone();
        let b_name_eng = banner_name.clone();
        let b_details_eng = banner_details.clone();
        let b_banner_eng = call_banner.clone();
        let sv_eng = split_view.clone();
        let app_eng = app.clone();
        let act_eng = active_call.clone();
        let hld_eng = held_call.clone();
        let finish_ui_eng = finish_active_call_ui.clone();
        let clear_held_eng = clear_held_line_ui.clone();
        let ring_eng = ringtone_player.clone();
        let bridge_eng = bridge.clone();

        let eb_sync = engine_bridge.clone();
        let c_list_sync = contacts_list.clone();
        let c_rel_sync = contacts_view_comps.reload.clone();
        let c_pres_sync = contacts_view_comps.refresh_presence.clone();
        let j_list_sync = journal_entries.clone();
        let j_rel_sync = journal_view_comps.reload.clone();
        let j_pres_sync = journal_view_comps.refresh_presence.clone();
        let sb_dial_sens_sync = sidebar_comps.refresh_dial_sensitivity.clone();
        let chat_wins_sync = open_chat_windows.clone();
        let bridge_sync = bridge.clone();
        let b_holder_sync = badge_journal_holder.clone();

        let do_sync_voicemails = {
            let eb = eb_sync.clone();
            let vb_badge = sidebar_comps.voicemail_badge.clone();
            let vb_st = vb_status.clone();
            let vb_lb = vb_list_box.clone();
            let vb_v = voicebox_view.clone();
            let tc_vm_row = trigger_call.clone();

            Rc::new(move || {
                let eb = eb.clone();
                let vb_badge = vb_badge.clone();
                let vb_st = vb_st.clone();
                let vb_lb = vb_lb.clone();
                let vb_v = vb_v.clone();
                let tc_vm = tc_vm_row.clone();

                glib::MainContext::default().spawn_local(async move {
                    if let Ok(messages) = eb.fetch_voicemails().await {
                        if !messages.is_empty() {
                            let unread = messages.iter().filter(|m| m.is_new).count();
                            if unread > 0 {
                                vb_badge.set_text(&unread.to_string());
                                vb_badge.set_visible(true);
                                vb_st.set_title(&crate::i18n::trf("{count} new voicemail(s)", &[("count", &unread.to_string())]));
                            } else {
                                vb_badge.set_visible(false);
                                vb_st.set_title(&crate::i18n::trf("{count} saved voicemail(s)", &[("count", &messages.len().to_string())]));
                            }
                            vb_st.set_description(Some(&crate::i18n::tr("Messages stored in your voicemail inbox:")));
                            vb_v.set_valign(gtk::Align::Start);

                            while let Some(child) = vb_lb.first_child() {
                                vb_lb.remove(&child);
                            }
                            for msg in messages {
                                let caller = msg
                                    .caller_name
                                    .clone()
                                    .filter(|s| !s.is_empty())
                                    .unwrap_or_else(|| msg.caller_number.clone());
                                let sub = format!(
                                    "{} • {} ({} Sek.)",
                                    msg.caller_number, msg.timestamp, msg.duration_seconds
                                );
                                let row = adw::ActionRow::builder()
                                    .title(&caller)
                                    .subtitle(&sub)
                                    .build();
                                let cb_btn = gtk::Button::from_icon_name("call-start-symbolic");
                                cb_btn.add_css_class("flat");
                                cb_btn.set_valign(gtk::Align::Center);
                                cb_btn.set_tooltip_text(Some(&crate::i18n::tr("Call back caller")));
                                let num = msg.caller_number.clone();
                                let name = caller.clone();
                                let tc = tc_vm.clone();
                                cb_btn.connect_clicked(move |_| {
                                    if !num.is_empty() {
                                        tc(name.clone(), num.clone());
                                    }
                                });
                                row.add_suffix(&cb_btn);
                                vb_lb.append(&row);
                            }
                            vb_lb.set_visible(true);
                        }
                    }
                });
            })
        };

        let do_sync_journal = {
            let eb = eb_sync.clone();
            let j_list = j_list_sync.clone();
            let bridge = bridge_sync.clone();
            let j_rel = j_rel_sync.clone();
            let b_holder = b_holder_sync.clone();

            Rc::new(move || {
                let eb = eb.clone();
                let j_list = j_list.clone();
                let bridge = bridge.clone();
                let j_rel = j_rel.clone();
                let b_holder = b_holder.clone();

                glib::MainContext::default().spawn_local(async move {
                    if let Ok(journal) = eb.fetch_journal().await {
                        if !journal.is_empty() {
                            let changed = {
                                let cur = j_list.borrow();
                                *cur != journal
                            };
                            if changed {
                                *j_list.borrow_mut() = journal.clone();
                                let mut st = bridge.load_or_init_data();
                                st.journal = journal;
                                bridge.save_data(&st);
                                j_rel();

                                let missed = j_list
                                    .borrow()
                                    .iter()
                                    .filter(|e| e.direction == crate::models::CallDirection::Missed && !st.seen_call_ids.contains(&e.id))
                                    .count();
                                if let Some(b) = b_holder.borrow().as_ref() {
                                    if missed > 0 {
                                        b.set_text(&missed.to_string());
                                        b.set_visible(true);
                                    } else {
                                        b.set_visible(false);
                                    }
                                }
                            }
                        }
                    }
                });
            })
        };

        let do_sync_contacts = {
            let eb = eb_sync.clone();
            let c_list = c_list_sync.clone();
            let sd_sync = speed_dials.clone();
            let bridge = bridge_sync.clone();
            let c_rel = c_rel_sync.clone();
            let c_pres = c_pres_sync.clone();
            let j_rel = j_rel_sync.clone();
            let j_pres = j_pres_sync.clone();
            let hk_rel = sidebar_comps.reload_hotkeys.clone();
            let hk_pres = sidebar_comps.refresh_hotkeys_presence.clone();
            let sb_sens = sb_dial_sens_sync.clone();
            let chat_wins = chat_wins_sync.clone();
            let p_rev = pickup_revealer.clone();
            let p_av = pickup_avatar.clone();
            let p_name = pickup_name.clone();
            let p_details = pickup_details.clone();
            let p_ext = current_pickup_ext.clone();
            let p_cname = current_pickup_name.clone();
            let in_flight = Rc::new(std::cell::Cell::new(false));
            let avatar_dl_attempted: Rc<RefCell<std::collections::HashSet<String>>> =
                Rc::new(RefCell::new(std::collections::HashSet::new()));

            Rc::new(move || {
                if in_flight.get() {
                    return;
                }
                in_flight.set(true);
                let in_flight_done = in_flight.clone();
                let av_attempted = avatar_dl_attempted.clone();
                let eb = eb.clone();
                let c_list = c_list.clone();
                let sd_sync = sd_sync.clone();
                let bridge = bridge.clone();
                let c_rel = c_rel.clone();
                let c_pres = c_pres.clone();
                let j_rel = j_rel.clone();
                let j_pres = j_pres.clone();
                let hk_rel = hk_rel.clone();
                let hk_pres = hk_pres.clone();
                let sb_sens = sb_sens.clone();
                let chat_wins = chat_wins.clone();
                let p_rev = p_rev.clone();
                let p_av = p_av.clone();
                let p_name = p_name.clone();
                let p_details = p_details.clone();
                let p_ext = p_ext.clone();
                let p_cname = p_cname.clone();

                glib::MainContext::default().spawn_local(async move {
                    if let Ok(phonebook) = eb.fetch_phonebook().await {
                        if !phonebook.is_empty() {
                            let prev_len = c_list.borrow().len();
                            let mut current = c_list.borrow().clone();
                            let mut seen_ids = std::collections::HashSet::new();
                            current.retain(|c| {
                                let keep = !c.name.starts_with("My Extention")
                                    && !c.name.starts_with("My Extension")
                                    && c.id != "1"
                                    && c.id != "2"
                                    && c.id != "3"
                                    && c.id != "4"
                                    && c.id != "5"
                                    && !c.id.starts_with("core-pb-");
                                if !keep {
                                    return false;
                                }
                                if c.id.starts_with("swyx-") {
                                    seen_ids.insert(c.id.clone())
                                } else {
                                    true
                                }
                            });
                            let mut structure_changed = current.len() != prev_len;
                            let mut presence_changed = false;
                            for nc in phonebook {
                                let is_live_employee = nc.id.starts_with("swyx-");
                                if is_live_employee {
                                    let existing_pos = current
                                        .iter()
                                        .position(|c| c.id == nc.id)
                                        .or_else(|| {
                                            if !nc.extension.is_empty() {
                                                current.iter().position(|c| c.extension == nc.extension)
                                            } else {
                                                None
                                            }
                                        });
                                    if let Some(pos) = existing_pos {
                                        let existing = &mut current[pos];
                                        if existing.id != nc.id
                                            || existing.name != nc.name
                                            || (!nc.department.is_empty() && existing.department != nc.department)
                                            || (!nc.phone.is_empty() && existing.phone != nc.phone)
                                            || existing.avatar_file_id != nc.avatar_file_id
                                        {
                                            existing.id = nc.id;
                                            existing.name = nc.name;
                                            if !nc.phone.is_empty() {
                                                existing.phone = nc.phone;
                                            }
                                            if !nc.department.is_empty() {
                                                existing.department = nc.department;
                                            }
                                            existing.avatar_file_id = nc.avatar_file_id;
                                            structure_changed = true;
                                        }
                                        if existing.presence != nc.presence {
                                            existing.presence = nc.presence;
                                            presence_changed = true;
                                        }
                                    } else {
                                        current.push(nc);
                                        structure_changed = true;
                                    }
                                } else {
                                    // Statische Telefonbuch-Einträge (pb-*) dürfen niemals echte Swyx-Mitarbeiter oder deren Live-Präsenz überschreiben
                                    let matches_employee = !nc.extension.is_empty()
                                        && current.iter().any(|c| c.id.starts_with("swyx-") && c.extension == nc.extension);
                                    if matches_employee {
                                        continue;
                                    }
                                    if let Some(existing) = current.iter_mut().find(|c| {
                                        !c.id.starts_with("swyx-")
                                            && (c.id == nc.id
                                                || (!nc.extension.is_empty() && c.extension == nc.extension))
                                    }) {
                                        if existing.name != nc.name
                                            || (!nc.phone.is_empty() && existing.phone != nc.phone)
                                        {
                                            existing.name = nc.name;
                                            if !nc.phone.is_empty() {
                                                existing.phone = nc.phone;
                                            }
                                            structure_changed = true;
                                        }
                                    } else {
                                        current.push(nc);
                                        structure_changed = true;
                                    }
                                }
                            }
                            if structure_changed || presence_changed {
                                current.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                                *c_list.borrow_mut() = current.clone();

                                // Auch SpeedDial-Zustände aktuell halten, damit Neustarts keinen veralteten Status zeigen
                                {
                                    let mut sds = sd_sync.borrow_mut();
                                    for sd in sds.iter_mut() {
                                        let tid = format!("swyx-{}", sd.entity_id);
                                        if let Some(c) = current.iter().find(|c| {
                                            (sd.entity_id > 0 && c.id == tid)
                                                || (!sd.number.is_empty() && c.extension == sd.number)
                                        }) {
                                            sd.state = c.presence.to_swyx_clmgr_state();
                                        }
                                    }
                                }

                                let mut st = bridge.load_or_init_data();
                                st.contacts = current
                                    .iter()
                                    .filter(|c| !c.id.starts_with(crate::gnome_contacts::EDS_ID_PREFIX))
                                    .cloned()
                                    .collect();
                                st.speed_dials = sd_sync.borrow().clone();
                                bridge.save_data(&st);

                                if structure_changed {
                                    c_rel();
                                    j_rel();
                                    hk_rel();
                                } else {
                                    c_pres();
                                    j_pres();
                                    hk_pres();
                                }
                                sb_sens();

                                // Offene Chat-Fenster ebenfalls live aktualisieren
                                let wins = chat_wins.borrow();
                                for c in &current {
                                    let ext_key = if !c.extension.trim().is_empty() {
                                        c.extension.trim()
                                    } else {
                                        c.phone.trim()
                                    };
                                    if let Some(handle) = wins.get(ext_key) {
                                        (handle.update_presence)(c.presence);
                                    }
                                }
                            }

                            // Fehlende Swyx-Profilbilder (Bitmap.FileId) asynchron im Hintergrund nachladen & cachen
                            let mut pending_avatars: Vec<(crate::models::Contact, String)> = Vec::new();
                            let mut preloaded_any = false;
                            for c in &current {
                                if let Some(ref fid) = c.avatar_file_id {
                                    if crate::ui::get_contact_photo(&c.name).is_some() {
                                        continue;
                                    }
                                    if crate::ui::has_avatar_file_on_disk(fid) {
                                        if crate::ui::preload_cached_contact_photo(c) {
                                            preloaded_any = true;
                                        }
                                    } else if av_attempted.borrow_mut().insert(fid.clone()) {
                                        pending_avatars.push((c.clone(), fid.clone()));
                                    }
                                }
                            }
                            if preloaded_any {
                                c_rel();
                                j_rel();
                            }
                            if !pending_avatars.is_empty() {
                                let eb_av = eb.clone();
                                let c_rel_av = c_rel.clone();
                                let j_rel_av = j_rel.clone();
                                glib::MainContext::default().spawn_local(async move {
                                    let mut downloaded = 0usize;
                                    for (contact, fid) in pending_avatars {
                                        if let Ok(Some(bytes)) = eb_av.fetch_avatar_bytes(&fid).await {
                                            if crate::ui::store_contact_photo_bytes(&contact, &fid, &bytes).is_some() {
                                                downloaded += 1;
                                            }
                                        }
                                    }
                                    if downloaded > 0 {
                                        println!(
                                            "[Deskphone] {} Kollegen-Profilbild(er) vom Swyx-Server geladen.",
                                            downloaded
                                        );
                                        c_rel_av();
                                        j_rel_av();
                                    }
                                });
                            }

                            // Signalisierung von Gruppenkollegen / Heranholen (Call Pickup)
                            let own_ext = bridge.load_or_init_data().extension;
                            let sds_now = sd_sync.borrow();
                            // Priorisiere Kollegen aus der eigenen Kurzwahl / Abteilung
                            let ringing_fav = sds_now.iter().find_map(|sd| {
                                current.iter().find(|c| {
                                    c.presence == crate::models::PresenceState::Ringing
                                        && !c.extension.is_empty()
                                        && c.extension != own_ext
                                        && c.extension == sd.number
                                })
                            });
                            let ringing = ringing_fav.or_else(|| {
                                current.iter().find(|c| {
                                    c.presence == crate::models::PresenceState::Ringing
                                        && !c.extension.is_empty()
                                        && c.extension != own_ext
                                })
                            });

                            if let Some(colleague) = ringing {
                                let clean_colleague = crate::ui::sidebar::format_hotkey_display_name(
                                    &colleague.name,
                                );
                                let short_name = crate::ui::clean_name_for_avatar(&clean_colleague);
                                p_av.set_text(Some(&short_name));
                                p_av.set_custom_image(
                                    crate::ui::get_contact_photo(&colleague.name)
                                        .or_else(|| crate::ui::get_contact_photo(&colleague.extension))
                                        .as_ref(),
                                );
                                p_name.set_text(&clean_colleague);
                                let dept = if !colleague.department.is_empty() && colleague.department != "Swyx Intern" {
                                    format!(" • {}", colleague.department)
                                } else {
                                    String::new()
                                };
                                p_details.set_text(&crate::i18n::trf(
                                    "Ringing on {extension}{department}",
                                    &[("extension", &colleague.extension), ("department", &dept)],
                                ));
                                *p_ext.borrow_mut() = colleague.extension.clone();
                                *p_cname.borrow_mut() = clean_colleague;
                                p_rev.set_reveal_child(true);
                            } else if p_rev.reveals_child() {
                                p_rev.set_reveal_child(false);
                                p_ext.borrow_mut().clear();
                                p_cname.borrow_mut().clear();
                            }
                        }
                    }
                    in_flight_done.set(false);
                });
            })
        };

        if !is_demo {
            // Realtime automatic background sync: poll journal & presence every 2 seconds
            // so short ringing phases on colleague extensions are reliably signaled
            let sync_j_tick = do_sync_journal.clone();
            let sync_c_tick = do_sync_contacts.clone();
            glib::timeout_add_local(Duration::from_secs(2), move || {
                sync_j_tick();
                sync_c_tick();
                glib::ControlFlow::Continue
            });

            // Instant background sync on window focus
            let sync_j_win = do_sync_journal.clone();
            let sync_c_win = do_sync_contacts.clone();
            window.connect_is_active_notify(move |w| {
                if w.is_active() {
                    sync_j_win();
                    sync_c_win();
                }
            });
        }

        let conn_banner_eng = conn_banner.clone();
        let conn_spinner_eng = conn_spinner.clone();
        let u_label_eng = sidebar_comps.user_label.clone();
        let u_avatar_eng = sidebar_comps.user_avatar.clone();
        let u_dot_eng = sidebar_comps.user_dot.clone();
        let cfg_user_name = store.user_name.clone();
        let do_sync_j_loop = do_sync_journal.clone();
        let do_sync_c_loop = do_sync_contacts.clone();
        let do_sync_vm_loop = do_sync_voicemails.clone();
        let eb_own_eng = engine_bridge.clone();
        let p_rev_eng = pickup_revealer.clone();
        let vb_badge_eng = sidebar_comps.voicemail_badge.clone();
        let vb_status_eng = vb_status.clone();
        let mwi_acc_eng = mwi_account_holder.clone();
        let chat_wins_eng = open_chat_windows.clone();
        let launch_chat_eng = launch_chat.clone();
        let contacts_eng = contacts_list.clone();
        let speed_dials_eng = speed_dials.clone();
        let registered_account_eng: Rc<RefCell<String>> = Rc::new(RefCell::new(store.extension.clone()));
        let tray_h_eng = tray_handle.clone();
        let cur_pres_eng = current_presence.clone();
        let i_ext_eng = incall_comps.caller_ext_label.clone();
        let i_timer_eng = incall_comps.timer_label.clone();
        let s_timer_eng = sidebar_comps.docked_timer_label.clone();

        glib::spawn_future_local(async move {
            while let Ok(event) = engine_rx.recv().await {
                match event {
                    SoftphoneEvent::RegistrationState { registered, account, message, .. } => {
                        println!(
                            "[EngineBridge] RegistrationState: registered={}, account={}, message={}",
                            registered, account, message
                        );
                        if registered {
                            *is_backend_ready_eng.borrow_mut() = true;
                            if !account.trim().is_empty() {
                                *registered_account_eng.borrow_mut() = account.trim().to_string();
                            }
                            win_eng.remove_css_class("call-blocked");
                            (set_br_eng)(true);

                            // Banner & Spinner aktualisieren
                            conn_spinner_eng.stop();
                            conn_spinner_eng.set_visible(false);
                            conn_banner_eng.remove_css_class("conn-banner-connecting");
                            conn_banner_eng.remove_css_class("conn-banner-warning");
                            conn_banner_eng.add_css_class("conn-banner-success");
                            conn_banner_eng.set_title(&crate::i18n::trf("Connected ({account})", &[("account", &account)]));

                            let cb_hide = conn_banner_eng.clone();
                            glib::timeout_add_local_once(Duration::from_millis(2000), move || {
                                cb_hide.set_revealed(false);
                                let cb_cleanup = cb_hide.clone();
                                glib::timeout_add_local_once(Duration::from_millis(350), move || {
                                    cb_cleanup.remove_css_class("conn-banner-success");
                                    cb_cleanup.remove_css_class("conn-banner-connecting");
                                    cb_cleanup.remove_css_class("conn-banner-warning");
                                    cb_cleanup.set_visible(false);
                                });
                            });

                            // Benutzerprofil in Sidebar aktualisieren
                            let short = if !cfg_user_name.is_empty() {
                                crate::ui::clean_name_for_avatar(&cfg_user_name)
                            } else {
                                format!("Durchwahl {}", account)
                            };
                            let display_name = if !account.is_empty() && !short.contains(&account) {
                                format!("{} ({})", short, account)
                            } else {
                                short.clone()
                            };
                            u_label_eng.set_text(&display_name);
                            u_avatar_eng.set_text(Some(&short));
                            u_dot_eng.remove_css_class("presence-busy");
                            u_dot_eng.remove_css_class("presence-away");
                            u_dot_eng.remove_css_class("presence-offline");
                            u_dot_eng.add_css_class("presence-available");

                            if let Some(ref h) = *tray_h_eng.borrow() {
                                h.update(|t| {
                                    t.presence = PresenceState::Available;
                                });
                            }
                            crate::portals::set_background_status(
                                eb_own_eng.runtime(),
                                &crate::i18n::tr("Ready for incoming calls"),
                            );

                            // Native CDS Contacts, Journal & Voicemail Initial-Synchronisation
                            do_sync_j_loop();
                            do_sync_c_loop();
                            do_sync_vm_loop();

                            // Eigenes Profilbild vom Swyx-Server abrufen (GET /user/{id}/image)
                            {
                                let eb_own_av = eb_own_eng.clone();
                                let av_widget = u_avatar_eng.clone();
                                let own_name_alias = cfg_user_name.clone();
                                let own_ext_alias = account.clone();
                                glib::MainContext::default().spawn_local(async move {
                                    let mut aliases = Vec::new();
                                    if !own_name_alias.trim().is_empty() {
                                        aliases.push(own_name_alias);
                                    }
                                    if !own_ext_alias.trim().is_empty() {
                                        aliases.push(own_ext_alias);
                                    }
                                    match eb_own_av.fetch_own_avatar_bytes().await {
                                        Ok(Some(bytes)) => {
                                            if let Some(tex) = crate::ui::store_own_avatar_bytes(&bytes, &aliases) {
                                                av_widget.set_custom_image(Some(&tex));
                                            }
                                        }
                                        Ok(None) => {
                                            crate::ui::clear_own_avatar_cache(&aliases);
                                            av_widget.set_custom_image(None::<&gtk::gdk::Texture>);
                                        }
                                        Err(_) => {}
                                    }
                                });
                            }
                        } else {
                            if message.contains("Verbinde") {
                                conn_banner_eng.set_title(&message);
                            } else if !message.is_empty() {
                                *is_backend_ready_eng.borrow_mut() = false;
                                win_eng.add_css_class("call-blocked");
                                (set_br_eng)(false);

                                conn_banner_eng.set_visible(true);
                                conn_banner_eng.remove_css_class("conn-banner-connecting");
                                conn_banner_eng.remove_css_class("conn-banner-success");
                                conn_banner_eng.add_css_class("conn-banner-warning");
                                let clean_msg = message.trim_start_matches("Warnung: ").trim_start_matches("⚠️ ");
                                conn_banner_eng.set_title(clean_msg);
                                conn_banner_eng.set_revealed(true);
                                conn_spinner_eng.stop();
                                conn_spinner_eng.set_visible(false);
                            }
                        }
                    }
                    SoftphoneEvent::VoicemailUpdate { new_messages, old_messages, account, .. } => {
                        if let Some(acc) = account {
                            *mwi_acc_eng.borrow_mut() = Some(acc);
                        }
                        if new_messages > 0 {
                            vb_badge_eng.set_text(&new_messages.to_string());
                            vb_badge_eng.set_visible(true);
                            vb_status_eng.set_title(&crate::i18n::trf("{count} new voicemail(s)", &[("count", &new_messages.to_string())]));
                            vb_status_eng.set_description(Some(&crate::i18n::trf(
                                "{new} new and {old} saved voicemail(s) in your inbox.",
                                &[("new", &new_messages.to_string()), ("old", &old_messages.to_string())],
                            )));
                        } else {
                            vb_badge_eng.set_visible(false);
                            vb_status_eng.set_title(&crate::i18n::tr("No New Voicemails"));
                            if old_messages > 0 {
                                vb_status_eng.set_description(Some(&crate::i18n::trf(
                                    "{count} saved voicemail(s) in your inbox.",
                                    &[("count", &old_messages.to_string())],
                                )));
                            } else {
                                vb_status_eng.set_description(Some(&crate::i18n::tr("Your voicemail inbox is currently empty.")));
                            }
                        }
                        do_sync_vm_loop();
                    }
                    SoftphoneEvent::IncomingCall {
                        call_id,
                        caller_number,
                        caller_name,
                        called_number,
                        called_name,
                    } => {
                        let resolved_caller = caller_name
                            .filter(|n| !n.trim().is_empty() && n.trim() != caller_number.trim())
                            .or_else(|| {
                                contacts_eng
                                    .borrow()
                                    .iter()
                                    .find(|c| {
                                        (!c.extension.is_empty() && c.extension == caller_number)
                                            || (!c.phone.is_empty() && c.phone == caller_number)
                                    })
                                    .map(|c| {
                                        crate::ui::sidebar::format_hotkey_display_name(&c.name)
                                    })
                            });

                        let display_name = resolved_caller.unwrap_or_else(|| {
                            if !caller_number.is_empty() {
                                caller_number.clone()
                            } else {
                                "Unbekannter Anrufer".into()
                            }
                        });

                        let st_in = bridge_eng.load_or_init_data();
                        let own_ext = if !st_in.extension.trim().is_empty() {
                            st_in.extension.trim().to_string()
                        } else {
                            registered_account_eng.borrow().clone()
                        };
                        let target_num = called_number.unwrap_or_default().trim().to_string();
                        let target_name = called_name.unwrap_or_default();
                        let in_active_call = act_eng.borrow().is_some();

                        *in_cid_eng.borrow_mut() = call_id.clone();
                        *in_name_eng.borrow_mut() = display_name.clone();
                        *b_num_eng.borrow_mut() = caller_number.clone();
                        let avatar_label = crate::ui::clean_name_for_avatar(&display_name);
                        b_av_eng.set_text(Some(&avatar_label));
                        b_av_eng.set_custom_image(
                            crate::ui::get_contact_photo(&display_name)
                                .or_else(|| crate::ui::get_contact_photo(&caller_number))
                                .as_ref(),
                        );
                        b_name_eng.set_text(&display_name);

                        let is_other_target = !target_num.is_empty() && !own_ext.is_empty() && target_num != own_ext;
                        let matched_colleague = if is_other_target {
                            resolve_called_colleague_name(
                                &target_num,
                                &contacts_eng.borrow(),
                                &speed_dials_eng.borrow(),
                            )
                        } else {
                            None
                        };

                        let target_name_lower = target_name.to_lowercase();
                        let has_group_keyword = [
                            "gruppe",
                            "group",
                            "abteilung",
                            "vertrieb",
                            "support",
                            "zentrale",
                            "empfang",
                            "edv",
                            "service",
                        ]
                        .iter()
                        .any(|kw| target_name_lower.contains(kw));

                        let is_group_call = matched_colleague.is_none()
                            && (has_group_keyword || is_other_target);

                        let num_suffix = if !caller_number.is_empty() && caller_number != display_name {
                            format!(" • {}", caller_number)
                        } else {
                            String::new()
                        };

                        let (notif_title, notif_body) = if let Some(ref colleague_name) = matched_colleague {
                            // Ruf geht an einen konkreten Kollegen aus derselben Gruppe -> kompakt & klar anzeigen
                            let detail_txt = if in_active_call {
                                crate::i18n::trf(
                                    "For {name} ({target}) • Call waiting",
                                    &[("name", colleague_name), ("target", &target_num)],
                                )
                            } else {
                                crate::i18n::trf(
                                    "For {name} ({target}){suffix}",
                                    &[("name", colleague_name), ("target", &target_num), ("suffix", &num_suffix)],
                                )
                            };
                            b_details_eng.set_text(&detail_txt);

                            let title = crate::i18n::trf(
                                "Call for {name} ({target})",
                                &[("name", colleague_name), ("target", &target_num)],
                            );
                            let body = if display_name != caller_number && !caller_number.is_empty() {
                                crate::i18n::trf(
                                    "From: {name} ({number})",
                                    &[("name", &display_name), ("number", &caller_number)],
                                )
                            } else {
                                crate::i18n::trf("From: {name}", &[("name", &display_name)])
                            };
                            (title, body)
                        } else if is_group_call {
                            let tgt_label = if !target_name.trim().is_empty() && !target_num.is_empty() {
                                format!("{} ({})", target_name.trim(), target_num)
                            } else if !target_num.is_empty() {
                                crate::i18n::trf("Group {target}", &[("target", &target_num)])
                            } else {
                                crate::i18n::tr("Group Call")
                            };
                            let detail_txt = crate::i18n::trf(
                                "For {target}{suffix}",
                                &[("target", &tgt_label), ("suffix", &num_suffix)],
                            );
                            b_details_eng.set_text(&detail_txt);

                            let title = if !target_name.trim().is_empty() && !target_num.is_empty() {
                                crate::i18n::trf(
                                    "Group Call {group} ({target}): {caller}",
                                    &[("group", target_name.trim()), ("target", &target_num), ("caller", &display_name)],
                                )
                            } else if !target_num.is_empty() {
                                crate::i18n::trf(
                                    "Group Call ({target}): {caller}",
                                    &[("target", &target_num), ("caller", &display_name)],
                                )
                            } else {
                                crate::i18n::trf("Group Call: {caller}", &[("caller", &display_name)])
                            };
                            let body = crate::i18n::trf("Number: {number}", &[("number", &caller_number)]);
                            (title, body)
                        } else {
                            let detail_txt = if in_active_call {
                                format!("Anklopfen{}", num_suffix)
                            } else {
                                format!("Eingehender Anruf{}", num_suffix)
                            };
                            b_details_eng.set_text(&detail_txt);

                            let title = if in_active_call {
                                format!("Anklopfen: {}", display_name)
                            } else {
                                format!("Eingehender Anruf: {}", display_name)
                            };
                            let body = if in_active_call {
                                crate::i18n::trf("Number: {number} • Line 1 on hold", &[("number", &caller_number)])
                            } else {
                                format!("Rufnummer: {}", caller_number)
                            };
                            (title, body)
                        };

                        b_banner_eng.set_visible(true);
                        sv_eng.set_show_content(true);

                        // Klingelton-Steuerung: Während eines laufenden Gesprächs NIEMALS laut klingeln!
                        // - Gruppen-/Kollegenruf während Telefonat: komplett stumm (nur Benachrichtigung)
                        // - Direkter Zweitanruf (Anklopfen): einmaliger extrem leiser Hinweiston (10 %)
                        let ring_dev = st_in.audio_ring_device;
                        if in_active_call {
                            if matched_colleague.is_none() && !is_group_call {
                                ring_eng.play_call_waiting_beep(&ring_dev);
                            }
                        } else {
                            ring_eng.start(&ring_dev);
                        }

                        let notif = gtk::gio::Notification::new(&notif_title);
                        notif.set_body(Some(&notif_body));
                        notif.set_priority(gtk::gio::NotificationPriority::Urgent);
                        notif.set_icon(&gtk::gio::ThemedIcon::new("call-start-symbolic"));
                        notif.set_default_action("app.show-window");
                        notif.add_button("Annehmen", "app.answer-incoming-call");
                        notif.add_button("Ablehnen", "app.reject-incoming-call");
                        app_eng.send_notification(Some("incoming-call"), &notif);

                        if let Some(ref h) = *tray_h_eng.borrow() {
                            let caller_d = display_name.clone();
                            h.update(move |t| {
                                t.has_incoming_call = true;
                                t.call_info = Some(caller_d);
                            });
                        }
                    }
                    SoftphoneEvent::CallRinging { call_id } => {
                        let ext = act_eng
                            .borrow()
                            .as_ref()
                            .filter(|c| c.id == call_id)
                            .map(|c| c.extension.clone());
                        if let Some(ext) = ext {
                            i_ext_eng.set_text(&crate::i18n::trf("{number} • Line 1 • Ringing…", &[("number", &ext)]));
                        }
                    }
                    SoftphoneEvent::CallConnected { call_id, target_number } => {
                        ring_eng.stop();
                        b_banner_eng.set_visible(false);
                        p_rev_eng.set_reveal_child(false);
                        app_eng.withdraw_notification("incoming-call");

                        // Gesprächsdauer ab Annahme zählen
                        let ext = {
                            let mut act = act_eng.borrow_mut();
                            match act.as_mut() {
                                Some(c) if c.id == call_id => {
                                    c.duration_secs = 0;
                                    Some(c.extension.clone())
                                }
                                _ => None,
                            }
                        };
                        if let Some(ext) = ext {
                            i_ext_eng.set_text(&crate::i18n::trf("{number} • Line 1 • Connected", &[("number", &ext)]));
                            i_timer_eng.set_text("00:00");
                            s_timer_eng.set_text("00:00");
                        }

                        let toast = adw::Toast::new(&crate::i18n::trf("Call connected: {target}", &[("target", &target_number)]));
                        toast.set_timeout(2);
                        toast_eng.add_toast(toast);

                        if let Some(ref h) = *tray_h_eng.borrow() {
                            let t_num = target_number.clone();
                            h.update(move |t| {
                                t.has_incoming_call = false;
                                t.call_info = Some(t_num);
                                t.presence = PresenceState::Busy;
                            });
                        }
                    }
                    SoftphoneEvent::CallEnded { call_id, reason } => {
                        // Klingel-Banner nur schließen, wenn genau dieser (oder kein) Ruf klingelt -
                        // ein endendes Gespräch darf einen parallel klingelnden Ruf nicht verstecken.
                        let ringing_cid = in_cid_eng.borrow().clone();
                        if ringing_cid.is_empty() || ringing_cid == call_id {
                            ring_eng.stop();
                            b_banner_eng.set_visible(false);
                            app_eng.withdraw_notification("incoming-call");
                        }
                        p_rev_eng.set_reveal_child(false);
                        println!("[EngineBridge] CallEnded {}: {}", call_id, reason);
                        let matching = {
                            let cur = act_eng.borrow();
                            cur.as_ref().map(|c| c.id == call_id).unwrap_or(false)
                        };
                        let matching_held_name = {
                            let cur_h = hld_eng.borrow();
                            cur_h
                                .as_ref()
                                .filter(|h| h.id == call_id)
                                .map(|h| h.caller_name.clone())
                        };
                        if matching {
                            let normal_end = reason == "User hangup"
                                || reason.starts_with("Transferred")
                                || reason.starts_with("Attended")
                                || reason.starts_with("Deflected");
                            let msg = if reason == "Remote party hung up" {
                                Some(crate::i18n::tr("Remote party hung up"))
                            } else if !normal_end {
                                Some(reason.clone())
                            } else {
                                None
                            };
                            if let Some(m) = msg {
                                let toast = adw::Toast::new(&m);
                                toast.set_timeout(4);
                                toast_eng.add_toast(toast);
                            }
                            finish_ui_eng();
                        } else if let Some(h_name) = matching_held_name {
                            clear_held_eng();
                            if !reason.starts_with("Attended") && !reason.starts_with("Transferred") {
                                let toast = adw::Toast::new(&crate::i18n::trf(
                                    "Held call ({name}) ended",
                                    &[("name", &h_name)],
                                ));
                                toast.set_timeout(3);
                                toast_eng.add_toast(toast);
                            }
                        } else {
                            let is_ringing_call = *in_cid_eng.borrow() == call_id;
                            let missed_name = in_name_eng.borrow().clone();
                            if is_ringing_call && !missed_name.is_empty() {
                                let text = if reason == "Answered elsewhere" {
                                    format!("Anruf von {} wurde anderweitig angenommen", missed_name)
                                } else {
                                    format!("Verpasster Anruf von {}", missed_name)
                                };
                                let toast = adw::Toast::new(&text);
                                toast.set_timeout(3);
                                toast_eng.add_toast(toast);
                                in_name_eng.borrow_mut().clear();
                                in_cid_eng.borrow_mut().clear();
                            }
                        }

                        if act_eng.borrow().is_none() && hld_eng.borrow().is_none() {
                            if let Some(ref h) = *tray_h_eng.borrow() {
                                let p = *cur_pres_eng.borrow();
                                h.update(move |t| {
                                    t.has_incoming_call = false;
                                    t.call_info = None;
                                    t.presence = p;
                                });
                            }
                        }

                        let sync_j_end = do_sync_j_loop.clone();
                        let sync_vm_end = do_sync_vm_loop.clone();
                        glib::timeout_add_local_once(Duration::from_millis(1500), move || {
                            sync_j_end();
                            sync_vm_end();
                        });
                    }
                    SoftphoneEvent::Dtmf { digit, .. } => {
                        println!("[EngineBridge] DTMF-Ton '{}' gesendet/empfangen", digit);
                    }
                    SoftphoneEvent::ChatMessageReceived { from, message, .. } => {
                        let trimmed_msg = message.trim();
                        if trimmed_msg.is_empty()
                            || trimmed_msg.starts_with("<?xml")
                            || trimmed_msg.starts_with("<messages")
                        {
                            continue;
                        }
                        let (parsed_name, parsed_ext) = if let Some(idx) = from.rfind('(') {
                            let name_part = from[..idx].trim().to_string();
                            let ext_part = from[idx + 1..].trim_end_matches(')').trim().to_string();
                            (name_part, ext_part)
                        } else {
                            let trimmed = from.trim().to_string();
                            (trimmed.clone(), trimmed)
                        };

                        let contact = contacts_eng
                            .borrow()
                            .iter()
                            .find(|c| {
                                (!parsed_ext.is_empty()
                                    && (c.extension == parsed_ext
                                        || c.phone == parsed_ext
                                        || c.id == parsed_ext
                                        || c.id == format!("swyx-{}", parsed_ext)))
                                    || (!parsed_name.is_empty()
                                        && (c.name == parsed_name
                                            || c.id == parsed_name
                                            || c.id == format!("swyx-{}", parsed_name)))
                            })
                            .cloned()
                            .unwrap_or_else(|| {
                                let fallback_id = if parsed_name.starts_with("swyx-") {
                                    parsed_name.clone()
                                } else if parsed_ext.starts_with("swyx-") {
                                    parsed_ext.clone()
                                } else {
                                    format!("chat-{}", parsed_ext)
                                };
                                crate::models::Contact {
                                    id: fallback_id,
                                    name: if parsed_name.is_empty() || parsed_name.starts_with("swyx-") {
                                        parsed_ext.clone()
                                    } else {
                                        parsed_name
                                    },
                                    extension: parsed_ext.clone(),
                                    department: "Swyx Messenger".into(),
                                    presence: crate::models::PresenceState::Available,
                                    phone: parsed_ext.clone(),
                                    avatar_file_id: None,
                                }
                            });

                        let ext_key = if !contact.extension.trim().is_empty() {
                            contact.extension.trim().to_string()
                        } else {
                            contact.phone.trim().to_string()
                        };

                        launch_chat_eng(contact.clone());

                        if let Some(handle) = chat_wins_eng.borrow().get(&ext_key) {
                            (handle.receive_message)(contact.name.clone(), message.clone());
                        }

                        let notif = gtk::gio::Notification::new(&crate::i18n::trf("Message from {name}", &[("name", &contact.name)]));
                        notif.set_body(Some(&message));
                        notif.set_icon(&gtk::gio::ThemedIcon::new("user-available-symbolic"));
                        notif.set_default_action("app.show-window");
                        app_eng.send_notification(Some("incoming-chat"), &notif);
                    }
                    _ => {}
                }
            }
        });

        // GNOME Contacts / GNOME Online Accounts (Evolution Data Server) live read-only sync
        let do_sync_gnome_contacts: Rc<dyn Fn(bool)> = {
            let eb_gc = engine_bridge.clone();
            let c_list_gc = contacts_list.clone();
            let c_rel_gc = contacts_view_comps.reload.clone();
            let j_rel_gc = journal_view_comps.reload.clone();
            let sb_sens_gc = sidebar_comps.refresh_dial_sensitivity.clone();
            Rc::new(move |enabled: bool| {
                let eb = eb_gc.clone();
                let c_list = c_list_gc.clone();
                let c_rel = c_rel_gc.clone();
                let j_rel = j_rel_gc.clone();
                let sb_sens = sb_sens_gc.clone();
                if !enabled {
                    for c in c_list.borrow().iter() {
                        if c.id.starts_with(crate::gnome_contacts::EDS_ID_PREFIX)
                            || c.avatar_file_id
                                .as_deref()
                                .is_some_and(|fid| fid.starts_with(crate::gnome_contacts::EDS_PHOTO_PREFIX))
                        {
                            crate::ui::clear_contact_photo_cache(c);
                        }
                    }
                    let changed = crate::gnome_contacts::merge_eds_contacts(
                        &mut c_list.borrow_mut(),
                        Vec::new(),
                    );
                    if changed {
                        c_rel();
                        j_rel();
                        sb_sens();
                    }
                    return;
                }
                glib::MainContext::default().spawn_local(async move {
                    let eds_list = eb.fetch_gnome_contacts().await;
                    let changed = crate::gnome_contacts::merge_eds_contacts(
                        &mut c_list.borrow_mut(),
                        eds_list,
                    );
                    let mut photos_loaded = false;
                    for c in c_list.borrow().iter() {
                        if c.avatar_file_id
                            .as_deref()
                            .is_some_and(|fid| fid.starts_with(crate::gnome_contacts::EDS_PHOTO_PREFIX))
                            && crate::ui::preload_cached_contact_photo(c)
                        {
                            photos_loaded = true;
                        }
                    }
                    if changed || photos_loaded {
                        c_rel();
                        j_rel();
                        sb_sens();
                    }
                });
            })
        };
        *gnome_contacts_sync_holder.borrow_mut() = Some(do_sync_gnome_contacts.clone());
        if !is_demo && store.sync_gnome_contacts {
            do_sync_gnome_contacts(true);
        }

        let show_demo_call: Rc<dyn Fn()> = {
            let sel = on_select.clone();
            let name_lbl = incall_comps.caller_name_label.clone();
            let ext_lbl = incall_comps.caller_ext_label.clone();
            let timer_lbl = incall_comps.timer_label.clone();
            Rc::new(move || {
                name_lbl.set_text("Clara Weber");
                ext_lbl.set_text(&crate::i18n::trf(
                    "{number} • Line 1 • Connected",
                    &[("number", "201")],
                ));
                timer_lbl.set_text("04:12");
                sel("incall");
            })
        };

        MainWindow {
            window,
            dial_entry: sidebar_comps.search_entry.clone(),
            show_demo_call,
        }
    }
}
