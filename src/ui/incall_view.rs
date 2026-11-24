use gtk4 as gtk;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[allow(dead_code)]
pub struct InCallComponents {
    pub container: gtk::Box,
    pub held_banner: gtk::Box,
    pub held_pill_button: gtk::Button,
    pub held_transfer_button: gtk::Button,
    pub held_pill_label: gtk::Label,
    pub caller_name_label: gtk::Label,
    pub caller_ext_label: gtk::Label,
    pub timer_label: gtk::Label,
    pub mute_button: gtk::Button,
    pub hold_button: gtk::Button,
    pub transfer_button: gtk::Button,
    pub keypad_button: gtk::Button,
    pub hangup_button: gtk::Button,
    pub waveform_box: gtk::Box,
    pub dtmf_digits_label: gtk::Label,
    pub dtmf_revealer: gtk::Revealer,
}

pub fn create_incall_view<FSendDtmf>(on_send_dtmf: FSendDtmf) -> InCallComponents
where
    FSendDtmf: Fn(char) + 'static + Clone,
{
    // Root container: Full width (flush), vertical stack
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    container.set_valign(gtk::Align::Fill);
    container.set_halign(gtk::Align::Fill);
    container.set_hexpand(true);

    // 1. Flush Held Call Banner (Edge-to-Edge Umschalt-Leiste für Zweitanruf / Makeln)
    let held_banner = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    held_banner.add_css_class("flush-held-banner");
    held_banner.set_hexpand(true);
    held_banner.set_valign(gtk::Align::Start);

    let held_icon = gtk::Image::from_icon_name("media-playback-pause-symbolic");
    held_icon.set_pixel_size(16);
    held_banner.append(&held_icon);

    let held_pill_label = gtk::Label::new(Some("Stefan K. (812) • In Warteschleife 01:14"));
    held_pill_label.add_css_class("heading");
    held_pill_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    held_banner.append(&held_pill_label);

    let banner_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    banner_spacer.set_hexpand(true);
    held_banner.append(&banner_spacer);

    let held_pill_button = gtk::Button::with_label(&crate::i18n::tr("Swap"));
    held_pill_button.add_css_class("swap-btn-minimal");
    held_pill_button.set_tooltip_text(Some(&crate::i18n::tr("Swap between active and held call")));
    held_banner.append(&held_pill_button);

    let held_transfer_button = gtk::Button::with_label(&crate::i18n::tr("Merge"));
    held_transfer_button.add_css_class("connect-transfer-btn");
    held_transfer_button.set_tooltip_text(Some(&crate::i18n::tr("Connect both calls (attended transfer)")));
    held_banner.append(&held_transfer_button);

    // Default hidden, only visible when a call is actually held
    held_banner.set_visible(false);
    crate::ui::attach_call_drag_source_with_tag(&held_banner, "CALL_TRANSFER_HELD", || true);
    container.append(&held_banner);

    // 2. Wide Horizontal Flush Active Call Surface
    let call_surface = gtk::Box::new(gtk::Orientation::Vertical, 20);
    call_surface.add_css_class("flush-call-surface");
    call_surface.set_hexpand(true);
    crate::ui::attach_call_drag_source(&call_surface);

    // Top Row: Avatar + Name + Extension (Left) <---> Timer + Waveform (Right)
    let top_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    top_row.set_hexpand(true);
    top_row.set_valign(gtk::Align::Center);

    // Left Column: Avatar + Caller Info
    let left_info = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    left_info.set_valign(gtk::Align::Center);

    let avatar_img = gtk::Image::from_icon_name("avatar-default-symbolic");
    avatar_img.set_pixel_size(54);
    left_info.append(&avatar_img);

    let text_col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text_col.set_valign(gtk::Align::Center);

    let caller_name_label = gtk::Label::new(Some("Max Mustermann"));
    caller_name_label.add_css_class("title-2");
    caller_name_label.set_halign(gtk::Align::Start);
    caller_name_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text_col.append(&caller_name_label);

    let caller_ext_label = gtk::Label::new(Some("101 • Leitung 1 • HD Voice"));
    caller_ext_label.add_css_class("dim-label");
    caller_ext_label.set_halign(gtk::Align::Start);
    caller_ext_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text_col.append(&caller_ext_label);

    left_info.append(&text_col);
    top_row.append(&left_info);

    // Spacer
    let row_spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row_spacer.set_hexpand(true);
    top_row.append(&row_spacer);

    // Right Column: Big Timer + Audio Waveform
    let right_info = gtk::Box::new(gtk::Orientation::Vertical, 4);
    right_info.set_valign(gtk::Align::Center);
    right_info.set_halign(gtk::Align::End);

    let timer_label = gtk::Label::new(Some("00:00"));
    timer_label.add_css_class("timer-text-flush");
    timer_label.set_halign(gtk::Align::End);
    right_info.append(&timer_label);

    // Waveform simulation bars (clean, subtle green)
    let waveform_box = gtk::Box::new(gtk::Orientation::Horizontal, 3);
    waveform_box.set_halign(gtk::Align::End);
    let heights = [8, 14, 20, 16, 24, 18, 12];
    for h in heights {
        let bar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bar.add_css_class("audio-waveform-bar");
        bar.set_size_request(3, h);
        waveform_box.append(&bar);
    }
    right_info.append(&waveform_box);

    top_row.append(&right_info);
    call_surface.append(&top_row);

    // Bottom Row: Minimalist In-Call Action Buttons [ Stumm | Halten | Weiterleiten | Tasten | Auflegen ]
    let actions_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions_box.set_halign(gtk::Align::Center);
    actions_box.set_margin_top(4);

    // Mute
    let mute_button = gtk::Button::from_icon_name("audio-input-microphone-symbolic");
    mute_button.add_css_class("call-btn-circular");
    mute_button.add_css_class("flat");
    mute_button.set_tooltip_text(Some(&crate::i18n::tr("Mute")));
    mute_button.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr("Mute"))]);
    actions_box.append(&mute_button);

    // Hold / Pause
    let hold_button = gtk::Button::from_icon_name("media-playback-pause-symbolic");
    hold_button.add_css_class("call-btn-circular");
    hold_button.add_css_class("flat");
    hold_button.set_tooltip_text(Some(&crate::i18n::tr("Hold")));
    hold_button.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr("Hold"))]);
    actions_box.append(&hold_button);

    // Transfer
    let transfer_button = gtk::Button::from_icon_name("edit-redo-symbolic");
    transfer_button.add_css_class("call-btn-circular");
    transfer_button.add_css_class("flat");
    transfer_button.set_tooltip_text(Some(&crate::i18n::tr("Transfer (Opens contacts or drag line onto contact)")));
    transfer_button.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr("Transfer Call"))]);
    actions_box.append(&transfer_button);

    // Keypad (DTMF) Button
    let keypad_button = gtk::Button::from_icon_name("input-dialpad-symbolic");
    keypad_button.add_css_class("call-btn-circular");
    keypad_button.add_css_class("flat");
    keypad_button.add_css_class("suggested-action"); // active by default
    keypad_button.set_tooltip_text(Some(&crate::i18n::tr("Show or hide keypad (DTMF for voice menus)")));
    keypad_button.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr("Keypad"))]);
    actions_box.append(&keypad_button);

    // Hang Up (Destructive red circular button)
    let hangup_button = gtk::Button::from_icon_name("call-stop-symbolic");
    hangup_button.add_css_class("call-btn-destructive");
    hangup_button.set_tooltip_text(Some(&crate::i18n::tr("Hang Up")));
    hangup_button.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr("Hang Up"))]);
    actions_box.append(&hangup_button);

    call_surface.append(&actions_box);
    container.append(&call_surface);

    // 3. Lower Area: DTMF Keypad with Revealer & Dialed Digits History
    let dtmf_digits_buffer = Rc::new(RefCell::new(String::new()));

    let lower_area = gtk::Box::new(gtk::Orientation::Vertical, 10);
    lower_area.set_hexpand(true);
    lower_area.set_vexpand(true);
    lower_area.set_margin_top(12);
    lower_area.set_margin_bottom(16);
    lower_area.set_valign(gtk::Align::Center);
    lower_area.set_halign(gtk::Align::Center);

    // DTMF Header & Digit Display
    let dtmf_header_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    dtmf_header_box.set_halign(gtk::Align::Center);

    let keypad_title = gtk::Label::new(Some(&crate::i18n::tr("DTMF Keypad (Voice Menu / IVR)")));
    keypad_title.add_css_class("dim-label");
    keypad_title.add_css_class("caption");
    dtmf_header_box.append(&keypad_title);

    lower_area.append(&dtmf_header_box);

    // Gesendete Ziffern Feed / Pill
    let dtmf_display_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    dtmf_display_box.set_halign(gtk::Align::Center);
    dtmf_display_box.add_css_class("card");
    dtmf_display_box.set_margin_bottom(6);
    dtmf_display_box.set_size_request(240, 36);

    let dtmf_digits_label = gtk::Label::new(Some(&crate::i18n::tr("Ready for digits")));
    dtmf_digits_label.set_hexpand(true);
    dtmf_digits_label.add_css_class("heading");
    dtmf_display_box.append(&dtmf_digits_label);

    let dtmf_clear_btn = gtk::Button::from_icon_name("edit-clear-symbolic");
    dtmf_clear_btn.add_css_class("flat");
    dtmf_clear_btn.add_css_class("circular");
    dtmf_clear_btn.set_tooltip_text(Some(&crate::i18n::tr("Clear digits")));

    let buf_clear = dtmf_digits_buffer.clone();
    let lbl_clear = dtmf_digits_label.clone();
    dtmf_clear_btn.connect_clicked(move |_| {
        buf_clear.borrow_mut().clear();
        lbl_clear.set_text(&crate::i18n::tr("Ready for digits"));
    });
    dtmf_display_box.append(&dtmf_clear_btn);
    lower_area.append(&dtmf_display_box);

    // 12-Tasten DTMF Raster
    let dtmf_grid = gtk::Grid::new();
    dtmf_grid.set_row_spacing(8);
    dtmf_grid.set_column_spacing(12);
    dtmf_grid.set_halign(gtk::Align::Center);

    let dtmf_keys = [
        ("1", "", 0, 0), ("2", "ABC", 1, 0), ("3", "DEF", 2, 0),
        ("4", "GHI", 0, 1), ("5", "JKL", 1, 1), ("6", "MNO", 2, 1),
        ("7", "PQRS", 0, 2), ("8", "TUV", 1, 2), ("9", "WXYZ", 2, 2),
        ("*", "", 0, 3), ("0", "+", 1, 3), ("#", "", 2, 3),
    ];

    for (k, sub, col, row) in dtmf_keys {
        let btn_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        btn_box.set_valign(gtk::Align::Center);
        btn_box.set_halign(gtk::Align::Center);

        let d_lbl = gtk::Label::new(Some(k));
        d_lbl.add_css_class("title-3");
        btn_box.append(&d_lbl);

        if !sub.is_empty() {
            let s_lbl = gtk::Label::new(Some(sub));
            s_lbl.add_css_class("caption");
            s_lbl.add_css_class("dim-label");
            btn_box.append(&s_lbl);
        }

        let btn = gtk::Button::new();
        btn.set_child(Some(&btn_box));
        btn.add_css_class("flat");
        btn.add_css_class("circular");
        btn.set_size_request(54, 48);

        let ch = k.chars().next().unwrap_or(' ');
        let on_dtmf_c = on_send_dtmf.clone();
        let buf_c = dtmf_digits_buffer.clone();
        let lbl_c = dtmf_digits_label.clone();
        btn.connect_clicked(move |_| {
            let mut b = buf_c.borrow_mut();
            b.push(ch);
            lbl_c.set_text(&crate::i18n::trf("Digits: {digits}", &[("digits", &*b)]));
            on_dtmf_c(ch);
        });

        dtmf_grid.attach(&btn, col, row, 1, 1);
    }
    lower_area.append(&dtmf_grid);

    let dtmf_revealer = gtk::Revealer::new();
    dtmf_revealer.set_child(Some(&lower_area));
    dtmf_revealer.set_reveal_child(true); // revealed by default
    dtmf_revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);

    // Toggle button handler
    let rev_toggle = dtmf_revealer.clone();
    let kp_btn_toggle = keypad_button.clone();
    keypad_button.connect_clicked(move |_| {
        let next_state = !rev_toggle.reveals_child();
        rev_toggle.set_reveal_child(next_state);
        if next_state {
            kp_btn_toggle.add_css_class("suggested-action");
        } else {
            kp_btn_toggle.remove_css_class("suggested-action");
        }
    });

    container.append(&dtmf_revealer);

    // Keyboard Controller: Erlaubt Tippen der DTMF-Ziffern direkt auf der Tastatur
    let key_controller = gtk::EventControllerKey::new();
    let on_dtmf_key = on_send_dtmf;
    let buf_key = dtmf_digits_buffer.clone();
    let lbl_key = dtmf_digits_label.clone();
    key_controller.connect_key_pressed(move |_, keyval, _, _| {
        let opt_char = match keyval.to_unicode() {
            Some(c) if c.is_ascii_digit() || c == '*' || c == '#' => Some(c),
            _ => None,
        };

        if let Some(ch) = opt_char {
            let mut b = buf_key.borrow_mut();
            b.push(ch);
            lbl_key.set_text(&crate::i18n::trf("Digits: {digits}", &[("digits", &*b)]));
            on_dtmf_key(ch);
            gtk::glib::Propagation::Stop
        } else {
            gtk::glib::Propagation::Proceed
        }
    });
    container.add_controller(key_controller);

    InCallComponents {
        container,
        held_banner,
        held_pill_button,
        held_transfer_button,
        held_pill_label,
        caller_name_label,
        caller_ext_label,
        timer_label,
        mute_button,
        hold_button,
        transfer_button,
        keypad_button,
        hangup_button,
        waveform_box,
        dtmf_digits_label,
        dtmf_revealer,
    }
}
