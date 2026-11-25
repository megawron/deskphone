use gtk4 as gtk;
use gtk::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
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

/// Builds a circular in-call action button with tooltip and accessible label.
fn call_action_button(icon: &str, tooltip: &str, a11y_label: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(icon);
    button.add_css_class("call-btn-circular");
    button.set_valign(gtk::Align::Center);
    button.set_tooltip_text(Some(tooltip));
    button.update_property(&[gtk::accessible::Property::Label(a11y_label)]);
    button
}

pub fn create_incall_view<FSendDtmf>(on_send_dtmf: FSendDtmf) -> InCallComponents
where
    FSendDtmf: Fn(char) + 'static + Clone,
{
    use crate::i18n::tr;

    // Root page: optional held-call banner on top, scrollable centered call content below
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    container.set_hexpand(true);
    container.set_vexpand(true);

    // 1. Held call banner (swap / merge when a second call is on hold)
    let held_banner = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    held_banner.add_css_class("flush-held-banner");
    held_banner.set_hexpand(true);

    let held_icon = gtk::Image::from_icon_name("media-playback-pause-symbolic");
    held_icon.set_pixel_size(16);
    held_banner.append(&held_icon);

    let held_pill_label = gtk::Label::new(None);
    held_pill_label.add_css_class("heading");
    held_pill_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    held_pill_label.set_hexpand(true);
    held_pill_label.set_xalign(0.0);
    held_banner.append(&held_pill_label);

    let held_pill_button = gtk::Button::with_label(&tr("Swap"));
    held_pill_button.add_css_class("swap-btn-minimal");
    held_pill_button.set_valign(gtk::Align::Center);
    held_pill_button.set_tooltip_text(Some(&tr("Swap between active and held call")));
    held_banner.append(&held_pill_button);

    let held_transfer_button = gtk::Button::with_label(&tr("Merge"));
    held_transfer_button.add_css_class("connect-transfer-btn");
    held_transfer_button.set_valign(gtk::Align::Center);
    held_transfer_button.set_tooltip_text(Some(&tr("Connect both calls (attended transfer)")));
    held_banner.append(&held_transfer_button);

    held_banner.set_visible(false);
    crate::ui::attach_call_drag_source_with_tag(&held_banner, "CALL_TRANSFER_HELD", || true);
    container.append(&held_banner);

    // 2. Scrollable, width-clamped call content (never clipped on small windows)
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_height(false)
        .vexpand(true)
        .hexpand(true)
        .build();

    let clamp = adw::Clamp::builder()
        .maximum_size(420)
        .tightening_threshold(320)
        .build();

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_valign(gtk::Align::Center);
    content.set_vexpand(true);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(18);
    content.set_margin_end(18);
    crate::ui::attach_call_drag_source(&content);

    // Caller identity: avatar (AdwAvatar for contacts/photos, calm neutral circle for unknown numbers)
    let avatar_stack = gtk::Stack::new();
    avatar_stack.set_halign(gtk::Align::Center);
    avatar_stack.set_margin_bottom(14);

    let avatar = adw::Avatar::builder()
        .size(88)
        .show_initials(true)
        .halign(gtk::Align::Center)
        .build();
    avatar_stack.add_named(&avatar, Some("contact"));

    let neutral_avatar = adw::Bin::new();
    neutral_avatar.add_css_class("avatar-neutral-number");
    neutral_avatar.set_size_request(88, 88);
    neutral_avatar.set_halign(gtk::Align::Center);
    neutral_avatar.set_valign(gtk::Align::Center);
    let neutral_icon = gtk::Image::from_icon_name("avatar-default-symbolic");
    neutral_icon.set_pixel_size(40);
    neutral_icon.set_halign(gtk::Align::Center);
    neutral_icon.set_valign(gtk::Align::Center);
    neutral_avatar.set_child(Some(&neutral_icon));
    avatar_stack.add_named(&neutral_avatar, Some("number"));
    avatar_stack.set_visible_child_name("number");
    content.append(&avatar_stack);

    let caller_name_label = gtk::Label::new(None);
    caller_name_label.add_css_class("title-2");
    caller_name_label.set_justify(gtk::Justification::Center);
    caller_name_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    caller_name_label.set_max_width_chars(28);
    caller_name_label.set_selectable(false);
    content.append(&caller_name_label);

    let caller_ext_label = gtk::Label::new(None);
    caller_ext_label.add_css_class("dim-label");
    caller_ext_label.set_justify(gtk::Justification::Center);
    caller_ext_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    caller_ext_label.set_max_width_chars(36);
    caller_ext_label.set_margin_top(4);
    content.append(&caller_ext_label);

    // Call duration with a subtle audio activity indicator
    let timer_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    timer_row.set_halign(gtk::Align::Center);
    timer_row.set_margin_top(14);

    let timer_label = gtk::Label::new(Some("00:00"));
    timer_label.add_css_class("title-3");
    timer_label.add_css_class("numeric");
    timer_row.append(&timer_label);

    let waveform_box = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    waveform_box.set_valign(gtk::Align::Center);
    for h in [6, 11, 16, 12, 18, 13, 8] {
        let bar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bar.add_css_class("audio-waveform-bar");
        bar.set_valign(gtk::Align::Center);
        bar.set_size_request(3, h);
        waveform_box.append(&bar);
    }
    timer_row.append(&waveform_box);
    content.append(&timer_row);

    // Call actions: Mute | Keypad | Transfer | Hold, then Hang Up slightly set apart
    let actions_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    actions_box.set_halign(gtk::Align::Center);
    actions_box.set_margin_top(24);

    let mute_button = call_action_button("audio-input-microphone-symbolic", &tr("Mute"), &tr("Mute"));
    actions_box.append(&mute_button);

    let keypad_button = call_action_button(
        "input-dialpad-symbolic",
        &tr("Show or hide keypad (DTMF for voice menus)"),
        &tr("Keypad"),
    );
    actions_box.append(&keypad_button);

    let transfer_button = call_action_button(
        "edit-redo-symbolic",
        &tr("Transfer (Opens contacts or drag line onto contact)"),
        &tr("Transfer Call"),
    );
    actions_box.append(&transfer_button);

    let hold_button = call_action_button("media-playback-pause-symbolic", &tr("Hold"), &tr("Hold"));
    actions_box.append(&hold_button);

    let hangup_button = gtk::Button::from_icon_name("call-stop-symbolic");
    hangup_button.add_css_class("call-btn-destructive");
    hangup_button.set_valign(gtk::Align::Center);
    hangup_button.set_margin_start(12);
    hangup_button.set_tooltip_text(Some(&tr("Hang Up")));
    hangup_button.update_property(&[gtk::accessible::Property::Label(&tr("Hang Up"))]);
    actions_box.append(&hangup_button);

    content.append(&actions_box);

    // 3. DTMF keypad for voice menus (collapsed by default, digits can also be typed)
    let dtmf_digits_buffer = Rc::new(RefCell::new(String::new()));

    let keypad_area = gtk::Box::new(gtk::Orientation::Vertical, 8);
    keypad_area.set_margin_top(16);
    keypad_area.set_halign(gtk::Align::Center);

    let dtmf_display_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    dtmf_display_box.add_css_class("card");
    dtmf_display_box.set_size_request(232, 36);

    let dtmf_digits_label = gtk::Label::new(Some(&tr("Ready for digits")));
    dtmf_digits_label.set_hexpand(true);
    dtmf_digits_label.set_margin_start(36);
    dtmf_digits_label.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    dtmf_digits_label.add_css_class("heading");
    dtmf_display_box.append(&dtmf_digits_label);

    let dtmf_clear_btn = gtk::Button::from_icon_name("edit-clear-symbolic");
    dtmf_clear_btn.add_css_class("flat");
    dtmf_clear_btn.add_css_class("circular");
    dtmf_clear_btn.set_valign(gtk::Align::Center);
    dtmf_clear_btn.set_margin_end(2);
    dtmf_clear_btn.set_tooltip_text(Some(&tr("Clear digits")));
    let buf_clear = dtmf_digits_buffer.clone();
    let lbl_clear = dtmf_digits_label.clone();
    dtmf_clear_btn.connect_clicked(move |_| {
        buf_clear.borrow_mut().clear();
        lbl_clear.set_text(&tr("Ready for digits"));
    });
    dtmf_display_box.append(&dtmf_clear_btn);
    keypad_area.append(&dtmf_display_box);

    let dtmf_grid = gtk::Grid::new();
    dtmf_grid.set_row_spacing(4);
    dtmf_grid.set_column_spacing(12);
    dtmf_grid.set_column_homogeneous(true);
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

        let s_lbl = gtk::Label::new(Some(if sub.is_empty() { " " } else { sub }));
        s_lbl.add_css_class("caption");
        s_lbl.add_css_class("dim-label");
        btn_box.append(&s_lbl);

        let btn = gtk::Button::new();
        btn.set_child(Some(&btn_box));
        btn.add_css_class("flat");
        btn.set_size_request(64, 44);
        btn.update_property(&[gtk::accessible::Property::Label(k)]);

        let ch = k.chars().next().unwrap_or(' ');
        let on_dtmf_c = on_send_dtmf.clone();
        let buf_c = dtmf_digits_buffer.clone();
        let lbl_c = dtmf_digits_label.clone();
        btn.connect_clicked(move |_| {
            let mut b = buf_c.borrow_mut();
            b.push(ch);
            lbl_c.set_text(&b);
            on_dtmf_c(ch);
        });

        dtmf_grid.attach(&btn, col, row, 1, 1);
    }
    keypad_area.append(&dtmf_grid);

    let dtmf_revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .reveal_child(false)
        .child(&keypad_area)
        .build();
    content.append(&dtmf_revealer);

    clamp.set_child(Some(&content));
    scroller.set_child(Some(&clamp));
    container.append(&scroller);

    // Keypad toggle: like GNOME Calls, the large avatar makes room for the keypad and the
    // spacing tightens, so everything fits the default window size without scrolling.
    let rev_toggle = dtmf_revealer.clone();
    let kp_btn_toggle = keypad_button.clone();
    let avatar_toggle = avatar_stack.clone();
    let content_toggle = content.clone();
    let timer_row_toggle = timer_row.clone();
    let actions_toggle = actions_box.clone();
    keypad_button.connect_clicked(move |_| {
        let show = !rev_toggle.reveals_child();
        rev_toggle.set_reveal_child(show);
        avatar_toggle.set_visible(!show);
        let (outer, gap) = if show { (12, 16) } else { (24, 24) };
        content_toggle.set_margin_top(outer);
        content_toggle.set_margin_bottom(outer);
        timer_row_toggle.set_margin_top(if show { 8 } else { 14 });
        actions_toggle.set_margin_top(gap);
        if show {
            kp_btn_toggle.add_css_class("suggested-action");
        } else {
            kp_btn_toggle.remove_css_class("suggested-action");
        }
    });

    // Avatar follows the caller: contact photo / initials for named contacts, calm neutral
    // circle for unknown phone numbers. Also drop the leading number from the details line
    // when it has the same digits as the title (e.g. "+49 7164 14950-17" vs "+4971641495017").
    let refresh_identity: Rc<dyn Fn()> = {
        let av = avatar.clone();
        let av_stack = avatar_stack.clone();
        let name_lbl = caller_name_label.clone();
        let ext_lbl = caller_ext_label.clone();
        Rc::new(move || {
            let name = name_lbl.text().to_string();
            let is_number = crate::ui::is_phone_number(&name);
            let photo = crate::ui::get_contact_photo(&name);
            if is_number && photo.is_none() {
                av_stack.set_visible_child_name("number");
            } else {
                av.set_text(Some(&crate::ui::clean_name_for_avatar(&name)));
                av.set_show_initials(!is_number);
                match photo {
                    Some(tex) => av.set_custom_image(Some(&tex)),
                    None => av.set_custom_image(None::<&gtk::gdk::Paintable>),
                }
                av_stack.set_visible_child_name("contact");
            }

            let details = ext_lbl.text().to_string();
            if let Some((first_part, rest)) = details.split_once(" • ") {
                let n_digits = crate::gnome_contacts::normalize_phone_digits(&name);
                let p_digits = crate::gnome_contacts::normalize_phone_digits(first_part);
                let same_raw = !name.trim().is_empty() && first_part.trim() == name.trim();
                let same_digits = !n_digits.is_empty() && n_digits == p_digits;
                if same_raw || same_digits {
                    ext_lbl.set_text(rest);
                }
            }
        })
    };
    let ri_name = refresh_identity.clone();
    caller_name_label.connect_label_notify(move |_| ri_name());
    let ri_ext = refresh_identity.clone();
    caller_ext_label.connect_label_notify(move |_| ri_ext());

    // Keyboard: type DTMF digits directly
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
            lbl_key.set_text(&b);
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
