use gtk4 as gtk;
use gtk::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;
use crate::models::{AudioMode, PresenceState};

#[allow(dead_code)]
pub struct HeaderComponents {
    pub header_bar: adw::HeaderBar,
    pub status_pill: gtk::Button,
    pub audio_button: gtk::Button,
}

#[allow(dead_code)]
pub fn create_header(
    view_stack: &adw::ViewStack,
    current_audio: Rc<RefCell<AudioMode>>,
    current_presence: Rc<RefCell<PresenceState>>,
) -> HeaderComponents {
    let header_bar = adw::HeaderBar::new();

    // 1. Left: User Presence Status Pill
    let status_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let avatar = adw::Avatar::builder()
        .size(28)
        .text("Benutzer")
        .show_initials(true)
        .build();
    avatar.set_size_request(28, 28);
    avatar.set_valign(gtk::Align::Start);
    avatar.set_halign(gtk::Align::Start);

    let overlay = gtk::Overlay::new();
    let overlay_size = 30; // 28 + 2
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

    let label = gtk::Label::new(Some("Benutzer"));
    label.add_css_class("heading");

    status_box.append(&overlay);
    status_box.append(&label);

    let status_pill = gtk::Button::new();
    status_pill.set_child(Some(&status_box));
    status_pill.add_css_class("flat");

    // Popover menu for presence states
    let popover = gtk::Popover::new();
    popover.set_parent(&status_pill);
    let pop_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    pop_box.set_margin_top(8);
    pop_box.set_margin_bottom(8);
    pop_box.set_margin_start(8);
    pop_box.set_margin_end(8);

    let states = [
        PresenceState::Available,
        PresenceState::Busy,
        PresenceState::Away,
        PresenceState::LoggedOut,
    ];

    for state in states {
        let btn_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let s_dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        s_dot.add_css_class("presence-badge-dot");
        s_dot.add_css_class(state.css_class());

        let s_lbl = gtk::Label::new(Some(&state.label()));
        btn_box.append(&s_dot);
        btn_box.append(&s_lbl);

        let item_btn = gtk::Button::new();
        item_btn.set_child(Some(&btn_box));
        item_btn.add_css_class("flat");

        let p_ref = current_presence.clone();
        let dot_clone = dot.clone();
        let pop_clone = popover.clone();
        item_btn.connect_clicked(move |_| {
            *p_ref.borrow_mut() = state;
            dot_clone.remove_css_class("presence-available");
            dot_clone.remove_css_class("presence-busy");
            dot_clone.remove_css_class("presence-away");
            dot_clone.remove_css_class("presence-offline");
            dot_clone.add_css_class(state.css_class());
            pop_clone.popdown();
        });

        pop_box.append(&item_btn);
    }

    popover.set_child(Some(&pop_box));
    let pop_clone = popover.clone();
    status_pill.connect_clicked(move |_| {
        pop_clone.popup();
    });

    header_bar.pack_start(&status_pill);

    // 2. Center: AdwViewSwitcher
    let view_switcher = adw::ViewSwitcher::builder()
        .stack(view_stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();
    header_bar.set_title_widget(Some(&view_switcher));

    // 3. Right: Audio Mode Toggle Button (Headset / Freisprechen)
    let audio_button = gtk::Button::from_icon_name(current_audio.borrow().icon_name());
    audio_button.set_tooltip_text(Some(&crate::i18n::trf(
        "Audio device: {device}",
        &[("device", &AudioMode::Headset.label())],
    )));
    audio_button.add_css_class("flat");

    let aud_ref = current_audio.clone();
    let btn_clone = audio_button.clone();
    audio_button.connect_clicked(move |_| {
        let new_mode = match *aud_ref.borrow() {
            AudioMode::Headset => AudioMode::Speaker,
            AudioMode::Speaker => AudioMode::Headset,
        };
        *aud_ref.borrow_mut() = new_mode;
        btn_clone.set_icon_name(new_mode.icon_name());
        btn_clone.set_tooltip_text(Some(&crate::i18n::trf(
            "Audio device: {device}",
            &[("device", &new_mode.label())],
        )));
    });

    header_bar.pack_end(&audio_button);

    HeaderComponents {
        header_bar,
        status_pill,
        audio_button,
    }
}
