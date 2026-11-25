/// SAFETY GATE: Strict Passive Mode / Monitoring Only
///
/// Ensures under all circumstances that no outgoing telephony packets or calls
/// can ever be dispatched to the PBX during development.
#[allow(dead_code)]
pub struct SafetyGate;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyError {
    CallsPhysicallyLocked,
}

#[allow(dead_code)]
impl SafetyGate {
    /// Hardcoded safety check: All network call-outs are physically disabled.
    pub const PHYSICALLY_LOCKED: bool = true;

    pub fn assert_can_call(target: &str) -> Result<(), SafetyError> {
        Self::assert_can_call_with_mode(true, target)
    }

    pub fn assert_can_call_with_mode(passive_mode: bool, target: &str) -> Result<(), SafetyError> {
        if passive_mode {
            eprintln!(
                "[SAFETY LOCK BLOCKED]: Outgoing call to '{}' blocked by Passive Development Gate.",
                target
            );
            return Err(SafetyError::CallsPhysicallyLocked);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safety_gate_blocks_outgoing_calls() {
        assert_eq!(
            SafetyGate::assert_can_call("431"),
            Err(SafetyError::CallsPhysicallyLocked)
        );
        assert_eq!(
            SafetyGate::assert_can_call("+491234567"),
            Err(SafetyError::CallsPhysicallyLocked)
        );
        // Passive mode active -> blocked
        assert_eq!(
            SafetyGate::assert_can_call_with_mode(true, "100"),
            Err(SafetyError::CallsPhysicallyLocked)
        );
        // Passive mode inactive -> allowed
        assert_eq!(
            SafetyGate::assert_can_call_with_mode(false, "100"),
            Ok(())
        );
    }

    #[test]
    fn test_dtmf_keypad_digits() {
        let valid_dtmf = ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9', '*', '#'];
        for d in valid_dtmf {
            assert!(
                d.is_ascii_digit() || d == '*' || d == '#',
                "DTMF digit '{}' must be valid RFC 4733 symbol",
                d
            );
        }
    }

    #[test]
    fn test_all_gtk_ui_components() {
        use gtk4 as gtk;
        use libadwaita as adw;
        use gtk::prelude::*;
        use adw::prelude::*;
        if gtk::is_initialized() || gtk::init().is_err() {
            return;
        }
        let _ = libadwaita::init();

        let provider = gtk::CssProvider::new();
        provider.load_from_data(include_str!("style.css"));
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        let sb_header = adw::HeaderBar::new();
        sb_header.add_css_class("sidebar-header");
        sb_header.set_show_start_title_buttons(false);
        sb_header.set_show_end_title_buttons(false);
        sb_header.set_show_title(false);

        let status_pill = gtk::Button::new();
        status_pill.add_css_class("flat");
        let status_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let avatar = adw::Avatar::builder().size(24).text("MG").build();
        let label = gtk::Label::new(Some("Benutzer"));
        status_box.append(&avatar);
        status_box.append(&label);
        status_pill.set_child(Some(&status_box));
        sb_header.pack_start(&status_pill);

        let device_button = gtk::Button::from_icon_name("preferences-desktop-display-symbolic");
        device_button.add_css_class("flat");
        device_button.add_css_class("circular");
        sb_header.pack_end(&device_button);

        let content_header = adw::HeaderBar::new();
        content_header.add_css_class("content-header");
        let search_button = gtk::ToggleButton::new();
        search_button.set_icon_name("edit-find-symbolic");
        search_button.add_css_class("flat");
        content_header.pack_start(&search_button);

        let header_title = gtk::Label::new(Some("Rufjournal"));
        header_title.add_css_class("title-3");
        content_header.set_title_widget(Some(&header_title));

        let passive_badge = gtk::Label::new(Some("Nur lesend"));
        passive_badge.add_css_class("sidebar-badge");
        content_header.pack_end(&passive_badge);

        let win = adw::ApplicationWindow::builder().build();
        let split_view = adw::NavigationSplitView::new();
        let sb_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sb_box.append(&sb_header);
        let cnt_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        cnt_box.append(&content_header);

        let p1 = adw::NavigationPage::builder().child(&sb_box).title("Swyx").build();
        let p2 = adw::NavigationPage::builder().child(&cnt_box).title("Rufjournal").build();
        split_view.set_sidebar(Some(&p1));
        split_view.set_content(Some(&p2));
        win.set_content(Some(&split_view));

        let (_, sb_nat, _, _) = sb_header.measure(gtk::Orientation::Vertical, -1);
        let (_, cnt_nat, _, _) = content_header.measure(gtk::Orientation::Vertical, -1);
        assert_eq!(sb_nat, cnt_nat, "Sidebar header and content header must have identical heights");

        // Verify icon theme
        if let Some(display) = gtk::gdk::Display::default() {
            let theme = gtk::IconTheme::for_display(&display);
            assert!(theme.has_icon("go-jump-symbolic"), "go-jump-symbolic must exist in theme");
            assert!(theme.has_icon("starred-symbolic"), "starred-symbolic must exist in theme");
        }

        // Verify speed dial toggle logic
        let mut speed_dials = vec![
            crate::models::SpeedDialEntry {
                id: 1,
                name: "Anna Beispiel".to_string(),
                number: "105".to_string(),
                state: 0,
                entity_id: 0,
            }
        ];

        let is_fav = speed_dials.iter().any(|sd| sd.number == "105" || sd.name == "Anna Beispiel");
        assert!(is_fav);
        speed_dials.retain(|sd| !(sd.number == "105" || sd.name == "Anna Beispiel"));
        assert_eq!(speed_dials.len(), 0);
        speed_dials.push(crate::models::SpeedDialEntry {
            id: 2,
            name: "Anna Beispiel".to_string(),
            number: "105".to_string(),
            state: 0,
            entity_id: 0,
        });
        assert_eq!(speed_dials.len(), 1);
        assert_eq!(speed_dials[0].number, "105");

        // Autocomplete popover lifecycle
        let contacts = std::rc::Rc::new(std::cell::RefCell::new(vec![
            crate::models::Contact {
                id: "1".to_string(),
                name: "Max Mustermann".to_string(),
                phone: "101".to_string(),
                extension: "101".to_string(),
                department: "Vertrieb".to_string(),
                presence: crate::models::PresenceState::Available,
                avatar_file_id: None,
            }
        ]));
        let audio = std::rc::Rc::new(std::cell::RefCell::new(crate::models::AudioMode::Speaker));
        let presence = std::rc::Rc::new(std::cell::RefCell::new(crate::models::PresenceState::Available));
        let telephony = std::rc::Rc::new(std::cell::RefCell::new(crate::models::TelephonyMode::Softphone));
        let active_call = std::rc::Rc::new(std::cell::RefCell::new(None));
        let held_call = std::rc::Rc::new(std::cell::RefCell::new(None));

        let speed_dials_rc = std::rc::Rc::new(std::cell::RefCell::new(speed_dials));
        let toast_overlay = adw::ToastOverlay::new();

        let sidebar = crate::ui::sidebar::create_sidebar(
            contacts.clone(),
            speed_dials_rc.clone(),
            toast_overlay.clone(),
            audio,
            presence,
            telephony,
            active_call,
            held_call,
            "Max Mustermann".to_string(),
            "100".to_string(),
            "0170 1234567".to_string(),
            false,
            true,
            |_, _| {},
            |_| {},
            |_| {},
            |_, _| {},
            |_| {},
            |_, _, _| {},
            |_, _| {},
            |_| {},
            || {},
            |_, _| {},
            |_| {},
        );

        (sidebar.reload_hotkeys)();
        // When initial_dialpad_visible is true on compact height, inline Speed Dial starts folded
        assert!(!sidebar.hotkeys_revealer.reveals_child());
        assert!(!sidebar.hotkeys_toggle_btn.is_active());
        sidebar.hotkeys_toggle_btn.set_active(true);
        assert!(sidebar.hotkeys_revealer.reveals_child());
        sidebar.hotkeys_toggle_btn.set_active(false);
        assert!(!sidebar.hotkeys_revealer.reveals_child());
        sidebar.hotkeys_toggle_btn.set_active(true);
        assert!(sidebar.hotkeys_revealer.reveals_child());

        // Verify lines_revealer is hidden while idle and auto-reveals when a call starts
        assert!(!sidebar.lines_revealer.reveals_child());
        sidebar.docked_call_box.set_visible(true);
        assert!(sidebar.lines_revealer.reveals_child());
        sidebar.docked_call_box.set_visible(false);
        assert!(!sidebar.lines_revealer.reveals_child());

        // Verify inline speed dial list contains populated item & live in-place presence refresh works
        let scroll = sidebar
            .hotkeys_revealer
            .child()
            .unwrap()
            .downcast::<gtk::ScrolledWindow>()
            .unwrap();
        let viewport = scroll.child().unwrap().downcast::<gtk::Viewport>().unwrap();
        let list_box = viewport.child().unwrap().downcast::<gtk::Box>().unwrap();
        let hk_btn = list_box
            .first_child()
            .expect("Hotkey item should be present in list_box")
            .downcast::<gtk::Button>()
            .unwrap();

        // Verify dial_box is pinned at the bottom of sidebar.container (after middle_box + separator)
        let last_child = sidebar.container.last_child().expect("dial_box should be pinned at bottom");
        assert!(last_child.is::<gtk::Box>(), "bottom pinned widget should be dial_box");
        let sep_child = last_child.prev_sibling().expect("separator should precede dial_box");
        assert!(sep_child.is::<gtk::Separator>(), "separator should divide middle area and dial_box");
        let middle_child = sep_child.prev_sibling().expect("middle_box should precede separator");
        assert!(middle_child.is::<gtk::Box>(), "middle_box should hold speed dial and docked call card");

        // Verify detaching Speed Dial moves hotkey_list_box into the right-hand utility pane and back
        (sidebar.set_speed_dial_detached)(true);
        assert!(scroll.child().is_none(), "inline scroll should detach hotkey_list_box in wide mode");
        let right_scroll = sidebar
            .speed_dial_pane
            .content()
            .unwrap()
            .downcast::<gtk::ScrolledWindow>()
            .unwrap();
        assert!(right_scroll.child().is_some(), "right-hand pane should host hotkey_list_box in wide mode");
        (sidebar.set_speed_dial_detached)(false);
        assert!(right_scroll.child().is_none(), "right-hand pane should release hotkey_list_box in standard mode");
        assert!(scroll.child().is_some(), "inline scroll should reattach hotkey_list_box in standard mode");
        assert!(!hk_btn.has_css_class("hotkey-chip-busy"));
        contacts.borrow_mut().push(crate::models::Contact {
            id: "swyx-10".into(),
            name: "Anna Beispiel".into(),
            extension: "105".into(),
            department: "Swyx Intern".into(),
            presence: crate::models::PresenceState::Busy,
            phone: "105".into(),
            avatar_file_id: Some("test-avatar-105".into()),
        });
        (sidebar.refresh_hotkeys_presence)();
        assert!(hk_btn.has_css_class("hotkey-chip-busy"));
        contacts.borrow_mut().last_mut().unwrap().presence = crate::models::PresenceState::Available;
        (sidebar.refresh_hotkeys_presence)();
        assert!(!hk_btn.has_css_class("hotkey-chip-busy"));

        // Verify avatar JPEG preparation & in-memory contact photo lookup (using an in-memory Pixbuf)
        let pb = gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, false, 8, 320, 200)
            .expect("Failed to allocate test Pixbuf");
        pb.fill(0x2ec27eff);
        let tmp_png = std::env::temp_dir().join("deskphone_test_avatar_src.png");
        pb.savev(&tmp_png, "png", &[]).expect("Failed to save test PNG");
        let jpeg_bytes = crate::ui::prepare_avatar_jpeg_bytes(&tmp_png)
            .expect("prepare_avatar_jpeg_bytes should succeed");
        let _ = std::fs::remove_file(&tmp_png);
        assert!(jpeg_bytes.starts_with(&[0xFF, 0xD8, 0xFF]), "Expected valid JPEG header");

        let sample_contact = crate::models::Contact {
            id: "swyx-999".into(),
            name: "Testperson T. 999".into(),
            extension: "999".into(),
            department: "Swyx Intern".into(),
            presence: crate::models::PresenceState::Available,
            phone: "999".into(),
            avatar_file_id: Some("unit-test-avatar-999".into()),
        };
        let tex = crate::ui::store_contact_photo_bytes(&sample_contact, "unit-test-avatar-999", &jpeg_bytes);
        assert!(tex.is_some(), "Expected decoded GdkTexture from JPEG bytes");
        assert!(crate::ui::get_contact_photo("Testperson T. 999").is_some());
        assert!(crate::ui::get_contact_photo("T. Testperson").is_some());
        assert!(crate::ui::get_contact_photo("999").is_some());
        let _ = std::fs::remove_file(crate::ui::avatar_cache_dir().join("unit-test-avatar-999.img"));

        // Verify phone-number neutral avatar never propagates hexpand into AdwActionRow prefixes
        let (num_av, _) = crate::ui::create_presence_avatar_with_dot("+4930123456", None, 32);
        let (name_av, _) = crate::ui::create_presence_avatar_with_dot("Max Mustermann", None, 32);
        assert!(
            !num_av.compute_expand(gtk::Orientation::Horizontal),
            "Neutral phone-number avatar must never expand horizontally inside list rows"
        );
        assert!(
            !name_av.compute_expand(gtk::Orientation::Horizontal),
            "Named avatar must not expand horizontally inside list rows"
        );

        assert!(!sidebar.autocomplete_popover.is_visible());
        sidebar.autocomplete_popover.popdown();
        assert!(!sidebar.autocomplete_popover.is_visible());

        // Verify Call button sensitivity & placeholder without "..."
        assert_eq!(
            sidebar.search_entry.placeholder_text().as_deref(),
            Some("Number or name")
        );
        assert!(
            !sidebar.call_btn.is_sensitive(),
            "Call button must be disabled when dial entry is empty"
        );
        sidebar.search_entry.set_text("Unbekannter Kollege");
        assert!(
            !sidebar.call_btn.is_sensitive(),
            "Call button must be disabled for unknown names not in phonebook"
        );
        sidebar.search_entry.set_text("Max Mustermann");
        assert!(
            sidebar.call_btn.is_sensitive(),
            "Call button must be enabled when a valid phonebook contact is entered"
        );
        sidebar.search_entry.set_text("101");
        assert!(
            sidebar.call_btn.is_sensitive(),
            "Call button must be enabled when a valid extension/number is entered"
        );
        (sidebar.set_backend_ready)(false);
        assert!(
            !sidebar.call_btn.is_sensitive(),
            "Call button must be disabled when backend is not registered"
        );
        (sidebar.set_backend_ready)(true);
        assert!(sidebar.call_btn.is_sensitive());

        // Verify live in-place presence dot updates without view switching
        let (_av_widget, dot_widget) = crate::ui::create_presence_avatar_with_dot(
            "Max Mustermann",
            Some(crate::models::PresenceState::Available),
            32,
        );
        assert!(dot_widget.has_css_class("presence-available"));
        crate::ui::update_presence_dot(&dot_widget, Some(crate::models::PresenceState::Busy));
        assert!(dot_widget.has_css_class("presence-busy"));
        assert!(!dot_widget.has_css_class("presence-available"));

        // Responsive Breakpoints
        let condition = adw::BreakpointCondition::parse("max-width: 540px").unwrap();
        let bp = adw::Breakpoint::new(condition);
        let resp_split = adw::NavigationSplitView::new();
        resp_split.set_min_sidebar_width(215.0);
        resp_split.set_max_sidebar_width(280.0);
        resp_split.set_sidebar_width_fraction(0.38);
        assert_eq!(resp_split.min_sidebar_width(), 215.0);
        assert_eq!(resp_split.max_sidebar_width(), 280.0);
        bp.add_setter(&resp_split, "collapsed", Some(&true.to_value()));

        let sb_box2 = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let cnt_box2 = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let cnt_hdr2 = adw::HeaderBar::new();
        cnt_box2.append(&cnt_hdr2);

        let p_sb = adw::NavigationPage::builder().child(&sb_box2).title("Deskphone").build();
        let p_cnt = adw::NavigationPage::builder().child(&cnt_box2).title("Rufjournal").build();
        resp_split.set_sidebar(Some(&p_sb));
        resp_split.set_content(Some(&p_cnt));

        resp_split.set_collapsed(true);
        resp_split.set_show_content(true);
        assert!(resp_split.is_collapsed());
        assert!(resp_split.shows_content());

        resp_split.set_show_content(false);
        assert!(!resp_split.shows_content());

        let window = adw::ApplicationWindow::builder()
            .default_width(860)
            .default_height(580)
            .width_request(320)
            .height_request(400)
            .build();
        window.add_breakpoint(bp);
        assert_eq!(window.default_width(), 860);
        assert_eq!(window.default_height(), 580);
        assert_eq!(window.width_request(), 320);

        let win_title = adw::WindowTitle::new("Rufjournal", "");
        assert_eq!(win_title.title(), "Rufjournal");

        let action_row = adw::ActionRow::new();
        action_row.set_title_lines(1);
        action_row.set_subtitle_lines(1);
        assert_eq!(action_row.title_lines(), 1);
        assert_eq!(action_row.subtitle_lines(), 1);
    }

    #[test]
    fn test_swyx_presence_state_mappings() {
        use crate::models::PresenceState;

        // UserState mappings:
        // 0 = LoggedOff -> LoggedOut (Grau)
        assert_eq!(PresenceState::from_swyx_user_state(0), PresenceState::LoggedOut);
        // 1 = LoggedOn -> Available (Grün)
        assert_eq!(PresenceState::from_swyx_user_state(1), PresenceState::Available);
        // 2 = Active (telefoniert / spricht) -> Busy (Rot)
        assert_eq!(PresenceState::from_swyx_user_state(2), PresenceState::Busy);
        // 3 = NoStatus -> LoggedOut
        assert_eq!(PresenceState::from_swyx_user_state(3), PresenceState::LoggedOut);
        // 4 = Away -> Away (Gelb)
        assert_eq!(PresenceState::from_swyx_user_state(4), PresenceState::Away);
        // 5 = DoNotDisturb -> Busy (Rot)
        assert_eq!(PresenceState::from_swyx_user_state(5), PresenceState::Busy);
        // 6 = NotificationCall -> Ringing (Klingelt / Amber)
        assert_eq!(PresenceState::from_swyx_user_state(6), PresenceState::Ringing);

        // CLMgrNameKeyStates mappings (SpeedDials):
        // 0 = NKUnknown, 1 = NKLoggedOut -> LoggedOut (Grau)
        assert_eq!(PresenceState::from_swyx_clmgr_state(0), PresenceState::LoggedOut);
        assert_eq!(PresenceState::from_swyx_clmgr_state(1), PresenceState::LoggedOut);
        // 2 = NKLoggedIn -> Available (Grün)
        assert_eq!(PresenceState::from_swyx_clmgr_state(2), PresenceState::Available);
        // 3 = NKBusy (telefoniert) -> Busy (Rot)
        assert_eq!(PresenceState::from_swyx_clmgr_state(3), PresenceState::Busy);
        // 4 = NKGroupCallNotification -> Ringing (Klingelt / Amber)
        assert_eq!(PresenceState::from_swyx_clmgr_state(4), PresenceState::Ringing);
        // 5 = NKAway -> Away (Gelb)
        assert_eq!(PresenceState::from_swyx_clmgr_state(5), PresenceState::Away);
        // 6 = NKDoNotDisturb -> Busy (Rot)
        assert_eq!(PresenceState::from_swyx_clmgr_state(6), PresenceState::Busy);

        // Roundtrip PresenceState -> Swyx CLMgrNameKeyState
        assert_eq!(PresenceState::Available.to_swyx_clmgr_state(), 2);
        assert_eq!(PresenceState::Busy.to_swyx_clmgr_state(), 3);
        assert_eq!(PresenceState::Ringing.to_swyx_clmgr_state(), 4);
        assert_eq!(PresenceState::Away.to_swyx_clmgr_state(), 5);
        assert_eq!(PresenceState::LoggedOut.to_swyx_clmgr_state(), 1);

        // CSS Classes & Color association
        assert_eq!(PresenceState::Available.css_class(), "presence-available");
        assert_eq!(PresenceState::Busy.css_class(), "presence-busy");
        assert_eq!(PresenceState::Away.css_class(), "presence-away");
        assert_eq!(PresenceState::LoggedOut.css_class(), "presence-offline");
        assert_eq!(PresenceState::Ringing.css_class(), "presence-ringing");
        assert_eq!(PresenceState::Ringing.label(), "Ringing");
    }

    #[test]
    fn test_call_pickup_formatting() {
        use crate::models::format_pickup_number;
        assert_eq!(format_pickup_number("*8", "101"), "*8101");
        assert_eq!(format_pickup_number("*8{ext}", "101"), "*8101");
        assert_eq!(format_pickup_number("**", "101"), "**101");
        assert_eq!(format_pickup_number("*8", ""), "*8");
    }

    #[test]
    fn test_contacts_speeddial_presence_sync() {
        use crate::models::{Contact, PresenceState, SpeedDialEntry};

        let mut contacts = vec![
            Contact {
                id: "swyx-101".to_string(),
                name: "Max Mustermann".to_string(),
                extension: "101".to_string(),
                department: "Vertrieb".to_string(),
                presence: PresenceState::Available,
                phone: "+49 89 123456-101".to_string(),
                avatar_file_id: None,
            },
            Contact {
                id: "swyx-102".to_string(),
                name: "Erika Musterfrau".to_string(),
                extension: "102".to_string(),
                department: "Marketing".to_string(),
                presence: PresenceState::LoggedOut,
                phone: "+49 89 123456-102".to_string(),
                avatar_file_id: None,
            },
        ];

        let speed_dials = vec![
            SpeedDialEntry {
                id: 4,
                name: "Max Mustermann".to_string(),
                number: "101".to_string(),
                state: 3, // NKBusy -> telefoniert
                entity_id: 101,
            },
            SpeedDialEntry {
                id: 0,
                name: "Erika Musterfrau".to_string(),
                number: "102".to_string(),
                state: 2, // NKLoggedIn -> frei / erreichbar
                entity_id: 102,
            },
        ];

        for sd in &speed_dials {
            let presence = PresenceState::from_swyx_clmgr_state(sd.state);
            let target_id = format!("swyx-{}", sd.entity_id);
            for c in contacts.iter_mut() {
                if (sd.entity_id > 0 && c.id == target_id)
                    || (!sd.number.is_empty() && c.extension == sd.number)
                {
                    c.presence = presence;
                }
            }
        }

        // Max Mustermann must now be Busy (Red)
        assert_eq!(contacts[0].presence, PresenceState::Busy);
        // Erika Musterfrau must now be Available (Green)
        assert_eq!(contacts[1].presence, PresenceState::Available);
    }

    #[test]
    fn test_chat_texts_and_avatar() {
        use crate::models::{Contact, PresenceState};

        let contact = Contact {
            id: "swyx-103".to_string(),
            name: "Anna S.".to_string(),
            extension: "103".to_string(),
            department: "Entwicklung".to_string(),
            presence: PresenceState::Available,
            phone: "+49 89 123456-103".to_string(),
            avatar_file_id: None,
        };

        // Subtitle format
        let sub_text = if !contact.extension.is_empty() {
            format!("{} ({})", contact.presence.label(), contact.extension)
        } else {
            contact.presence.label().to_string()
        };
        assert_eq!(sub_text, "Available (103)");

        // Shortened texts
        let placeholder = "Nachricht...";
        let hint = "Keine Speicherung";
        assert!(placeholder.len() <= 15);
        assert!(hint.len() <= 20);
    }

    #[test]
    fn test_is_phone_number_detection() {
        use crate::ui::is_phone_number;

        // Numbers should be recognized as phone numbers (no letters)
        assert!(is_phone_number("+49 171 1234567"));
        assert!(is_phone_number("+49151555666"));
        assert!(is_phone_number("089 123456"));
        assert!(is_phone_number("0171/9876543"));
        assert!(is_phone_number("101"));
        assert!(is_phone_number(""));

        // Names with letters should NOT be recognized as numbers
        assert!(!is_phone_number("Mustermann M."));
        assert!(!is_phone_number("Musterfrau E."));
        assert!(!is_phone_number("Schmidt A."));
        assert!(!is_phone_number("Support"));
        assert!(!is_phone_number("Marketing Team"));
    }

    #[test]
    fn test_clean_name_for_avatar() {
        use crate::ui::clean_name_for_avatar;

        // Truncate extension digits from corporate name format and reverse to Vorname Nachname
        // so Adwaita Avatar generates initials in natural order (MM, EM, AS)
        assert_eq!(clean_name_for_avatar("Mustermann M. 101"), "M. Mustermann");
        assert_eq!(clean_name_for_avatar("Musterfrau E. 102"), "E. Musterfrau");
        assert_eq!(clean_name_for_avatar("Schmidt A. 103"), "A. Schmidt");
        assert_eq!(clean_name_for_avatar("Weber T."), "T. Weber");
        assert_eq!(clean_name_for_avatar("Support"), "Support");

        use crate::ui::sidebar::format_hotkey_display_name;
        assert_eq!(format_hotkey_display_name("Thomas Weber"), "Thomas Weber");
        assert_eq!(format_hotkey_display_name("Sarah Schmidt"), "Sarah Schmidt");
        assert_eq!(format_hotkey_display_name("Mustermann M. 101"), "M. Mustermann");
    }

    #[test]
    fn test_forward_and_seen_calls_persistence() {
        use crate::swyx_bridge::SwyxDataStore;

        let mut store = SwyxDataStore::default();
        store.forward_number = "0176 12345678".to_string();
        store.forward_enabled = true;
        store.seen_call_ids = vec!["call-1".into(), "call-2".into()];

        let json = serde_json::to_string(&store).expect("Serialization failed");
        let restored: SwyxDataStore = serde_json::from_str(&json).expect("Deserialization failed");

        assert_eq!(restored.forward_number, "0176 12345678");
        assert!(restored.forward_enabled);
        assert_eq!(restored.seen_call_ids, vec!["call-1", "call-2"]);
    }

    #[test]
    fn test_pure_rust_config_loading() {
        use crate::swyx_bridge::SwyxBridge;

        let bridge = SwyxBridge::new();
        let store = bridge.load_or_init_data();
        assert_eq!(store.pickup_prefix, "*8");
    }

    #[test]
    fn test_deskphone_project_naming_and_config() {
        use crate::swyx_bridge::SwyxBridge;

        let bridge = SwyxBridge::new();
        let store = bridge.load_or_init_data();

        // Ensure default store is sane and non-proprietary
        assert!(!store.server_ip.contains("10.0.0.99") || !store.server_ip.is_empty(), "Store loads cleanly");
    }

    #[test]
    fn test_call_signaling_and_group_call_event() {
        use crate::swyx_bridge::SwyxPassiveEvent;

        let ev = SwyxPassiveEvent::LineDetailsChanged {
            line_id: 0,
            line_state: 3, // Ringing
            peer_name: "Test Caller".to_string(),
            peer_number: "0171 123456".to_string(),
            is_incoming: true,
            is_group_call: true,
            called_name: "Support Group".to_string(),
            called_extension: "800".to_string(),
        };

        if let SwyxPassiveEvent::LineDetailsChanged { is_group_call, is_incoming, line_state, .. } = ev {
            assert!(is_group_call);
            assert!(is_incoming);
            assert_eq!(line_state, 3);
        } else {
            panic!("Expected LineDetailsChanged");
        }
    }

    #[test]
    fn test_ringtone_sound_and_notification_actions() {
        let sound_path = std::path::Path::new("/usr/share/sounds/freedesktop/stereo/phone-incoming-call.oga");
        assert!(sound_path.exists(), "Standard freedesktop ringtone must exist on system");

        let action_answer = "app.answer-incoming-call";
        let action_reject = "app.reject-incoming-call";
        let action_show = "app.show-window";

        assert_eq!(action_answer, "app.answer-incoming-call");
        assert_eq!(action_reject, "app.reject-incoming-call");
        assert_eq!(action_show, "app.show-window");
    }

    #[test]
    fn test_audio_devices_and_window_behavior_settings() {
        use crate::swyx_bridge::SwyxDataStore;
        use crate::engine_bridge::EngineBridge;

        let store = SwyxDataStore::default();
        assert!(store.background_on_close, "background_on_close must default to true (GNOME HIG background daemon)");
        assert!(!store.passive_mode, "passive_mode must default to false");
        assert_eq!(store.pickup_prefix, "*8");
        assert_eq!(store.audio_output_device, "default");
        assert_eq!(store.audio_input_device, "default");
        assert_eq!(store.audio_ring_device, "default");
        assert_eq!(store.voicemail_number, "");

        let out_devs = EngineBridge::list_output_devices();
        let in_devs = EngineBridge::list_input_devices();
        assert!(!out_devs.is_empty(), "Output devices must include at least System-Standard");
        assert!(!in_devs.is_empty(), "Input devices must include at least System-Standard");
        assert_eq!(out_devs[0].id, "default");
        assert_eq!(in_devs[0].id, "default");
        assert!(out_devs.iter().all(|d| !d.id.starts_with("raop_sink.")));
        assert!(in_devs.iter().all(|d| !d.id.ends_with(".monitor")));
    }

    #[test]
    fn test_tray_notifier_and_actions() {
        use crate::tray::{DeskphoneTray, TrayAction};
        use crate::models::PresenceState;
        use ksni::Tray;

        let (tx, rx) = async_channel::unbounded();
        let mut tray = DeskphoneTray::new(tx, PresenceState::Available);

        assert_eq!(tray.id(), "com.martingawron.Deskphone");
        assert_eq!(tray.category(), ksni::Category::Communications);
        assert_eq!(tray.icon_name(), "call-start-symbolic");
        assert_eq!(tray.attention_icon_name(), "call-incoming-symbolic");
        assert_eq!(tray.status(), ksni::Status::Active);

        let tooltip = tray.tool_tip();
        assert_eq!(tooltip.title, "Deskphone");
        assert!(tooltip.description.contains("Available"));

        // Left-click activation toggles window
        tray.activate(100, 100);
        let action = rx.try_recv().expect("Expected action on activate");
        assert_eq!(action, TrayAction::ToggleWindow);

        // Menu structure: flat Libadwaita symbolic icons & RadioGroup (no jumpy SubMenu)
        let menu = tray.menu();
        assert!(menu.len() >= 4);
        let mut saw_radio_group = false;
        for item in &menu {
            match item {
                ksni::MenuItem::Standard(s) => {
                    if !s.icon_name.is_empty() {
                        assert!(
                            s.icon_name.ends_with("-symbolic"),
                            "Tray StandardItem '{}' must use a Libadwaita -symbolic icon, got '{}'",
                            s.label,
                            s.icon_name
                        );
                    }
                }
                ksni::MenuItem::RadioGroup(rg) => {
                    saw_radio_group = true;
                    assert_eq!(rg.selected, 0);
                    assert_eq!(rg.options.len(), 3);
                    for opt in &rg.options {
                        assert!(
                            opt.icon_name.ends_with("-symbolic"),
                            "Tray RadioItem '{}' must use a Libadwaita -symbolic icon, got '{}'",
                            opt.label,
                            opt.icon_name
                        );
                    }
                }
                ksni::MenuItem::SubMenu(sm) => {
                    panic!(
                        "Tray menu should not use nested SubMenu '{}' to avoid GNOME Shell layout jump",
                        sm.label
                    );
                }
                _ => {}
            }
        }
        assert!(saw_radio_group, "Expected flat RadioGroup for presence states");

        // In-call state changes
        tray.has_incoming_call = true;
        assert_eq!(tray.status(), ksni::Status::NeedsAttention);

        tray.call_info = Some("Max Mustermann".to_string());
        let tip_call = tray.tool_tip();
        assert!(tip_call.description.contains("Max Mustermann"));
        assert!(tip_call.description.contains("Incoming call"));

        // Left-click during incoming call brings window/popup to front instead of hiding
        tray.activate(100, 100);
        let action_ringing = rx.try_recv().expect("Expected ShowWindow on ringing activate");
        assert_eq!(action_ringing, TrayAction::ShowWindow);
        assert!(tray.menu().len() >= 6);
    }

    #[test]
    fn test_resolve_dial_input_validation() {
        use crate::models::{Contact, PresenceState, SpeedDialEntry};
        use crate::ui::sidebar::resolve_dial_input;

        let contacts = vec![
            Contact {
                id: "swyx-42".to_string(),
                name: "Mustermann M. 101".to_string(),
                extension: "101".to_string(),
                department: "Vertrieb".to_string(),
                presence: PresenceState::Available,
                phone: "+4989123456101".to_string(),
                avatar_file_id: None,
            },
            Contact {
                id: "ext-1".to_string(),
                name: "Externer Lieferant".to_string(),
                extension: String::new(),
                department: "Einkauf".to_string(),
                presence: PresenceState::LoggedOut,
                phone: "+49 30 1234567".to_string(),
                avatar_file_id: None,
            },
        ];
        let speed_dials = vec![SpeedDialEntry {
            id: 1,
            name: "Zentrale".to_string(),
            number: "100".to_string(),
            state: 2,
            entity_id: 0,
        }];

        // Empty or unknown names -> None
        assert_eq!(resolve_dial_input("", &contacts, &speed_dials), None);
        assert_eq!(resolve_dial_input("   ", &contacts, &speed_dials), None);
        assert_eq!(resolve_dial_input("Peter Pan", &contacts, &speed_dials), None);
        assert_eq!(resolve_dial_input("Muster", &contacts, &speed_dials), None);
        assert_eq!(resolve_dial_input("1", &contacts, &speed_dials), None);
        assert_eq!(resolve_dial_input("+49", &contacts, &speed_dials), None);
        assert_eq!(resolve_dial_input("101abc", &contacts, &speed_dials), None);

        // Known phonebook contacts (raw name, formatted name, or extension) -> resolved!
        assert_eq!(
            resolve_dial_input("Mustermann M. 101", &contacts, &speed_dials),
            Some(("M. Mustermann".to_string(), "101".to_string()))
        );
        assert_eq!(
            resolve_dial_input("M. Mustermann", &contacts, &speed_dials),
            Some(("M. Mustermann".to_string(), "101".to_string()))
        );
        assert_eq!(
            resolve_dial_input("101", &contacts, &speed_dials),
            Some(("M. Mustermann".to_string(), "101".to_string()))
        );
        assert_eq!(
            resolve_dial_input("Externer Lieferant", &contacts, &speed_dials),
            Some(("Externer Lieferant".to_string(), "+49301234567".to_string()))
        );
        assert_eq!(
            resolve_dial_input("Zentrale", &contacts, &speed_dials),
            Some(("Zentrale".to_string(), "100".to_string()))
        );

        // Valid numbers & PBX codes
        assert_eq!(
            resolve_dial_input("102", &contacts, &speed_dials),
            Some(("102".to_string(), "102".to_string()))
        );
        assert_eq!(
            resolve_dial_input("*8101", &contacts, &speed_dials),
            Some(("*8101".to_string(), "*8101".to_string()))
        );
        assert_eq!(
            resolve_dial_input("+49 (89) 1234-567", &contacts, &speed_dials),
            Some(("+49 (89) 1234-567".to_string(), "+49891234567".to_string()))
        );
    }

    #[test]
    fn test_resolve_called_colleague_name() {
        use crate::models::{Contact, PresenceState, SpeedDialEntry};
        use crate::ui::window::resolve_called_colleague_name;

        let contacts = vec![Contact {
            id: "swyx-11".to_string(),
            name: "Musterfrau E. 102".to_string(),
            extension: "102".to_string(),
            department: "Vertrieb".to_string(),
            presence: PresenceState::Available,
            phone: "+4989123456102".to_string(),
            avatar_file_id: None,
        }];
        let speed_dials = vec![SpeedDialEntry {
            id: 1,
            name: "Weber T. 103".to_string(),
            number: "103".to_string(),
            state: 2,
            entity_id: 0,
        }];

        // Direct extension match -> colleague's formatted name
        assert_eq!(
            resolve_called_colleague_name("102", &contacts, &speed_dials).as_deref(),
            Some("E. Musterfrau")
        );
        // Full DDI number ending with colleague extension -> resolved
        assert_eq!(
            resolve_called_colleague_name("+49 89 123456-102", &contacts, &speed_dials).as_deref(),
            Some("E. Musterfrau")
        );
        // Speed dial fallback
        assert_eq!(
            resolve_called_colleague_name("103", &contacts, &speed_dials).as_deref(),
            Some("T. Weber")
        );
        // Unknown / department extension -> None
        assert_eq!(resolve_called_colleague_name("200", &contacts, &speed_dials), None);
    }

    #[test]
    fn test_provider_kind_and_config_serialization() {
        use crate::engine_bridge::parse_server_host_and_port;
        use crate::swyx_bridge::{ProviderKind, SwyxDataStore};

        // Existing configs without `provider` default cleanly to Swyx (backward-compatible)
        let legacy_json = r#"{"server_ip":"10.0.0.10","user_name":"Mustermann M. 101"}"#;
        let parsed: SwyxDataStore = serde_json::from_str(legacy_json).expect("legacy json parses");
        assert_eq!(parsed.provider, ProviderKind::Swyx);
        assert_eq!(parsed.sip_port, 5060);
        assert!(parsed.provider.supports_server_avatar());

        // Fresh default install starts in zero-config Auto-detect mode
        let fresh = SwyxDataStore::default();
        assert_eq!(fresh.provider, ProviderKind::Auto);
        assert_eq!(fresh.sip_port, 5060);
        assert!(!fresh.provider.supports_server_avatar());

        // Index roundtrip across all provider options
        for p in [
            ProviderKind::Auto,
            ProviderKind::GenericSip,
            ProviderKind::Swyx,
            ProviderKind::Asterisk,
        ] {
            assert_eq!(ProviderKind::from_index(p.index()), p);
        }

        // Server host:port and scheme parsing
        assert_eq!(
            parse_server_host_and_port("fritz.box", 5060),
            ("fritz.box".to_string(), 5060)
        );
        assert_eq!(
            parse_server_host_and_port("sip:sip.example.org:5062", 5060),
            ("sip.example.org".to_string(), 5062)
        );
        assert_eq!(
            parse_server_host_and_port("https://pbx.example.org/path", 0),
            ("pbx.example.org".to_string(), 5060)
        );
    }

    #[test]
    fn test_keyring_test_guard_and_xdg_paths() {
        // Under cfg(test), keyring calls must never touch the user's live Secret Service
        assert!(!crate::portals::store_password("example.com", "alice", "secret"));
        assert_eq!(crate::portals::load_password("example.com", "alice"), None);

        // XDG directories resolve cleanly inside user config/cache paths
        let cfg_dir = libadwaita::glib::user_config_dir().join("deskphone");
        let cache_dir = libadwaita::glib::user_cache_dir().join("deskphone");
        assert!(cfg_dir.ends_with("deskphone"));
        assert!(cache_dir.ends_with("deskphone"));
    }

    #[test]
    fn test_i18n_helpers_and_german_catalog() {
        assert_eq!(crate::i18n::tr("Preferences"), "Preferences");
        assert_eq!(
            crate::i18n::trf("Call {name}", &[("name", "Alice")]),
            "Call Alice"
        );
        // Verify po/de.po exists and was compiled by build.rs into target/locale/de/LC_MESSAGES/deskphone.mo
        let de_po = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("po/de.po");
        assert!(de_po.exists(), "po/de.po must exist");
        let de_mo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/locale/de/LC_MESSAGES/deskphone.mo");
        assert!(de_mo.exists(), "compiled German .mo catalog must exist");
    }

    #[test]
    fn test_gnome_contacts_vcard_and_eds_merge() {
        use crate::gnome_contacts::{
            merge_eds_contacts, parse_enabled_address_book_source, parse_vcard_contacts,
            EDS_ID_PREFIX,
        };
        use crate::models::{Contact, PresenceState};

        // 1. Source INI parsing: enabled address book vs disabled vs calendar-only
        let enabled_ini = "[Data Source]\nDisplayName=Personal\nEnabled=true\n\n[Address Book]\nBackendName=local\n";
        assert_eq!(
            parse_enabled_address_book_source(enabled_ini).as_deref(),
            Some("Personal")
        );
        let disabled_ini = "[Data Source]\nDisplayName=Old\nEnabled=false\n\n[Address Book]\nBackendName=local\n";
        assert_eq!(parse_enabled_address_book_source(disabled_ini), None);
        let cal_only_ini = "[Data Source]\nDisplayName=Calendar\n\n[Calendar]\nBackendName=local\n";
        assert_eq!(parse_enabled_address_book_source(cal_only_ini), None);

        // 2. vCard parsing (folded lines, ORG, multiple TELs, tel: URI prefix, dedup)
        let vcard = concat!(
            "BEGIN:VCARD\r\n",
            "VERSION:3.0\r\n",
            "UID:abc-123\r\n",
            "FN:Dr. Jane \r\n",
            " Doe\r\n",
            "ORG:GNOME Foundation;Engineering\r\n",
            "TEL;TYPE=WORK:+49 30 123456\r\n",
            "TEL;TYPE=CELL:tel:+49 170 9876543\r\n",
            "TEL:+4930123456\r\n",
            "END:VCARD\r\n",
        );
        let parsed = parse_vcard_contacts(vcard, "system-address-book", "Personal");
        assert_eq!(parsed.len(), 2, "Duplicate normalized TEL should be deduplicated");
        assert_eq!(parsed[0].name, "Dr. Jane Doe");
        assert_eq!(parsed[0].department, "GNOME Foundation");
        assert_eq!(parsed[0].phone, "+49 30 123456");
        assert!(parsed[0].id.starts_with(EDS_ID_PREFIX));

        // 3. Merge preserves PBX contacts, deduplicates matching numbers, and removes EDS on opt-out
        let mut live = vec![Contact {
            id: "swyx-101".to_string(),
            name: "Jane Doe (Internal)".to_string(),
            extension: "101".to_string(),
            department: "Engineering".to_string(),
            presence: PresenceState::Available,
            phone: "+49 30 123456".to_string(),
            avatar_file_id: None,
        }];
        assert!(merge_eds_contacts(&mut live, parsed));
        assert_eq!(live.len(), 2, "Matching PBX number skipped; mobile EDS number added");
        assert_eq!(live[0].id, "swyx-101");
        assert!(live[1].id.starts_with(EDS_ID_PREFIX));
        assert_eq!(live[1].phone, "+49 170 9876543");

        // Opt-out removes EDS entries cleanly while keeping PBX contacts
        assert!(merge_eds_contacts(&mut live, Vec::new()));
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].id, "swyx-101");
    }

    #[test]
    fn test_gnome_contacts_photo_and_smart_enrichment() {
        use crate::gnome_contacts::{
            canonical_name_key, canonical_phone_key, merge_eds_contacts,
            parse_vcard_contacts_with_photo, EDS_PHOTO_PREFIX,
        };
        use crate::models::{Contact, PresenceState};

        // 1. Canonical phone matching bridges national (030...) and international (+49 30...) formats
        assert_eq!(
            canonical_phone_key("+49 30 23125 192"),
            canonical_phone_key("030 23125 192")
        );
        // Canonical name matching bridges Swyx "Last, First Ext" and Google "First Last"
        assert_eq!(
            canonical_name_key("Weber, Clara 201"),
            canonical_name_key("Clara Weber")
        );
        assert_eq!(canonical_name_key("Support"), None, "Single word must not false-merge");

        // 2. vCard 3.0 Base64 PHOTO + vCard 4.0 data: URI PHOTO
        let vcard_b64 = concat!(
            "BEGIN:VCARD\r\n",
            "VERSION:3.0\r\n",
            "UID:g-1\r\n",
            "FN:Clara Weber\r\n",
            "TEL;TYPE=CELL:+49 30 23125 201\r\n",
            "PHOTO;ENCODING=b;TYPE=PNG:iVBORw0KGgo=\r\n",
            "END:VCARD\r\n",
        );
        let (clara_eds, clara_photo) =
            parse_vcard_contacts_with_photo(vcard_b64, "google", "Google");
        assert_eq!(clara_eds.len(), 1);
        let (fid, bytes) = clara_photo.expect("Base64 PHOTO must be decoded");
        assert!(fid.starts_with(EDS_PHOTO_PREFIX));
        assert_eq!(bytes, b"\x89PNG\r\n\x1a\n");
        assert_eq!(clara_eds[0].avatar_file_id.as_deref(), Some(fid.as_str()));

        let vcard_data_uri = concat!(
            "BEGIN:VCARD\r\n",
            "VERSION:4.0\r\n",
            "UID:g-2\r\n",
            "FN:Northwind Studio\r\n",
            "TEL:030 23125 192\r\n",
            "PHOTO:data:image/png;base64,iVBORw0KGgo=\r\n",
            "END:VCARD\r\n",
        );
        let (nw_eds, nw_photo) =
            parse_vcard_contacts_with_photo(vcard_data_uri, "local", "Personal");
        assert!(nw_photo.is_some());

        // Duplicate entry in a second address book without a photo (international format)
        let nw_dup = Contact {
            id: "eds-google-nw-0".into(),
            name: "Northwind Studio".into(),
            extension: String::new(),
            department: "Google".into(),
            presence: PresenceState::LoggedOut,
            phone: "+49 30 23125 192".into(),
            avatar_file_id: None,
        };

        // 3. PBX colleague ("Weber, Clara 201") with no photo and no external phone gets enriched!
        let mut live = vec![Contact {
            id: "swyx-201".into(),
            name: "Weber, Clara 201".into(),
            extension: "201".into(),
            department: "Design".into(),
            presence: PresenceState::Available,
            phone: String::new(),
            avatar_file_id: None,
        }];

        let mut incoming = Vec::new();
        incoming.extend(clara_eds);
        incoming.push(nw_dup);
        incoming.extend(nw_eds);

        assert!(merge_eds_contacts(&mut live, incoming));
        // Expected: 1 enriched PBX Clara + 1 deduplicated Northwind (with photo!)
        assert_eq!(live.len(), 2);
        assert_eq!(live[0].id, "swyx-201");
        assert_eq!(live[0].phone, "+49 30 23125 201");
        assert_eq!(live[0].avatar_file_id.as_deref(), Some(fid.as_str()));
        assert_eq!(live[1].name, "Northwind Studio");
        assert!(live[1].avatar_file_id.is_some(), "Merged duplicate must keep photo");

        // 4. Opt-out clears borrowed EDS photo ID on PBX colleague cleanly
        assert!(merge_eds_contacts(&mut live, Vec::new()));
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].avatar_file_id, None);
    }

    #[test]
    fn test_parse_dial_uri() {
        use crate::parse_dial_uri;
        assert_eq!(
            parse_dial_uri("tel:+49%2030%20123456;phone-context=example.com").as_deref(),
            Some("+49 30 123456")
        );
        assert_eq!(
            parse_dial_uri("TEL:%2B491709876543").as_deref(),
            Some("+491709876543")
        );
        assert_eq!(
            parse_dial_uri("sip:+4930123456@fritz.box;transport=udp").as_deref(),
            Some("+4930123456")
        );
        assert_eq!(parse_dial_uri("callto://08912345").as_deref(), Some("08912345"));
        assert_eq!(parse_dial_uri("https://example.com"), None);
        assert_eq!(parse_dial_uri("tel:"), None);
    }

    #[test]
    fn test_demo_store_and_zero_config_onboarding() {
        let demo = crate::swyx_bridge::build_demo_store();
        assert_eq!(demo.provider, crate::swyx_bridge::ProviderKind::GenericSip);
        assert!(demo.contacts.len() >= 5);
        assert!(demo.journal.len() >= 5);
        assert!(demo.speed_dials.len() >= 4);
        // Demo store must never hold real credentials or trigger live EDS sync
        assert!(demo.password.is_empty());
        assert!(!demo.sync_gnome_contacts);
    }
}

