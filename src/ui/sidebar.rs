use gtk4 as gtk;
use gtk::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use crate::models::{ActiveCall, AudioMode, Contact, HeldCall, PresenceState, TelephonyMode};

#[allow(dead_code)]
pub struct SidebarComponents {
    pub root: adw::BreakpointBin,
    pub container: gtk::Box,
    pub header_bar: adw::HeaderBar,
    pub list_box: gtk::ListBox,
    pub journal_badge: gtk::Label,
    pub voicemail_badge: gtk::Label,
    pub device_button: gtk::Button,
    pub line1_chip: gtk::Box,
    pub line1_badge: gtk::Label,
    pub line1_icon: gtk::Image,
    pub line1_volume_scale: gtk::Scale,
    pub line2_chip: gtk::Box,
    pub line2_volume_scale: gtk::Scale,
    pub line2_badge: gtk::Label,
    pub line2_icon: gtk::Image,
    pub lines_revealer: gtk::Revealer,
    pub search_entry: gtk::SearchEntry,
    pub call_btn: gtk::Button,
    pub docked_call_box: gtk::Box,
    pub docked_call_revealer: gtk::Revealer,
    pub docked_held_box: gtk::Box,
    pub docked_held_label: gtk::Label,
    pub docked_swap_btn: gtk::Button,
    pub docked_transfer_held_btn: gtk::Button,
    pub docked_caller_label: gtk::Label,
    pub docked_ext_label: gtk::Label,
    pub docked_timer_label: gtk::Label,
    pub docked_mute_btn: gtk::Button,
    pub docked_dtmf_btn: gtk::Button,
    pub docked_hangup_btn: gtk::Button,
    pub user_label: gtk::Label,
    pub user_avatar: adw::Avatar,
    pub user_dot: gtk::Box,
    pub autocomplete_popover: gtk::Popover,
    pub hotkeys_revealer: gtk::Revealer,
    pub hotkeys_toggle_btn: gtk::ToggleButton,
    pub speed_dial_pane: adw::ToolbarView,
    pub set_speed_dial_detached: Rc<dyn Fn(bool)>,
    pub reload_hotkeys: Rc<dyn Fn()>,
    pub refresh_hotkeys_presence: Rc<dyn Fn()>,
    pub set_backend_ready: Rc<dyn Fn(bool)>,
    pub refresh_dial_sensitivity: Rc<dyn Fn()>,
}

/// Prüft eine Eingabe in der Schnellwahlleiste und löst entweder einen bekannten Kontakt
/// (aus Telefonbuch oder Kurzwahl) zu `(Anzeigename, Zielnummer)` auf oder validiert eine
/// direkte Rufnummer / Durchwahl / Anlagen-Steuercode. Existiert ein eingegebener Name nicht
/// im Telefonbuch, wird `None` zurückgegeben (Anruf-Button bleibt deaktiviert).
pub fn resolve_dial_input(
    raw: &str,
    contacts: &[Contact],
    speed_dials: &[crate::models::SpeedDialEntry],
) -> Option<(String, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let q = trimmed.to_lowercase();

    // 1. Exakter Treffer im Telefonbuch (Name, formatierter Anzeigename, Durchwahl oder Rufnummer)
    if let Some(c) = contacts.iter().find(|c| {
        let disp_lower = format_hotkey_display_name(&c.name).trim().to_lowercase();
        let clean_lower = crate::ui::clean_name_for_avatar(&c.name).trim().to_lowercase();
        c.name.trim().to_lowercase() == q
            || disp_lower == q
            || clean_lower == q
            || (!c.extension.trim().is_empty() && c.extension.trim().to_lowercase() == q)
            || (!c.phone.trim().is_empty() && c.phone.trim().to_lowercase() == q)
    }) {
        let raw_num = if !c.extension.trim().is_empty() {
            c.extension.trim()
        } else {
            c.phone.trim()
        };
        let dial_num: String = raw_num
            .chars()
            .filter(|ch| !matches!(ch, ' ' | '-' | '/' | '(' | ')' | '.'))
            .collect();
        if !dial_num.is_empty() {
            let disp_name = format_hotkey_display_name(&c.name);
            return Some((disp_name, dial_num));
        }
    }

    // 2. Exakter Treffer in den Kurzwahl-Favoriten
    if let Some(sd) = speed_dials.iter().find(|sd| {
        !sd.number.trim().is_empty()
            && (sd.name.trim().to_lowercase() == q
                || format_hotkey_display_name(&sd.name).trim().to_lowercase() == q
                || sd.number.trim().to_lowercase() == q)
    }) {
        let disp_name = if sd.name.trim().is_empty() {
            sd.number.trim().to_string()
        } else {
            format_hotkey_display_name(&sd.name)
        };
        return Some((disp_name, sd.number.trim().to_string()));
    }

    // 3. Reine Rufnummer / Durchwahl / Anlagen-Steuercode (keine Buchstaben!)
    if trimmed.chars().any(|c| c.is_alphabetic()) {
        return None;
    }

    let mut normalized = String::with_capacity(trimmed.len());
    let mut digit_count = 0usize;
    for (idx, ch) in trimmed.chars().enumerate() {
        if ch.is_ascii_digit() {
            digit_count += 1;
            normalized.push(ch);
        } else if ch == '*' || ch == '#' {
            normalized.push(ch);
        } else if ch == '+' && idx == 0 && normalized.is_empty() {
            normalized.push(ch);
        } else if matches!(ch, ' ' | '-' | '/' | '(' | ')' | '.') {
            continue;
        } else {
            return None;
        }
    }

    let min_digits = if normalized.starts_with('+') { 4 } else { 1 };
    if digit_count >= min_digits && normalized.len() >= 2 {
        Some((trimmed.to_string(), normalized))
    } else {
        None
    }
}

pub fn format_hotkey_display_name(name: &str) -> String {
    let parts: Vec<&str> = name.split_whitespace().collect();
    if parts.is_empty() {
        return name.to_string();
    }
    let has_initial_dot = parts.iter().any(|p| p.ends_with('.'));
    if has_initial_dot {
        crate::ui::clean_name_for_avatar(name)
    } else {
        let filtered: Vec<&str> = parts
            .into_iter()
            .filter(|part| part.chars().any(|c| c.is_alphabetic()))
            .collect();
        if filtered.is_empty() {
            name.to_string()
        } else {
            filtered.join(" ")
        }
    }
}

pub fn create_sidebar<FCall, FSelect, FToggleCti, FForward, FPresence, FHotkey, FTransfer, FCopy, FFav, FChat, FDialpad>(
    contacts: Rc<RefCell<Vec<Contact>>>,
    speed_dials: Rc<RefCell<Vec<crate::models::SpeedDialEntry>>>,
    toast_overlay: adw::ToastOverlay,
    _current_audio: Rc<RefCell<AudioMode>>,
    current_presence: Rc<RefCell<PresenceState>>,
    current_telephony_mode: Rc<RefCell<TelephonyMode>>,
    active_call: Rc<RefCell<Option<ActiveCall>>>,
    held_call: Rc<RefCell<Option<HeldCall>>>,
    initial_user_name: String,
    initial_extension: String,
    initial_forward_number: String,
    initial_forward_enabled: bool,
    initial_dialpad_visible: bool,
    on_trigger_call: FCall,
    on_select_page: FSelect,
    on_toggle_cti: FToggleCti,
    on_forward_changed: FForward,
    on_presence_changed: FPresence,
    on_hotkey_click: FHotkey,
    on_transfer_call: FTransfer,
    on_copy_number: FCopy,
    on_fav_changed: FFav,
    on_open_chat: FChat,
    on_dialpad_toggle: FDialpad,
) -> SidebarComponents
where
    FCall: Fn(String, String) + 'static + Clone,
    FSelect: Fn(&str) + 'static + Clone,
    FToggleCti: Fn(TelephonyMode) + 'static + Clone,
    FForward: Fn(bool, String) + 'static + Clone,
    FPresence: Fn(PresenceState) + 'static + Clone,
    FHotkey: Fn(String, String, PresenceState) + 'static + Clone,
    FTransfer: Fn(String, String) + 'static + Clone,
    FCopy: Fn(String) + 'static + Clone,
    FFav: Fn() + 'static + Clone,
    FChat: Fn(String, String) + 'static + Clone,
    FDialpad: Fn(bool) + 'static + Clone,
{
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    container.add_css_class("sidebar-container");
    container.set_size_request(200, -1);

    let is_user_typing = Rc::new(Cell::new(false));
    let popover_holder: Rc<RefCell<Option<gtk::Popover>>> = Rc::new(RefCell::new(None));

    // Pre-create Leitung 1 status widgets so CTI toggle can update them dynamically
    let chip_line1 = gtk::Box::new(gtk::Orientation::Vertical, 4);
    chip_line1.add_css_class("line-chip");
    let act_drag1 = active_call.clone();
    crate::ui::attach_call_drag_source_with_tag(&chip_line1, "CALL_TRANSFER", move || {
        act_drag1.borrow().is_some()
    });

    let badge_line1 = gtk::Label::new(Some(&crate::i18n::tr("Ready")));
    badge_line1.add_css_class("caption");
    badge_line1.add_css_class("dim-label");

    // 1. Sidebar HeaderBar: Exakt gleiche dünne Höhe & Trennlinie wie die gesamte Anwendung
    let header_bar = adw::HeaderBar::new();
    header_bar.add_css_class("sidebar-header");
    header_bar.set_show_start_title_buttons(false);
    header_bar.set_show_end_title_buttons(false);
    header_bar.set_show_title(false);

    let status_pill = gtk::Button::new();
    status_pill.add_css_class("flat");
    status_pill.set_valign(gtk::Align::Center);
    status_pill.set_tooltip_text(Some(&crate::i18n::tr("Change presence status (click to select)")));

    let status_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    status_box.set_valign(gtk::Align::Center);

    // Native Libadwaita avatar showing user initials with badge dot overlay
    let initial_short = if !initial_user_name.is_empty() {
        crate::ui::clean_name_for_avatar(&initial_user_name)
    } else {
        "Deskphone".to_string()
    };

    let initial_display = if initial_forward_enabled {
        initial_short.clone()
    } else if !initial_extension.is_empty() && !initial_short.contains(&initial_extension) {
        format!("{} ({})", initial_short, initial_extension)
    } else {
        initial_short.clone()
    };

    let avatar = adw::Avatar::builder()
        .size(24)
        .text(&initial_short)
        .show_initials(true)
        .build();
    if let Some(own_tex) = crate::ui::load_own_avatar_texture() {
        avatar.set_custom_image(Some(&own_tex));
    }
    avatar.set_size_request(24, 24);
    avatar.set_valign(gtk::Align::Center);
    avatar.set_halign(gtk::Align::Center);

    let overlay = gtk::Overlay::new();
    let overlay_size = 26;
    overlay.set_size_request(overlay_size, overlay_size);
    overlay.set_valign(gtk::Align::Center);
    overlay.set_halign(gtk::Align::Center);
    overlay.set_overflow(gtk::Overflow::Visible);
    overlay.set_child(Some(&avatar));

    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("presence-badge-dot");
    dot.add_css_class(current_presence.borrow().css_class());
    dot.set_halign(gtk::Align::End);
    dot.set_valign(gtk::Align::End);
    overlay.add_overlay(&dot);

    let label = gtk::Label::new(Some(&initial_display));
    label.add_css_class("heading");
    label.set_valign(gtk::Align::Center);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(22);

    let fwd_icon = gtk::Image::from_icon_name("go-jump-symbolic");
    fwd_icon.add_css_class("fwd-active-icon");
    fwd_icon.set_pixel_size(14);
    fwd_icon.set_valign(gtk::Align::Center);
    fwd_icon.set_tooltip_text(Some(&crate::i18n::tr("Call forwarding active")));
    fwd_icon.set_visible(false);

    status_box.append(&overlay);
    status_box.append(&label);
    status_box.append(&fwd_icon);
    status_pill.set_child(Some(&status_box));

    // Popover menu for presence states
    let popover = gtk::Popover::new();
    popover.set_parent(&status_pill);
    popover.set_position(gtk::PositionType::Bottom);
    popover.add_css_class("menu");
    let pop_unparent = popover.clone();
    status_pill.connect_destroy(move |_| {
        pop_unparent.unparent();
    });
    let pop_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    pop_box.set_size_request(260, -1);
    pop_box.set_margin_top(6);
    pop_box.set_margin_bottom(6);
    pop_box.set_margin_start(6);
    pop_box.set_margin_end(6);

    let states = [
        PresenceState::Available,
        PresenceState::Busy,
        PresenceState::Away,
        PresenceState::LoggedOut,
    ];

    let pres_checks: Rc<RefCell<Vec<(PresenceState, gtk::Image)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let initial_pres = *current_presence.borrow();

    for state in states {
        let btn_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let s_dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        s_dot.add_css_class("presence-badge-dot");
        s_dot.add_css_class(state.css_class());
        s_dot.set_valign(gtk::Align::Center);

        let s_lbl = gtk::Label::new(Some(&state.label()));
        s_lbl.set_halign(gtk::Align::Start);

        let s_check = gtk::Image::from_icon_name("object-select-symbolic");
        s_check.set_hexpand(true);
        s_check.set_halign(gtk::Align::End);
        s_check.set_pixel_size(14);
        s_check.set_opacity(if state == initial_pres { 1.0 } else { 0.0 });
        pres_checks.borrow_mut().push((state, s_check.clone()));

        btn_box.append(&s_dot);
        btn_box.append(&s_lbl);
        btn_box.append(&s_check);

        let item_btn = gtk::Button::new();
        item_btn.set_child(Some(&btn_box));
        item_btn.add_css_class("flat");

        let p_ref = current_presence.clone();
        let dot_clone = dot.clone();
        let pop_clone = popover.clone();
        let on_pres = on_presence_changed.clone();
        let checks_click = pres_checks.clone();
        item_btn.connect_clicked(move |_| {
            *p_ref.borrow_mut() = state;
            dot_clone.remove_css_class("presence-available");
            dot_clone.remove_css_class("presence-busy");
            dot_clone.remove_css_class("presence-away");
            dot_clone.remove_css_class("presence-offline");
            dot_clone.add_css_class(state.css_class());
            for (st, img) in checks_click.borrow().iter() {
                img.set_opacity(if *st == state { 1.0 } else { 0.0 });
            }
            on_pres(state);
            pop_clone.popdown();
        });

        pop_box.append(&item_btn);
    }

    // Keep checkmarks synced with external presence changes (e.g. tray) without changing layout geometry
    let p_show = current_presence.clone();
    let checks_show = pres_checks.clone();
    popover.connect_show(move |_| {
        let cur = *p_show.borrow();
        for (st, img) in checks_show.borrow().iter() {
            img.set_opacity(if *st == cur { 1.0 } else { 0.0 });
        }
    });

    // --- SECTION 2: Rufumleitung (Kompakt & aufgeräumt ohne Extra-Icons im Hauptfenster) ---
    let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
    sep.set_margin_top(6);
    sep.set_margin_bottom(4);
    pop_box.append(&sep);

    let sec2_lbl = gtk::Label::new(Some(&crate::i18n::tr("CALL FORWARDING")));
    sec2_lbl.add_css_class("caption");
    sec2_lbl.add_css_class("dim-label");
    sec2_lbl.set_halign(gtk::Align::Start);
    sec2_lbl.set_margin_start(6);
    sec2_lbl.set_margin_top(2);
    pop_box.append(&sec2_lbl);

    let initial_num = if initial_forward_number.is_empty() {
        String::from("0170 1234567")
    } else {
        initial_forward_number
    };
    let initial_mode = if initial_forward_enabled { 1 } else { 0 };

    let forward_state = Rc::new(RefCell::new(initial_mode)); // 0 = Aus, 1 = Aktiv an Zielnummer
    let forward_number = Rc::new(RefCell::new(initial_num.clone()));

    // 1) Aus (Keine Umleitung)
    let btn_fwd_off = gtk::Button::new();
    btn_fwd_off.add_css_class("flat");
    let box_fwd_off = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon_fwd_off = gtk::Image::from_icon_name("action-unavailable-symbolic");
    icon_fwd_off.set_pixel_size(14);
    let lbl_fwd_off = gtk::Label::new(Some(&crate::i18n::tr("Off (No Forwarding)")));
    box_fwd_off.append(&icon_fwd_off);
    box_fwd_off.append(&lbl_fwd_off);
    let check_off = gtk::Image::from_icon_name("object-select-symbolic");
    check_off.set_hexpand(true);
    check_off.set_halign(gtk::Align::End);
    check_off.set_pixel_size(14);
    check_off.set_opacity(if !initial_forward_enabled { 1.0 } else { 0.0 });
    box_fwd_off.append(&check_off);
    btn_fwd_off.set_child(Some(&box_fwd_off));
    pop_box.append(&btn_fwd_off);

    // 2) Weiterleiten an Rufnummer (1-Klick Auswahl!)
    let btn_fwd_on = gtk::Button::new();
    btn_fwd_on.add_css_class("flat");
    let box_fwd_on = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon_fwd_on = gtk::Image::from_icon_name("go-jump-symbolic");
    icon_fwd_on.add_css_class("fwd-active-icon");
    icon_fwd_on.set_pixel_size(14);
    let lbl_fwd_on = gtk::Label::new(Some(&crate::i18n::tr("Forward to:")));
    box_fwd_on.append(&icon_fwd_on);
    box_fwd_on.append(&lbl_fwd_on);
    let check_on = gtk::Image::from_icon_name("object-select-symbolic");
    check_on.set_hexpand(true);
    check_on.set_halign(gtk::Align::End);
    check_on.set_pixel_size(14);
    check_on.set_opacity(if initial_forward_enabled { 1.0 } else { 0.0 });
    box_fwd_on.append(&check_on);
    btn_fwd_on.set_child(Some(&box_fwd_on));
    pop_box.append(&btn_fwd_on);

    // 3) Rufnummer bearbeiten (Direktes Eingabefeld)
    let entry_fwd = gtk::Entry::new();
    entry_fwd.set_placeholder_text(Some(&crate::i18n::tr("Phone number (Enter)")));
    entry_fwd.set_text(&initial_num);
    entry_fwd.set_margin_top(4);
    entry_fwd.set_margin_bottom(2);
    entry_fwd.set_margin_start(4);
    entry_fwd.set_margin_end(4);
    pop_box.append(&entry_fwd);

    // Popover-Inhalt setzen
    popover.set_child(Some(&pop_box));

    // Umschalt-Logik
    let cb_fwd = on_forward_changed.clone();
    let update_fwd_ui = {
        let f_state = forward_state.clone();
        let f_num = forward_number.clone();
        let s_pill = status_pill.clone();
        let c_off = check_off.clone();
        let c_on = check_on.clone();
        let f_icon = fwd_icon.clone();
        let p_pop = popover.clone();
        let u_label = label.clone();
        let on_change = cb_fwd.clone();
        let u_name = initial_user_name.clone();
        let u_ext = initial_extension.clone();

        Rc::new(move |new_mode: i32| {
            *f_state.borrow_mut() = new_mode;
            c_off.set_opacity(if new_mode == 0 { 1.0 } else { 0.0 });
            c_on.set_opacity(if new_mode == 1 { 1.0 } else { 0.0 });

            let base_name = if !u_name.is_empty() {
                crate::ui::clean_name_for_avatar(&u_name)
            } else {
                "Deskphone".to_string()
            };

            let num = f_num.borrow().clone();
            if new_mode == 1 {
                let display_num = if num.is_empty() { "Mobil".to_string() } else { num.clone() };
                f_icon.set_visible(true);
                u_label.set_text(&base_name);
                s_pill.set_tooltip_text(Some(&crate::i18n::trf("Status • Call forwarding active to: {number}", &[("number", &display_num)])));
            } else {
                f_icon.set_visible(false);
                let full_lbl = if !u_ext.is_empty() && !base_name.contains(&u_ext) {
                    format!("{} ({})", base_name, u_ext)
                } else {
                    base_name
                };
                u_label.set_text(&full_lbl);
                s_pill.set_tooltip_text(Some(&crate::i18n::tr("Presence status & call forwarding (click to change)")));
            }
            on_change(new_mode == 1, num);
            p_pop.popdown();
        })
    };

    // Wende den initial gespeicherten Zustand auf die UI an (Pfeil-Icon / Tooltip)
    if initial_forward_enabled {
        let display_num = if initial_num.is_empty() { "Mobil".to_string() } else { initial_num.clone() };
        fwd_icon.set_visible(true);
        let base_name = if !initial_user_name.is_empty() {
            crate::ui::clean_name_for_avatar(&initial_user_name)
        } else {
            "Deskphone".to_string()
        };
        label.set_text(&base_name);
        status_pill.set_tooltip_text(Some(&crate::i18n::trf("Status • Call forwarding active to: {number}", &[("number", &display_num)])));
    }

    let u1 = update_fwd_ui.clone();
    btn_fwd_off.connect_clicked(move |_| u1(0));

    let u2 = update_fwd_ui.clone();
    btn_fwd_on.connect_clicked(move |_| u2(1));

    let u_entry = update_fwd_ui.clone();
    let e_clone1 = entry_fwd.clone();
    let fn_ref1 = forward_number.clone();
    entry_fwd.connect_activate(move |_| {
        let text = e_clone1.text().trim().to_string();
        *fn_ref1.borrow_mut() = text;
        u_entry(1);
    });

    let fn_change = forward_number.clone();
    let on_fwd_type = on_forward_changed.clone();
    let f_state_type = forward_state.clone();
    entry_fwd.connect_changed(move |e| {
        let text = e.text().trim().to_string();
        *fn_change.borrow_mut() = text.clone();
        on_fwd_type(*f_state_type.borrow() == 1, text);
    });

    let pop_clone = popover.clone();
    status_pill.connect_clicked(move |_| {
        pop_clone.popup();
    });

    header_bar.pack_start(&status_pill);

    // Unified Telephony Mode Switcher (reserved for future hardware CTI support; hidden until implemented)
    let cur_t = *current_telephony_mode.borrow();
    let device_button = gtk::Button::from_icon_name(cur_t.icon_name());
    device_button.add_css_class("flat");
    device_button.add_css_class("circular");
    device_button.set_valign(gtk::Align::Center);
    device_button.set_tooltip_text(Some(&cur_t.tooltip()));
    device_button.set_visible(false);

    if cur_t.is_cti() {
        badge_line1.set_text("CTI");
        chip_line1.set_tooltip_text(Some(&crate::i18n::tr("Line 1 • CTI mode ready • Desk phone connected")));
    } else {
        badge_line1.set_text(&crate::i18n::tr("Ready"));
        chip_line1.set_tooltip_text(Some(&crate::i18n::tr("Line 1 • Softphone ready • PC Audio")));
    }

    let tm = current_telephony_mode.clone();
    let dev_btn = device_button.clone();
    let b_l1 = badge_line1.clone();
    let c_l1 = chip_line1.clone();
    let on_cti = on_toggle_cti.clone();

    device_button.connect_clicked(move |_| {
        let new_mode = match *tm.borrow() {
            TelephonyMode::Softphone => TelephonyMode::Cti,
            TelephonyMode::Cti => TelephonyMode::Softphone,
        };
        *tm.borrow_mut() = new_mode;

        dev_btn.set_icon_name(new_mode.icon_name());
        dev_btn.set_tooltip_text(Some(&new_mode.tooltip()));

        if new_mode.is_cti() {
            b_l1.set_text("CTI");
            c_l1.set_tooltip_text(Some(&crate::i18n::tr("Line 1 • CTI mode active • Controlling desk phone")));
        } else {
            b_l1.set_text(&crate::i18n::tr("Ready"));
            c_l1.set_tooltip_text(Some(&crate::i18n::tr("Line 1 • Softphone ready • PC Audio")));
        }

        on_cti(new_mode);
    });

    let p_hdr = popover_holder.clone();
    let is_typing_hdr = is_user_typing.clone();
    let hdr_click = gtk::GestureClick::new();
    hdr_click.connect_pressed(move |_, _, _, _| {
        is_typing_hdr.set(false);
        if let Some(p) = p_hdr.borrow().as_ref() {
            p.popdown();
        }
    });
    header_bar.add_controller(hdr_click);

    header_bar.pack_end(&device_button);
    container.append(&header_bar);

    // 2. Navigation ListBox (pinned directly below HeaderBar — never scrolls away)
    let list_box = gtk::ListBox::new();
    list_box.add_css_class("navigation-sidebar");
    list_box.set_selection_mode(gtk::SelectionMode::Single);
    list_box.set_margin_start(10);
    list_box.set_margin_end(10);
    list_box.set_margin_top(2);
    list_box.set_margin_bottom(4);

    // Row 1: Rufjournal (Badge 9+)
    let row_journal = gtk::ListBoxRow::new();
    let box_j = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    box_j.set_margin_top(4);
    box_j.set_margin_bottom(4);
    box_j.set_margin_start(8);
    box_j.set_margin_end(8);

    let icon_j = gtk::Image::from_icon_name("document-open-recent-symbolic");
    let lbl_j = gtk::Label::new(Some(&crate::i18n::tr("Call Journal")));
    lbl_j.add_css_class("heading");
    let sp_j = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    sp_j.set_hexpand(true);

    let badge_j = gtk::Label::new(Some("9+"));
    badge_j.add_css_class("sidebar-badge");

    box_j.append(&icon_j);
    box_j.append(&lbl_j);
    box_j.append(&sp_j);
    box_j.append(&badge_j);
    row_journal.set_child(Some(&box_j));
    row_journal.set_tooltip_text(Some(&crate::i18n::tr("Call journal & missed calls")));
    list_box.append(&row_journal);

    // Row 2: Contacts
    let row_contacts = gtk::ListBoxRow::new();
    let box_c = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    box_c.set_margin_top(4);
    box_c.set_margin_bottom(4);
    box_c.set_margin_start(8);
    box_c.set_margin_end(8);

    let icon_c = gtk::Image::from_icon_name("system-users-symbolic");
    let lbl_c = gtk::Label::new(Some(&crate::i18n::tr("Contacts")));
    lbl_c.add_css_class("heading");

    box_c.append(&icon_c);
    box_c.append(&lbl_c);
    row_contacts.set_child(Some(&box_c));
    row_contacts.set_tooltip_text(Some(&crate::i18n::tr("Contacts & colleagues directory")));
    list_box.append(&row_contacts);

    // Row 3: Voicemail
    let row_vb = gtk::ListBoxRow::new();
    let box_v = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    box_v.set_margin_top(4);
    box_v.set_margin_bottom(4);
    box_v.set_margin_start(8);
    box_v.set_margin_end(8);

    let icon_v = gtk::Image::from_icon_name("media-tape-symbolic");
    let lbl_v = gtk::Label::new(Some(&crate::i18n::tr("Voicemail")));
    lbl_v.add_css_class("heading");
    let sp_v = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    sp_v.set_hexpand(true);

    let badge_vb = gtk::Label::new(Some("0"));
    badge_vb.add_css_class("sidebar-badge");
    badge_vb.set_visible(false);

    box_v.append(&icon_v);
    box_v.append(&lbl_v);
    box_v.append(&sp_v);
    box_v.append(&badge_vb);
    row_vb.set_child(Some(&box_v));
    row_vb.set_tooltip_text(Some(&crate::i18n::tr("Voicemail messages")));
    list_box.append(&row_vb);

    let b_gesture = badge_j.clone();
    let r_gesture = row_journal.clone();
    let gesture_j = gtk::GestureClick::new();
    let p_gj = popover_holder.clone();
    let is_typing_gj = is_user_typing.clone();
    gesture_j.connect_released(move |_, _, _, _| {
        is_typing_gj.set(false);
        if let Some(p) = p_gj.borrow().as_ref() {
            p.popdown();
        }
        b_gesture.set_visible(false);
        r_gesture.set_tooltip_text(Some(&crate::i18n::tr("Call Journal")));
    });
    row_journal.add_controller(gesture_j);

    let on_sel_clone = on_select_page.clone();
    let b_act = badge_j.clone();
    let r_act = row_journal.clone();
    let p_list = popover_holder.clone();
    let is_typing_nav = is_user_typing.clone();
    list_box.connect_row_activated(move |_, row| {
        is_typing_nav.set(false);
        if let Some(p) = p_list.borrow().as_ref() {
            p.popdown();
        }
        let idx = row.index();
        match idx {
            0 => {
                b_act.set_visible(false);
                r_act.set_tooltip_text(Some(&crate::i18n::tr("Call Journal")));
                on_sel_clone("journal");
            }
            1 => {
                on_sel_clone("contacts");
            }
            2 => {
                on_sel_clone("voicebox");
            }
            _ => (),
        }
    });

    let list_click = gtk::GestureClick::new();
    let p_lc = popover_holder.clone();
    let is_typing_lc = is_user_typing.clone();
    list_click.connect_pressed(move |_, _, _, _| {
        is_typing_lc.set(false);
        if let Some(p) = p_lc.borrow().as_ref() {
            p.popdown();
        }
    });
    list_box.add_controller(list_click);

    list_box.select_row(Some(&row_journal));
    container.append(&list_box);

    // Middle area between pinned navigation and pinned bottom Dialer:
    // holds inline Speed Dial (in 1- and 2-pane modes) or an expanding spacer
    // (when Speed Dial detaches into its own right-hand utility pane on wide windows),
    // plus the optional docked active-call card.
    let middle_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    middle_box.set_vexpand(true);
    middle_box.set_hexpand(true);
    container.append(&middle_box);

    let inline_speed_dial_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    inline_speed_dial_box.set_vexpand(true);
    inline_speed_dial_box.set_hexpand(true);

    // 2b. Inline Kurzwahl / Team-BLF direkt in der linken Sidebar (1- & 2-Spalten-Layout)
    let hk_section_header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    hk_section_header.add_css_class("sidebar-section-header");
    hk_section_header.set_margin_start(16);
    hk_section_header.set_margin_end(12);
    hk_section_header.set_margin_top(4);
    hk_section_header.set_margin_bottom(2);

    let hk_title_lbl = gtk::Label::new(Some(&crate::i18n::tr("Speed Dial")));
    hk_title_lbl.add_css_class("caption-heading");
    hk_title_lbl.add_css_class("dim-label");
    hk_title_lbl.set_hexpand(true);
    hk_title_lbl.set_halign(gtk::Align::Start);
    hk_title_lbl.set_valign(gtk::Align::Center);
    hk_section_header.append(&hk_title_lbl);

    let initial_hotkeys_open = !initial_dialpad_visible;
    let speed_dial_was_open_before_dialpad = Rc::new(Cell::new(initial_dialpad_visible));

    let hotkeys_toggle_btn = gtk::ToggleButton::new();
    hotkeys_toggle_btn.set_icon_name(if initial_hotkeys_open {
        "pan-down-symbolic"
    } else {
        "pan-end-symbolic"
    });
    hotkeys_toggle_btn.add_css_class("flat");
    hotkeys_toggle_btn.add_css_class("circular");
    hotkeys_toggle_btn.add_css_class("sidebar-section-toggle");
    hotkeys_toggle_btn.set_valign(gtk::Align::Center);
    hotkeys_toggle_btn.set_active(initial_hotkeys_open);
    hotkeys_toggle_btn.set_tooltip_text(Some(&crate::i18n::tr("Show or hide speed dial")));
    hotkeys_toggle_btn.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr("Speed Dial"))]);
    hk_section_header.append(&hotkeys_toggle_btn);
    inline_speed_dial_box.append(&hk_section_header);

    let hotkeys_revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .transition_duration(180)
        .reveal_child(initial_hotkeys_open)
        .vexpand(true)
        .hexpand(true)
        .build();

    let rev_hk_toggle = hotkeys_revealer.clone();
    hotkeys_toggle_btn.connect_toggled(move |btn| {
        let active = btn.is_active();
        rev_hk_toggle.set_reveal_child(active);
        btn.set_icon_name(if active {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        });
    });

    // Only the Speed Dial items list scrolls (and only when items actually overflow),
    // never the main navigation rows above it.
    let inline_hotkeys_scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .min_content_height(0)
        .propagate_natural_height(false)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .overlay_scrolling(true)
        .has_frame(false)
        .build();
    inline_hotkeys_scroll.add_css_class("hotkeys-scroller");

    let hotkey_list_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
    hotkey_list_box.set_margin_start(10);
    hotkey_list_box.set_margin_end(10);
    hotkey_list_box.set_margin_top(2);
    hotkey_list_box.set_margin_bottom(6);
    hotkey_list_box.set_valign(gtk::Align::Start);
    hotkey_list_box.set_hexpand(true);

    let p_space = popover_holder.clone();
    let is_typing_space = is_user_typing.clone();
    let space_click = gtk::GestureClick::new();
    space_click.connect_pressed(move |_, _, _, _| {
        is_typing_space.set(false);
        if let Some(p) = p_space.borrow().as_ref() {
            p.popdown();
        }
    });
    inline_hotkeys_scroll.add_controller(space_click);

    inline_hotkeys_scroll.set_child(Some(&hotkey_list_box));
    hotkeys_revealer.set_child(Some(&inline_hotkeys_scroll));
    inline_speed_dial_box.append(&hotkeys_revealer);
    middle_box.append(&inline_speed_dial_box);

    let detached_spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    detached_spacer.set_vexpand(true);
    detached_spacer.set_visible(false);
    middle_box.append(&detached_spacer);

    // 2c. Dedicated right-hand Speed Dial utility pane (shown on wide windows >= 900px)
    let speed_dial_pane = adw::ToolbarView::new();
    speed_dial_pane.add_css_class("speed-dial-pane");
    speed_dial_pane.set_width_request(235);
    let right_header = adw::HeaderBar::new();
    right_header.set_show_start_title_buttons(false);
    right_header.set_show_end_title_buttons(true);
    let right_title = adw::WindowTitle::new(&crate::i18n::tr("Speed Dial"), "");
    right_header.set_title_widget(Some(&right_title));
    speed_dial_pane.add_top_bar(&right_header);

    let right_hotkeys_scroll = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .min_content_height(0)
        .propagate_natural_height(false)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .overlay_scrolling(true)
        .has_frame(false)
        .build();
    right_hotkeys_scroll.add_css_class("hotkeys-scroller");
    let p_space_r = popover_holder.clone();
    let is_typing_space_r = is_user_typing.clone();
    let space_click_r = gtk::GestureClick::new();
    space_click_r.connect_pressed(move |_, _, _, _| {
        is_typing_space_r.set(false);
        if let Some(p) = p_space_r.borrow().as_ref() {
            p.popdown();
        }
    });
    right_hotkeys_scroll.add_controller(space_click_r);
    speed_dial_pane.set_content(Some(&right_hotkeys_scroll));

    let is_speed_dial_detached = Rc::new(Cell::new(false));
    let is_short_height = Rc::new(Cell::new(false));
    let dialpad_toggle_holder: Rc<RefCell<Option<gtk::ToggleButton>>> = Rc::new(RefCell::new(None));
    let set_speed_dial_detached: Rc<dyn Fn(bool)> = {
        let is_detached = is_speed_dial_detached.clone();
        let is_short = is_short_height.clone();
        let dp_holder = dialpad_toggle_holder.clone();
        let hk_btn = hotkeys_toggle_btn.clone();
        let hk_rev = hotkeys_revealer.clone();
        let was_open = speed_dial_was_open_before_dialpad.clone();
        let inline_box = inline_speed_dial_box.clone();
        let spacer = detached_spacer.clone();
        let inline_scroll = inline_hotkeys_scroll.clone();
        let right_scroll = right_hotkeys_scroll.clone();
        let list = hotkey_list_box.clone();
        Rc::new(move |detached: bool| {
            if is_detached.get() == detached {
                return;
            }
            is_detached.set(detached);
            if detached {
                inline_box.set_visible(false);
                spacer.set_visible(true);
                inline_scroll.set_child(gtk::Widget::NONE);
                list.set_margin_top(6);
                list.set_margin_bottom(10);
                right_scroll.set_child(Some(&list));
            } else {
                // If returning to inline mode on a short window while the dialpad is open,
                // fold inline Speed Dial immediately (without animation lag) so the Call button
                // can never be pushed off-screen.
                let dp_open = dp_holder
                    .borrow()
                    .as_ref()
                    .is_some_and(|b| b.is_active());
                if dp_open && is_short.get() && hk_btn.is_active() {
                    was_open.set(true);
                    hk_rev.set_transition_duration(0);
                    hk_btn.set_active(false);
                    hk_rev.set_reveal_child(false);
                    hk_rev.set_transition_duration(180);
                }
                right_scroll.set_child(gtk::Widget::NONE);
                list.set_margin_top(2);
                list.set_margin_bottom(6);
                inline_scroll.set_child(Some(&list));
                spacer.set_visible(false);
                inline_box.set_visible(true);
            }
        })
    };

    struct HotkeyItemRef {
        entity_id: i64,
        number: String,
        raw_name: String,
        display_name: String,
        led: gtk::Box,
        btn: gtk::Button,
        live_presence: Rc<Cell<PresenceState>>,
    }

    fn apply_hotkey_presence(
        led: &gtk::Box,
        hk_btn: &gtk::Button,
        display_name: &str,
        number: &str,
        presence: PresenceState,
    ) {
        crate::ui::update_presence_dot(led, Some(presence));
        hk_btn.remove_css_class("hotkey-chip-busy");
        hk_btn.remove_css_class("hotkey-chip-ringing");
        match presence {
            PresenceState::Busy => hk_btn.add_css_class("hotkey-chip-busy"),
            PresenceState::Ringing => hk_btn.add_css_class("hotkey-chip-ringing"),
            _ => {}
        }
        let status_color = match presence {
            PresenceState::Available => "#2ec27e",
            PresenceState::Busy => "#e01b24",
            PresenceState::Away => "#f6d32d",
            PresenceState::LoggedOut => "#77767b",
            PresenceState::Ringing => "#ff7800",
        };
        let action_hint = if presence == PresenceState::Ringing {
            crate::i18n::tr("Ringing! Click to pick up • Drag a line here or right-click to transfer")
        } else {
            crate::i18n::tr("Click to call • Drag a line here or right-click to transfer")
        };
        let tip = format!(
            "<b>{}</b> ({})\nStatus: <span foreground=\"{}\">{}</span>\n<span size=\"small\" alpha=\"75%\">{}</span>",
            gtk::glib::markup_escape_text(display_name),
            gtk::glib::markup_escape_text(number),
            status_color,
            presence.label(),
            action_hint
        );
        hk_btn.set_tooltip_markup(Some(&tip));
    }

    let hk_list_pop = hotkey_list_box.clone();
    let sd_pop = speed_dials.clone();
    let contacts_hk = contacts.clone();
    let toast_hk = toast_overlay.clone();
    let on_hk_click = on_hotkey_click.clone();
    let on_hk_transfer = on_transfer_call.clone();
    let on_hk_copy = on_copy_number.clone();
    let on_hk_fav = on_fav_changed.clone();
    let on_hk_chat = on_open_chat.clone();
    let hk_reload_holder: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let hk_reload_self = hk_reload_holder.clone();
    let hotkey_items: Rc<RefCell<Vec<HotkeyItemRef>>> = Rc::new(RefCell::new(Vec::new()));
    let hk_items_pop = hotkey_items.clone();

    let populate_hotkeys: Rc<dyn Fn()> = Rc::new(move || {
        while let Some(child) = hk_list_pop.first_child() {
            hk_list_pop.remove(&child);
        }
        hk_items_pop.borrow_mut().clear();

        let sds = sd_pop.borrow().clone();
        let c_list = contacts_hk.borrow();

        if sds.is_empty() {
            let empty_lbl = gtk::Label::new(Some(&crate::i18n::tr(
                "No speed dials yet.\nAdd contacts via ★ in Contacts or Call Journal.",
            )));
            empty_lbl.add_css_class("caption");
            empty_lbl.add_css_class("dim-label");
            empty_lbl.set_wrap(true);
            empty_lbl.set_justify(gtk::Justification::Center);
            empty_lbl.set_margin_top(24);
            empty_lbl.set_margin_start(8);
            empty_lbl.set_margin_end(8);
            hk_list_pop.append(&empty_lbl);
            return;
        }

        for sd in sds.iter() {
            let target_id = format!("swyx-{}", sd.entity_id);
            let matched = c_list.iter().find(|c| {
                (sd.entity_id > 0 && c.id == target_id)
                    || (!sd.number.is_empty() && c.extension == sd.number)
                    || (!sd.name.is_empty() && c.name == sd.name)
            });

            let presence = matched
                .map(|c| c.presence)
                .unwrap_or_else(|| PresenceState::from_swyx_clmgr_state(sd.state));

            let raw_name = if !sd.name.is_empty() {
                &sd.name
            } else if let Some(c) = matched {
                &c.name
            } else {
                &sd.number
            };
            let display_name = format_hotkey_display_name(raw_name);
            let number = sd.number.clone();

            let chip_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            chip_box.set_valign(gtk::Align::Center);
            chip_box.set_hexpand(true);

            let led = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            led.add_css_class("hotkey-led");
            led.set_valign(gtk::Align::Center);
            chip_box.append(&led);

            let name_lbl = gtk::Label::new(Some(&display_name));
            name_lbl.add_css_class("hotkey-name");
            name_lbl.set_hexpand(true);
            name_lbl.set_halign(gtk::Align::Start);
            name_lbl.set_ellipsize(gtk::pango::EllipsizeMode::End);
            chip_box.append(&name_lbl);

            let hk_btn = gtk::Button::new();
            hk_btn.set_child(Some(&chip_box));
            hk_btn.add_css_class("flat");
            hk_btn.add_css_class("hotkey-chip");
            hk_btn.set_hexpand(true);

            apply_hotkey_presence(&led, &hk_btn, &display_name, &number, presence);
            let live_presence = Rc::new(Cell::new(presence));

            // Linksklick: Anrufen (oder Heranholen wenn Kollege gerade klingelt)
            let on_click = on_hk_click.clone();
            let c_name_click = display_name.clone();
            let c_num_click = number.clone();
            let lp_click = live_presence.clone();
            hk_btn.connect_clicked(move |_| {
                if !c_num_click.is_empty() {
                    on_click(c_name_click.clone(), c_num_click.clone(), lp_click.get());
                }
            });

            hk_items_pop.borrow_mut().push(HotkeyItemRef {
                entity_id: sd.entity_id,
                number: number.clone(),
                raw_name: sd.name.clone(),
                display_name: display_name.clone(),
                led: led.clone(),
                btn: hk_btn.clone(),
                live_presence,
            });

            // Drag & Drop Ziel: Leitung direkt auf Hotkey ziehen zum Weiterleiten
            crate::ui::attach_call_drop_target(
                &hk_btn,
                display_name.clone(),
                number.clone(),
                on_hk_transfer.clone(),
            );

            // Rechtsklick-Kontextmenü auf Hotkey (Telefonat abgeben, Nummer kopieren, Aus Kurzwahl entfernen)
            let pop = gtk::Popover::new();
            pop.set_parent(&hk_btn);
            pop.set_has_arrow(true);
            let pop_unparent = pop.clone();
            hk_btn.connect_destroy(move |_| {
                pop_unparent.unparent();
            });

            let m_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            m_box.set_margin_top(4);
            m_box.set_margin_bottom(4);
            m_box.set_margin_start(4);
            m_box.set_margin_end(4);

            let trans_btn = gtk::Button::new();
            trans_btn.add_css_class("flat");
            trans_btn.add_css_class("transfer-context-btn");
            let t_box = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            let t_icon = gtk::Image::from_icon_name("mail-forward-symbolic");
            t_icon.set_pixel_size(14);
            let t_lbl = gtk::Label::new(Some(&crate::i18n::tr("Transfer Call")));
            t_lbl.add_css_class("caption");
            t_lbl.add_css_class("heading");
            t_box.append(&t_icon);
            t_box.append(&t_lbl);
            trans_btn.set_child(Some(&t_box));
            trans_btn.set_tooltip_text(Some(&crate::i18n::trf(
                "Transfer current or ringing call directly to {name} ({number})",
                &[("name", &display_name), ("number", &number)],
            )));
            let p_tr = pop.clone();
            let on_tr = on_hk_transfer.clone();
            let n_tr = display_name.clone();
            let num_tr = number.clone();
            trans_btn.connect_clicked(move |_| {
                p_tr.popdown();
                on_tr(n_tr.clone(), num_tr.clone());
            });
            m_box.append(&trans_btn);

            // Sofortnachricht / Chat (falls interner Swyx Kontakt)
            let is_swyx_internal = crate::models::is_internal_extension(&number)
                || matched.as_ref().map(|c| c.is_internal()).unwrap_or(false);

            if is_swyx_internal {
                let chat_btn = gtk::Button::from_icon_name("chat-message-new-symbolic");
                chat_btn.add_css_class("flat");
                chat_btn.add_css_class("circular");
                chat_btn.set_tooltip_text(Some(&format!("Sofortnachricht an {}", display_name)));

                let p_ch = pop.clone();
                let on_ch = on_hk_chat.clone();
                let n_ch = display_name.clone();
                let num_ch = number.clone();
                chat_btn.connect_clicked(move |_| {
                    p_ch.popdown();
                    on_ch(n_ch.clone(), num_ch.clone());
                });
                m_box.append(&chat_btn);
            }

            let copy_btn = gtk::Button::from_icon_name("edit-copy-symbolic");
            copy_btn.add_css_class("flat");
            copy_btn.add_css_class("circular");
            copy_btn.set_tooltip_text(Some(&crate::i18n::tr("Copy number")));
            let p_cp = pop.clone();
            let on_cp = on_hk_copy.clone();
            let num_cp = number.clone();
            copy_btn.connect_clicked(move |_| {
                p_cp.popdown();
                on_cp(num_cp.clone());
            });
            m_box.append(&copy_btn);

            let rem_btn = gtk::Button::from_icon_name("starred-symbolic");
            rem_btn.add_css_class("flat");
            rem_btn.add_css_class("circular");
            rem_btn.add_css_class("favorite-star-gold");
            rem_btn.set_tooltip_text(Some(&crate::i18n::tr("Remove from speed dial / favorites")));
            let p_rm = pop.clone();
            let sd_rm = sd_pop.clone();
            let toast_rm = toast_hk.clone();
            let on_fav_rm = on_hk_fav.clone();
            let rl_self = hk_reload_self.clone();
            let n_rm = display_name.clone();
            let raw_rm = sd.name.clone();
            let num_rm = number.clone();
            rem_btn.connect_clicked(move |_| {
                p_rm.popdown();
                sd_rm.borrow_mut().retain(|item| {
                    let num_m = !item.number.is_empty() && item.number == num_rm;
                    let name_m = !item.name.is_empty() && (item.name == raw_rm || item.name == n_rm);
                    !(num_m || name_m)
                });
                let t = adw::Toast::new(&crate::i18n::trf("{name} removed from favorites", &[("name", &n_rm)]));
                t.set_timeout(2);
                toast_rm.add_toast(t);
                if let Some(r) = rl_self.borrow().as_ref() {
                    r();
                }
                on_fav_rm();
            });
            m_box.append(&rem_btn);

            pop.set_child(Some(&m_box));

            let p_rc = pop.clone();
            let rc = gtk::GestureClick::new();
            rc.set_button(3);
            rc.connect_pressed(move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let rect = gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
                p_rc.set_pointing_to(Some(&rect));
                p_rc.popup();
            });
            hk_btn.add_controller(rc);

            hk_list_pop.append(&hk_btn);
        }
    });

    *hk_reload_holder.borrow_mut() = Some(populate_hotkeys.clone());
    populate_hotkeys();

    let hk_items_pres = hotkey_items.clone();
    let contacts_hk_pres = contacts.clone();
    let sd_hk_pres = speed_dials.clone();
    let refresh_hotkeys_presence: Rc<dyn Fn()> = Rc::new(move || {
        let c_list = contacts_hk_pres.borrow();
        let sds = sd_hk_pres.borrow();
        for item in hk_items_pres.borrow().iter() {
            let target_id = format!("swyx-{}", item.entity_id);
            let matched = c_list.iter().find(|c| {
                (item.entity_id > 0 && c.id == target_id)
                    || (!item.number.is_empty() && c.extension == item.number)
                    || (!item.raw_name.is_empty() && c.name == item.raw_name)
            });
            let fallback_state = sds
                .iter()
                .find(|sd| {
                    (item.entity_id > 0 && sd.entity_id == item.entity_id)
                        || (!item.number.is_empty() && sd.number == item.number)
                })
                .map(|sd| PresenceState::from_swyx_clmgr_state(sd.state))
                .unwrap_or(PresenceState::LoggedOut);
            let new_pres = matched.map(|c| c.presence).unwrap_or(fallback_state);
            if item.live_presence.get() != new_pres {
                item.live_presence.set(new_pres);
                apply_hotkey_presence(
                    &item.led,
                    &item.btn,
                    &item.display_name,
                    &item.number,
                    new_pres,
                );
            }
        }
    });

    // 3. Docked Active Call Card (When in call)
    let docked_call_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    docked_call_box.add_css_class("docked-call-card");
    docked_call_box.set_margin_start(12);
    docked_call_box.set_margin_end(12);
    docked_call_box.set_margin_bottom(12);
    docked_call_box.set_visible(false); // Shown dynamically when call starts
    crate::ui::attach_call_drag_source(&docked_call_box);

    // Zweitanruf / Rückfrage-Banner in Docked Card (1-Klick Makeln & Verbinden)
    let docked_held_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    docked_held_box.add_css_class("docked-held-banner");
    docked_held_box.set_visible(false);
    let hld_drag_dock = held_call.clone();
    crate::ui::attach_call_drag_source_with_tag(&docked_held_box, "CALL_TRANSFER_HELD", move || {
        hld_drag_dock.borrow().is_some()
    });

    let docked_held_label = gtk::Label::new(Some(&crate::i18n::tr("On Hold")));
    docked_held_label.add_css_class("caption");
    docked_held_label.add_css_class("heading");
    docked_held_label.set_hexpand(true);
    docked_held_label.set_halign(gtk::Align::Start);
    docked_held_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    docked_held_box.append(&docked_held_label);

    let docked_held_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    docked_held_actions.set_homogeneous(true);

    let docked_swap_btn = gtk::Button::with_label(&crate::i18n::tr("Swap"));
    docked_swap_btn.add_css_class("swap-btn-minimal");
    docked_swap_btn.set_tooltip_text(Some(&crate::i18n::tr("Swap between active and held call")));
    docked_held_actions.append(&docked_swap_btn);

    let docked_transfer_held_btn = gtk::Button::with_label(&crate::i18n::tr("Merge"));
    docked_transfer_held_btn.add_css_class("connect-transfer-btn");
    docked_transfer_held_btn.set_tooltip_text(Some(&crate::i18n::tr("Connect both calls (attended transfer)")));
    docked_held_actions.append(&docked_transfer_held_btn);

    docked_held_box.append(&docked_held_actions);
    docked_call_box.append(&docked_held_box);

    // Active Call Main Row
    let active_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let avatar_icon = gtk::Image::from_icon_name("avatar-default-symbolic");
    avatar_icon.set_pixel_size(32);
    active_row.append(&avatar_icon);

    let text_col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text_col.set_hexpand(true);
    let docked_caller_label = gtk::Label::new(Some("Max Mustermann (101)"));
    docked_caller_label.add_css_class("heading");
    docked_caller_label.set_halign(gtk::Align::Fill);
    docked_caller_label.set_xalign(0.0);
    docked_caller_label.set_hexpand(true);
    docked_caller_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    docked_caller_label.set_max_width_chars(18);
    text_col.append(&docked_caller_label);

    let docked_ext_label = gtk::Label::new(Some(&crate::i18n::tr("Line 1 • HD")));
    docked_ext_label.add_css_class("caption");
    docked_ext_label.add_css_class("dim-label");
    docked_ext_label.set_halign(gtk::Align::Fill);
    docked_ext_label.set_xalign(0.0);
    docked_ext_label.set_hexpand(true);
    docked_ext_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    docked_ext_label.set_max_width_chars(18);
    text_col.append(&docked_ext_label);
    active_row.append(&text_col);

    let docked_timer_label = gtk::Label::new(Some("00:00"));
    docked_timer_label.add_css_class("heading");
    docked_timer_label.add_css_class("numeric");
    docked_timer_label.set_valign(gtk::Align::Center);
    active_row.append(&docked_timer_label);

    docked_call_box.append(&active_row);

    // Buttons: Mute & Hangup
    let btns_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    btns_row.set_halign(gtk::Align::Center);

    let docked_mute_btn = gtk::Button::from_icon_name("audio-input-microphone-symbolic");
    docked_mute_btn.add_css_class("flat");
    docked_mute_btn.add_css_class("circular");
    docked_mute_btn.set_tooltip_text(Some(&crate::i18n::tr("Mute")));
    docked_mute_btn.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr("Mute"))]);
    btns_row.append(&docked_mute_btn);

    let docked_dtmf_btn = gtk::Button::from_icon_name("input-dialpad-symbolic");
    docked_dtmf_btn.add_css_class("flat");
    docked_dtmf_btn.add_css_class("circular");
    docked_dtmf_btn.set_tooltip_text(Some(&crate::i18n::tr("Keypad (DTMF for voice menus)")));
    docked_dtmf_btn.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr("Keypad"))]);
    btns_row.append(&docked_dtmf_btn);

    let docked_hangup_btn = gtk::Button::from_icon_name("call-stop-symbolic");
    docked_hangup_btn.add_css_class("call-btn-destructive");
    docked_hangup_btn.set_tooltip_text(Some(&crate::i18n::tr("Hang Up")));
    docked_hangup_btn.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr("Hang Up"))]);
    btns_row.append(&docked_hangup_btn);

    let p_dock = popover_holder.clone();
    let is_typing_dock = is_user_typing.clone();
    let dock_click = gtk::GestureClick::new();
    dock_click.connect_pressed(move |_, _, _, _| {
        is_typing_dock.set(false);
        if let Some(p) = p_dock.borrow().as_ref() {
            p.popdown();
        }
    });
    docked_call_box.add_controller(dock_click);

    docked_call_box.append(&btns_row);

    // The docked call card mirrors the main in-call page; it is hidden while that page is visible.
    let docked_call_revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideUp)
        .reveal_child(true)
        .child(&docked_call_box)
        .build();
    middle_box.append(&docked_call_revealer);

    // 4. Universal Schnellwahl (kompakt am unteren Rand verankert)
    let dial_box = gtk::Box::new(gtk::Orientation::Vertical, 5);
    dial_box.set_margin_start(12);
    dial_box.set_margin_end(12);
    dial_box.set_margin_top(8);
    dial_box.set_margin_bottom(10);

    // A. Überschrift "Rufnummer" mit ausfahrbarem Wähltasten-Toggle
    let dial_header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let dial_title = gtk::Label::new(Some(&crate::i18n::tr("Dial")));
    dial_title.add_css_class("heading");
    dial_title.set_hexpand(true);
    dial_title.set_halign(gtk::Align::Start);

    let dialpad_toggle = gtk::ToggleButton::new();
    dialpad_toggle.set_icon_name("input-dialpad-symbolic");
    dialpad_toggle.add_css_class("flat");
    dialpad_toggle.add_css_class("circular");
    dialpad_toggle.add_css_class("sidebar-dialpad-toggle");
    dialpad_toggle.set_valign(gtk::Align::Center);
    dialpad_toggle.set_tooltip_text(Some(&crate::i18n::tr("Show or hide dialpad")));
    dialpad_toggle.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr("Keypad"))]);

    dial_header.append(&dial_title);
    dial_header.append(&dialpad_toggle);
    dial_box.append(&dial_header);

    // B. Input field "Number or name"
    let search_entry = gtk::SearchEntry::new();
    search_entry.set_placeholder_text(Some(&crate::i18n::tr("Number or name")));
    search_entry.set_width_chars(10);
    search_entry.set_hexpand(true);
    search_entry.set_halign(gtk::Align::Fill);
    search_entry.add_css_class("search-dial-entry");

    // Autocomplete Popover (Öffnet OBERHALB des Eingabefelds)
    let popover = gtk::Popover::new();
    popover.set_parent(&search_entry);
    popover.set_position(gtk::PositionType::Top);
    popover.set_autohide(false); // false prevents Wayland compositor seat-grab lockups
    popover.set_focusable(false);
    popover.add_css_class("autocomplete-popover");
    *popover_holder.borrow_mut() = Some(popover.clone());

    let pop_list = gtk::ListBox::new();
    pop_list.add_css_class("autocomplete-list");
    pop_list.set_selection_mode(gtk::SelectionMode::None);

    let pop_scroll = gtk::ScrolledWindow::builder()
        .child(&pop_list)
        .max_content_height(220)
        .propagate_natural_height(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .has_frame(false)
        .build();
    pop_scroll.set_size_request(260, -1);
    popover.set_child(Some(&pop_scroll));

    // Pre-allocated slot pool (up to 4 items) for 0ms lag-free typing
    struct AutocompleteSlot {
        row: adw::ActionRow,
        avatar: adw::Avatar,
        dot: gtk::Box,
        ext_target: Rc<RefCell<String>>,
    }

    struct CachedSearchEntry {
        name: String,
        name_lower: String,
        extension: String,
        phone: String,
        dept_lower: String,
        department: String,
        presence: crate::models::PresenceState,
    }

    let mut slots = Vec::new();
    for _ in 0..4 {
        let ext_target = Rc::new(RefCell::new(String::new()));
        let row = adw::ActionRow::new();
        row.set_activatable(true);
        row.set_focusable(false);
        row.set_visible(false);

        let avatar = adw::Avatar::new(32, Some(""), true);
        avatar.set_size_request(32, 32);
        avatar.set_valign(gtk::Align::Start);
        avatar.set_halign(gtk::Align::Start);

        let overlay = gtk::Overlay::new();
        overlay.set_size_request(34, 34);
        overlay.set_valign(gtk::Align::Center);
        overlay.set_halign(gtk::Align::Center);
        overlay.set_overflow(gtk::Overflow::Visible);
        overlay.set_child(Some(&avatar));

        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("presence-badge-dot");
        dot.set_halign(gtk::Align::End);
        dot.set_valign(gtk::Align::End);
        overlay.add_overlay(&dot);

        row.add_prefix(&overlay);

        let se_clone = search_entry.clone();
        let p_clone = popover.clone();
        let ext_ref = ext_target.clone();
        let is_typing_act = is_user_typing.clone();
        row.connect_activated(move |_| {
            is_typing_act.set(false);
            let num = ext_ref.borrow().clone();
            if !num.is_empty() {
                se_clone.set_text(&num);
                se_clone.grab_focus();
                se_clone.set_position(-1);
                p_clone.popdown();
            }
        });

        pop_list.append(&row);
        slots.push(AutocompleteSlot {
            row,
            avatar,
            dot,
            ext_target,
        });
    }

    // A. Key Controller: Detect when the user is actively typing in the field
    let key_ctrl = gtk::EventControllerKey::new();
    let is_typing_key = is_user_typing.clone();
    key_ctrl.connect_key_pressed(move |_, _keyval, _keycode, _state| {
        is_typing_key.set(true);
        gtk::glib::Propagation::Proceed
    });
    search_entry.add_controller(key_ctrl);

    // B. Focus Controller: Dismiss popover when focus leaves search entry
    let focus_ctrl = gtk::EventControllerFocus::new();
    let p_focus = popover.clone();
    let is_typing_focus = is_user_typing.clone();
    focus_ctrl.connect_leave(move |_| {
        is_typing_focus.set(false);
        p_focus.popdown();
    });

    // C. Contains-focus notify on FocusController
    let p_cf = popover.clone();
    let is_typing_cf = is_user_typing.clone();
    focus_ctrl.connect_contains_focus_notify(move |ctrl| {
        if !ctrl.contains_focus() {
            is_typing_cf.set(false);
            p_cf.popdown();
        }
    });
    search_entry.add_controller(focus_ctrl);

    // D. Escape key / stop search cancels autocomplete
    let p_esc = popover.clone();
    let is_typing_esc = is_user_typing.clone();
    search_entry.connect_stop_search(move |_| {
        is_typing_esc.set(false);
        p_esc.popdown();
    });

    let contacts_search = contacts.clone();
    let pop_ref = popover.clone();
    let slots_ref = Rc::new(slots);
    let search_cache: Rc<RefCell<Vec<CachedSearchEntry>>> = Rc::new(RefCell::new(Vec::new()));
    let debounce_source: Rc<RefCell<Option<gtk::glib::SourceId>>> = Rc::new(RefCell::new(None));
    let db_ref = debounce_source.clone();
    let is_typing_sc = is_user_typing.clone();

    search_entry.connect_search_changed(move |entry| {
        if let Some(src) = db_ref.borrow_mut().take() {
            src.remove();
        }

        let query = entry.text().trim().to_lowercase();
        let p_pop = pop_ref.clone();
        let s_slots = slots_ref.clone();
        let c_search = contacts_search.clone();
        let s_cache = search_cache.clone();
        let db_inner = db_ref.clone();
        let is_typing_inner = is_typing_sc.clone();

        let src = gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(30), move || {
            db_inner.borrow_mut().take();

            if query.is_empty() {
                for s in s_slots.iter() {
                    s.row.set_visible(false);
                }
                p_pop.popdown();
                return;
            }

            // Only show suggestions when actively typing
            if !is_typing_inner.get() {
                p_pop.popdown();
                return;
            }

            // Maintain zero-allocation lowercased contact index (including live presence updates)
            let mut cache = s_cache.borrow_mut();
            let c_borrow = c_search.borrow();
            if cache.len() != c_borrow.len() {
                cache.clear();
                for c in c_borrow.iter() {
                    cache.push(CachedSearchEntry {
                        name: c.name.clone(),
                        name_lower: c.name.to_lowercase(),
                        extension: c.extension.clone(),
                        phone: c.phone.clone(),
                        dept_lower: c.department.to_lowercase(),
                        department: c.department.clone(),
                        presence: c.presence,
                    });
                }
            } else {
                for (ce, c) in cache.iter_mut().zip(c_borrow.iter()) {
                    ce.presence = c.presence;
                    if ce.name != c.name {
                        ce.name = c.name.clone();
                        ce.name_lower = c.name.to_lowercase();
                    }
                    if ce.extension != c.extension {
                        ce.extension = c.extension.clone();
                    }
                    if ce.phone != c.phone {
                        ce.phone = c.phone.clone();
                    }
                    if ce.department != c.department {
                        ce.department = c.department.clone();
                        ce.dept_lower = c.department.to_lowercase();
                    }
                }
            }
            drop(c_borrow);

            let mut count = 0;
            for c in cache.iter() {
                if c.name_lower.contains(&query)
                    || c.extension.contains(&query)
                    || c.phone.contains(&query)
                    || c.dept_lower.contains(&query)
                {
                    let slot = &s_slots[count];
                    slot.row.set_title(&c.name);
                    slot.row.set_subtitle(&format!("{} • {}", c.extension, c.department));

                    let clean = crate::ui::clean_name_for_avatar(&c.name);
                    slot.avatar.set_text(Some(&clean));
                    if let Some(texture) = crate::ui::get_contact_photo(&c.name) {
                        slot.avatar.set_custom_image(Some(&texture));
                    } else {
                        slot.avatar.set_custom_image(None::<&gtk::gdk::Texture>);
                    }

                    slot.dot.remove_css_class("presence-available");
                    slot.dot.remove_css_class("presence-busy");
                    slot.dot.remove_css_class("presence-away");
                    slot.dot.remove_css_class("presence-offline");
                    slot.dot.remove_css_class("presence-empty");
                    slot.dot.remove_css_class("presence-ringing");
                    slot.dot.add_css_class(c.presence.css_class());

                    *slot.ext_target.borrow_mut() = if !c.extension.is_empty() {
                        c.extension.clone()
                    } else {
                        c.phone.clone()
                    };

                    slot.row.set_visible(true);
                    count += 1;
                    if count >= 4 {
                        break;
                    }
                }
            }

            for i in count..4 {
                s_slots[i].row.set_visible(false);
            }

            if count > 0 && is_typing_inner.get() {
                if !p_pop.is_visible() {
                    p_pop.popup();
                }
            } else if p_pop.is_visible() {
                p_pop.popdown();
            }
        });

        *db_ref.borrow_mut() = Some(src);
    });

    dial_box.append(&search_entry);

    // B2. Ausfahrbares Wähltastenfeld (3x4 Grid)
    let dialpad_revealer = gtk::Revealer::new();
    dialpad_revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    dialpad_revealer.set_transition_duration(200);
    dialpad_revealer.set_reveal_child(initial_dialpad_visible);
    dialpad_toggle.set_active(initial_dialpad_visible);

    *dialpad_toggle_holder.borrow_mut() = Some(dialpad_toggle.clone());

    let rev_toggle = dialpad_revealer.clone();
    let on_dp_tgl = on_dialpad_toggle.clone();
    let hk_btn_for_dp = hotkeys_toggle_btn.clone();
    let was_open_dp = speed_dial_was_open_before_dialpad.clone();
    let detached_dp = is_speed_dial_detached.clone();
    let short_dp = is_short_height.clone();
    let container_dp = container.clone();
    dialpad_toggle.connect_toggled(move |btn| {
        let is_active = btn.is_active();
        // On compact-height windows in 1-/2-pane mode, fold inline Speed Dial smoothly
        // when opening the keypad (and restore it when closing) so nothing ever overflows.
        let is_short = short_dp.get() || container_dp.height() == 0 || container_dp.height() <= 595;
        if is_active {
            if !detached_dp.get() && is_short && hk_btn_for_dp.is_active() {
                was_open_dp.set(true);
                hk_btn_for_dp.set_active(false);
            }
        } else if was_open_dp.replace(false) && !detached_dp.get() && !hk_btn_for_dp.is_active() {
            hk_btn_for_dp.set_active(true);
        }
        rev_toggle.set_reveal_child(is_active);
        on_dp_tgl(is_active);
    });

    // Reverse accordion: manually expanding inline Speed Dial on a short window folds the keypad.
    let dp_btn_for_hk = dialpad_toggle.clone();
    let was_open_hk = speed_dial_was_open_before_dialpad.clone();
    let detached_hk = is_speed_dial_detached.clone();
    let short_hk = is_short_height.clone();
    let container_hk = container.clone();
    hotkeys_toggle_btn.connect_toggled(move |btn| {
        let is_short = short_hk.get() || container_hk.height() == 0 || container_hk.height() <= 595;
        if btn.is_active() && !detached_hk.get() && is_short && dp_btn_for_hk.is_active() {
            was_open_hk.set(false);
            dp_btn_for_hk.set_active(false);
        }
    });

    let dial_grid = gtk::Grid::new();
    dial_grid.set_row_spacing(4);
    dial_grid.set_column_spacing(5);
    dial_grid.set_column_homogeneous(true);
    dial_grid.set_halign(gtk::Align::Fill);
    dial_grid.set_hexpand(true);
    dial_grid.set_margin_top(2);
    dial_grid.set_margin_bottom(2);

    let keys: [(&str, &str); 12] = [
        ("1", ""), ("2", "ABC"), ("3", "DEF"),
        ("4", "GHI"), ("5", "JKL"), ("6", "MNO"),
        ("7", "PQRS"), ("8", "TUV"), ("9", "WXYZ"),
        ("*", ""), ("0", "+"), ("#", ""),
    ];

    for (index, (digit, sub)) in keys.iter().enumerate() {
        let col = (index % 3) as i32;
        let row = (index / 3) as i32;

        let btn_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        btn_box.set_valign(gtk::Align::Center);
        btn_box.set_halign(gtk::Align::Center);

        let d_lbl = gtk::Label::new(Some(digit));
        d_lbl.add_css_class("sidebar-dialpad-digit");
        btn_box.append(&d_lbl);

        if !sub.is_empty() {
            let s_lbl = gtk::Label::new(Some(sub));
            s_lbl.add_css_class("sidebar-dialpad-sub");
            btn_box.append(&s_lbl);
        }

        let key_btn = gtk::Button::new();
        key_btn.set_child(Some(&btn_box));
        key_btn.set_hexpand(true);
        key_btn.set_size_request(-1, 30);
        key_btn.add_css_class("flat");
        key_btn.add_css_class("sidebar-dialpad-key");

        let digit_str = digit.to_string();
        let entry_for_key = search_entry.clone();
        key_btn.connect_clicked(move |_| {
            let cur = entry_for_key.text();
            let new_text = format!("{}{}", cur, digit_str);
            entry_for_key.set_text(&new_text);
            entry_for_key.set_position(-1);
        });

        dial_grid.attach(&key_btn, col, row, 1, 1);
    }

    dialpad_revealer.set_child(Some(&dial_grid));
    dial_box.append(&dialpad_revealer);

    // C. Grüner Anruf-Knopf mit zentriertem Hörer-Icon (nur aktiv bei gültiger Nummer oder Kontakt)
    let call_btn = gtk::Button::from_icon_name("call-start-symbolic");
    call_btn.set_hexpand(true);
    call_btn.set_halign(gtk::Align::Fill);
    call_btn.add_css_class("call-btn-full-green");
    call_btn.set_sensitive(false);
    call_btn.set_tooltip_text(Some(&crate::i18n::tr("Enter a phone number or select a contact")));

    let backend_ready = Rc::new(Cell::new(true));
    let refresh_dial_sensitivity: Rc<dyn Fn()> = {
        let br = backend_ready.clone();
        let se = search_entry.clone();
        let cb = call_btn.clone();
        let cs = contacts.clone();
        let sds = speed_dials.clone();
        Rc::new(move || {
            let text = se.text();
            let resolved = resolve_dial_input(&text, &cs.borrow(), &sds.borrow());
            let is_ready = br.get();
            let enabled = is_ready && resolved.is_some();
            cb.set_sensitive(enabled);

            if !is_ready {
                cb.set_cursor_from_name(Some("not-allowed"));
                cb.set_tooltip_text(Some(&crate::i18n::tr("Dialing unavailable while connecting")));
            } else if let Some((disp, num)) = resolved {
                cb.set_cursor_from_name(None);
                if disp != num {
                    cb.set_tooltip_text(Some(&crate::i18n::trf("Call {name} ({number})", &[("name", &disp), ("number", &num)])));
                } else {
                    cb.set_tooltip_text(Some(&crate::i18n::trf("Call {number} (Enter)", &[("number", &num)])));
                }
            } else if text.trim().is_empty() {
                cb.set_cursor_from_name(None);
                cb.set_tooltip_text(Some(&crate::i18n::tr("Enter a phone number or select a contact")));
            } else {
                cb.set_cursor_from_name(Some("not-allowed"));
                cb.set_tooltip_text(Some(&crate::i18n::tr(
                    "Contact not found or invalid phone number",
                )));
            }
        })
    };

    let set_backend_ready: Rc<dyn Fn(bool)> = {
        let br = backend_ready.clone();
        let refresh = refresh_dial_sensitivity.clone();
        Rc::new(move |ready: bool| {
            br.set(ready);
            refresh();
        })
    };

    {
        let refresh = refresh_dial_sensitivity.clone();
        search_entry.connect_changed(move |_| {
            refresh();
        });
    }
    refresh_dial_sensitivity();

    let call_clone = on_trigger_call.clone();
    let entry_clone = search_entry.clone();
    let contacts_call = contacts.clone();
    let sds_call = speed_dials.clone();
    let br_call = backend_ready.clone();
    let p_call = popover.clone();
    let is_typing_call = is_user_typing.clone();
    call_btn.connect_clicked(move |_| {
        is_typing_call.set(false);
        p_call.popdown();
        if !br_call.get() {
            return;
        }
        let text = entry_clone.text().to_string();
        if let Some((disp_name, dial_num)) =
            resolve_dial_input(&text, &contacts_call.borrow(), &sds_call.borrow())
        {
            call_clone(disp_name, dial_num);
        }
    });

    let enter_call = on_trigger_call.clone();
    let enter_entry = search_entry.clone();
    let contacts_enter = contacts.clone();
    let sds_enter = speed_dials.clone();
    let br_enter = backend_ready.clone();
    let p_enter = popover.clone();
    let is_typing_enter = is_user_typing.clone();
    search_entry.connect_activate(move |_| {
        is_typing_enter.set(false);
        p_enter.popdown();
        if !br_enter.get() {
            return;
        }
        let text = enter_entry.text().to_string();
        if let Some((disp_name, dial_num)) =
            resolve_dial_input(&text, &contacts_enter.borrow(), &sds_enter.borrow())
        {
            enter_call(disp_name, dial_num);
        }
    });
    dial_box.append(&call_btn);

    // Leitungen Status-Chips: Im Leerlauf ausgeblendet (null Clutter), erscheinen automatisch bei aktivem/gehaltenem Gespräch
    let lines_revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .transition_duration(180)
        .reveal_child(false)
        .build();
    let lines_box = gtk::Box::new(gtk::Orientation::Vertical, 5);
    lines_box.set_margin_top(2);

    // Leitung 1 Chip: Startet im Leerlauf ("Bereit") und wird bei Gesprächsaufbau aktiv
    let chip1_top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let speaker1 = gtk::Image::from_icon_name("call-start-symbolic");
    speaker1.add_css_class("dim-label");
    speaker1.set_pixel_size(14);

    let lbl_line1 = gtk::Label::new(Some(&crate::i18n::tr("Line 1")));
    lbl_line1.add_css_class("caption");
    lbl_line1.add_css_class("dim-label");

    let sp1 = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    sp1.set_hexpand(true);

    let grip1 = gtk::Label::new(Some("⋮⋮"));
    grip1.add_css_class("line-drag-grip");
    grip1.set_tooltip_text(Some("Leitung auf Kollegen oder Kontakt ziehen zum Weiterleiten"));

    chip1_top.append(&speaker1);
    chip1_top.append(&lbl_line1);
    chip1_top.append(&sp1);
    chip1_top.append(&badge_line1);
    chip1_top.append(&grip1);
    chip_line1.append(&chip1_top);

    // Leitung 1 Lautstärkeregler (Einstellbar pro Leitung)
    let volume_scale1 = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 5.0);
    volume_scale1.set_value(75.0);
    volume_scale1.set_draw_value(false);
    volume_scale1.add_css_class("line-volume-scale");
    volume_scale1.set_tooltip_text(Some(&crate::i18n::trf(
        "Line 1 volume: {percent}% • Drag to adjust",
        &[("percent", "75")],
    )));
    volume_scale1.set_visible(false);

    let spk1_clone = speaker1.clone();
    volume_scale1.connect_value_changed(move |scale| {
        let val = scale.value();
        let pct = format!("{:.0}", val);
        scale.set_tooltip_text(Some(&crate::i18n::trf(
            "Line 1 volume: {percent}% • Drag to adjust",
            &[("percent", &pct)],
        )));
        if val <= 0.0 {
            spk1_clone.set_icon_name(Some("audio-volume-muted-symbolic"));
            spk1_clone.remove_css_class("speaker-green-icon");
            spk1_clone.add_css_class("dim-label");
        } else {
            spk1_clone.set_icon_name(Some("audio-volume-high-symbolic"));
            spk1_clone.remove_css_class("dim-label");
            spk1_clone.add_css_class("speaker-green-icon");
        }
    });
    chip_line1.append(&volume_scale1);

    let p_c1 = popover.clone();
    let is_typing_c1 = is_user_typing.clone();
    let c1_click = gtk::GestureClick::new();
    c1_click.connect_pressed(move |_, _, _, _| {
        is_typing_c1.set(false);
        p_c1.popdown();
    });
    chip_line1.add_controller(c1_click);

    lines_box.append(&chip_line1);

    // Leitung 2 Chip: Borderless Standby / Bereit
    let chip_line2 = gtk::Box::new(gtk::Orientation::Vertical, 4);
    chip_line2.add_css_class("line-chip");
    chip_line2.set_tooltip_text(Some(&crate::i18n::tr(
        "Line 2 • Click to swap • Drag onto a contact to transfer",
    )));
    let hld_drag2 = held_call.clone();
    crate::ui::attach_call_drag_source_with_tag(&chip_line2, "CALL_TRANSFER_HELD", move || {
        hld_drag2.borrow().is_some()
    });

    let chip2_top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let idle_icon = gtk::Image::from_icon_name("call-start-symbolic");
    idle_icon.add_css_class("dim-label");
    idle_icon.set_pixel_size(14);

    let lbl_line2 = gtk::Label::new(Some(&crate::i18n::tr("Line 2")));
    lbl_line2.add_css_class("caption");
    lbl_line2.add_css_class("dim-label");

    let sp2 = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    sp2.set_hexpand(true);

    let badge_line2 = gtk::Label::new(Some(&crate::i18n::tr("Free")));
    badge_line2.add_css_class("caption");
    badge_line2.add_css_class("dim-label");

    let grip2 = gtk::Label::new(Some("⋮⋮"));
    grip2.add_css_class("line-drag-grip");
    grip2.set_tooltip_text(Some(&crate::i18n::tr(
        "Drag line onto a colleague or contact to transfer",
    )));

    chip2_top.append(&idle_icon);
    chip2_top.append(&lbl_line2);
    chip2_top.append(&sp2);
    chip2_top.append(&badge_line2);
    chip2_top.append(&grip2);
    chip_line2.append(&chip2_top);

    // Leitung 2 Lautstärkeregler (Einstellbar pro Leitung)
    let volume_scale2 = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 5.0);
    volume_scale2.set_value(75.0);
    volume_scale2.set_draw_value(false);
    volume_scale2.add_css_class("line-volume-scale");
    volume_scale2.set_tooltip_text(Some(&crate::i18n::trf(
        "Line 2 volume: {percent}% • Drag to adjust",
        &[("percent", "75")],
    )));
    volume_scale2.set_visible(false);

    let spk2_clone = idle_icon.clone();
    volume_scale2.connect_value_changed(move |scale| {
        let val = scale.value();
        let pct = format!("{:.0}", val);
        scale.set_tooltip_text(Some(&crate::i18n::trf(
            "Line 2 volume: {percent}% • Drag to adjust",
            &[("percent", &pct)],
        )));
        if val <= 0.0 {
            spk2_clone.set_icon_name(Some("audio-volume-muted-symbolic"));
        } else {
            spk2_clone.set_icon_name(Some("audio-volume-high-symbolic"));
        }
    });
    chip_line2.append(&volume_scale2);

    let p_c2 = popover.clone();
    let is_typing_c2 = is_user_typing.clone();
    let c2_click = gtk::GestureClick::new();
    c2_click.connect_pressed(move |_, _, _, _| {
        is_typing_c2.set(false);
        p_c2.popdown();
    });
    chip_line2.add_controller(c2_click);

    lines_box.append(&chip_line2);
    lines_revealer.set_child(Some(&lines_box));
    docked_call_box
        .bind_property("visible", &lines_revealer, "reveal-child")
        .sync_create()
        .build();
    let dp_toggle_on_call = dialpad_toggle.clone();
    docked_call_box.connect_visible_notify(move |box_w| {
        if box_w.is_visible() && dp_toggle_on_call.is_active() {
            dp_toggle_on_call.set_active(false);
        }
    });
    dial_box.append(&lines_revealer);

    // Pin dial_box at the bottom of container outside inline Speed Dial so the dialer,
    // keypad, and Call button are always 100% visible and never clipped at any window height.
    let dial_sep = gtk::Separator::new(gtk::Orientation::Horizontal);
    container.append(&dial_sep);
    container.append(&dial_box);

    // Independent height BreakpointBin: automatically folds inline Speed Dial whenever
    // the window is resized below 595px height with the dialpad open (and restores it
    // once the window grows taller again), without competing with window width breakpoints.
    let root = adw::BreakpointBin::new();
    root.set_size_request(215, 480);
    root.set_hexpand(true);
    root.set_vexpand(true);
    root.set_child(Some(&container));

    let bp_short = adw::Breakpoint::new(
        adw::BreakpointCondition::parse("max-height: 595px").unwrap(),
    );
    {
        let short_on = is_short_height.clone();
        let detached_on = is_speed_dial_detached.clone();
        let dp_on = dialpad_toggle.clone();
        let hk_btn_on = hotkeys_toggle_btn.clone();
        let hk_rev_on = hotkeys_revealer.clone();
        let was_open_on = speed_dial_was_open_before_dialpad.clone();
        bp_short.connect_apply(move |_| {
            short_on.set(true);
            if !detached_on.get() && dp_on.is_active() && hk_btn_on.is_active() {
                was_open_on.set(true);
                hk_rev_on.set_transition_duration(0);
                hk_btn_on.set_active(false);
                hk_rev_on.set_reveal_child(false);
                hk_rev_on.set_transition_duration(180);
            }
        });
    }
    {
        let short_off = is_short_height.clone();
        let detached_off = is_speed_dial_detached.clone();
        let hk_btn_off = hotkeys_toggle_btn.clone();
        let was_open_off = speed_dial_was_open_before_dialpad.clone();
        bp_short.connect_unapply(move |_| {
            short_off.set(false);
            if !detached_off.get() && was_open_off.replace(false) && !hk_btn_off.is_active() {
                hk_btn_off.set_active(true);
            }
        });
    }
    root.add_breakpoint(bp_short);

    SidebarComponents {
        root,
        container,
        header_bar,
        list_box,
        journal_badge: badge_j,
        voicemail_badge: badge_vb,
        device_button,
        line1_chip: chip_line1,
        line1_badge: badge_line1,
        line1_icon: speaker1,
        line1_volume_scale: volume_scale1,
        line2_chip: chip_line2,
        line2_volume_scale: volume_scale2,
        line2_badge: badge_line2,
        line2_icon: idle_icon,
        lines_revealer,
        search_entry,
        call_btn,
        docked_call_box,
        docked_call_revealer,
        docked_held_box,
        docked_held_label,
        docked_swap_btn,
        docked_transfer_held_btn,
        docked_caller_label,
        docked_ext_label,
        docked_timer_label,
        docked_mute_btn,
        docked_dtmf_btn,
        docked_hangup_btn,
        user_label: label,
        user_avatar: avatar,
        user_dot: dot,
        autocomplete_popover: popover,
        hotkeys_revealer,
        hotkeys_toggle_btn,
        speed_dial_pane,
        set_speed_dial_detached,
        reload_hotkeys: populate_hotkeys,
        refresh_hotkeys_presence,
        set_backend_ready,
        refresh_dial_sensitivity,
    }
}
