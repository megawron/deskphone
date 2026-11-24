use gtk4 as gtk;
use gtk::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use chrono::Local;
use gtk::glib;
use crate::engine_bridge::EngineBridge;
use crate::models::{Contact, ChatMessage};

#[derive(Clone)]
pub struct ChatWindowHandle {
    pub window: adw::Window,
    pub receive_message: Rc<dyn Fn(String, String)>,
    pub update_presence: Rc<dyn Fn(crate::models::PresenceState)>,
}

/// Öffnet ein eigenständiges, flüchtiges Swyx-Chat-Fenster für einen Kollegen (adw::Window, non-modal).
/// Die Nachrichten leben NUR solange dieses Fenster existiert (analog SwyxIt!).
/// Sobald das Fenster geschlossen wird, werden alle Nachrichten verworfen.
pub fn open_chat_window<FCall>(
    parent: &adw::ApplicationWindow,
    contact: &Contact,
    engine_bridge: EngineBridge,
    on_trigger_call: FCall,
) -> ChatWindowHandle
where
    FCall: Fn(String, String) + 'static + Clone,
{
    let window = adw::Window::builder()
        .transient_for(parent)
        .modal(false) // Non-modal: blockiert niemals Telefonate oder das Hauptfenster!
        .default_width(320)
        .default_height(380)
        .title(&contact.name)
        .icon_name(crate::APP_ID)
        .destroy_with_parent(true)
        .build();

    // Flüchtiger Nachrichtenspeicher, der exklusiv an diesen Fensterlebenszyklus gebunden ist
    let messages: Rc<RefCell<Vec<ChatMessage>>> = Rc::new(RefCell::new(Vec::new()));

    let main_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main_box.add_css_class("chat-main-container");

    // ==========================================
    // 1. HeaderBar mit nativem Libadwaita Layout & Status
    // ==========================================
    let header_bar = adw::HeaderBar::new();
    header_bar.add_css_class("chat-header-bar");

    // Anruf-Button oben LINKS (pack_start)
    let call_btn = gtk::Button::from_icon_name("call-start-symbolic");
    call_btn.add_css_class("flat");
    call_btn.add_css_class("chat-header-call-btn");
    call_btn.set_tooltip_text(Some(&crate::i18n::trf("Call {name}", &[("name", &contact.name)])));
    call_btn.set_valign(gtk::Align::Center);

    let name_c = contact.name.clone();
    let ext_c = contact.extension.clone();
    let on_call = on_trigger_call.clone();
    call_btn.connect_clicked(move |_| {
        on_call(name_c.clone(), ext_c.clone());
    });

    header_bar.pack_start(&call_btn);

    // Zentrierter Titel mit Avatar & Presence-Status
    let title_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    title_box.set_valign(gtk::Align::Center);
    title_box.set_halign(gtk::Align::Center);

    let (avatar, avatar_dot) =
        crate::ui::create_presence_avatar_with_dot(&contact.name, Some(contact.presence), 28);
    avatar.set_valign(gtk::Align::Center);
    title_box.append(&avatar);

    let title_col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    title_col.set_valign(gtk::Align::Center);

    let name_lbl = gtk::Label::new(Some(&contact.name));
    name_lbl.add_css_class("title");
    name_lbl.set_xalign(0.0);

    let sub_text = if !contact.extension.is_empty() {
        format!("{} ({})", contact.presence.label(), contact.extension)
    } else {
        contact.presence.label().to_string()
    };
    let sub_lbl = gtk::Label::new(Some(&sub_text));
    sub_lbl.add_css_class("subtitle");
    sub_lbl.set_xalign(0.0);

    title_col.append(&name_lbl);
    title_col.append(&sub_lbl);
    title_box.append(&title_col);

    header_bar.set_title_widget(Some(&title_box));

    main_box.append(&header_bar);

    // ==========================================
    // 2. Nachrichten ScrolledWindow
    // ==========================================
    let msg_scroll = gtk::ScrolledWindow::new();
    msg_scroll.set_vexpand(true);
    msg_scroll.set_hexpand(true);
    msg_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);

    let content_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content_box.set_vexpand(true);

    // Minimalistischer Hinweis im Leerzustand
    let hint_card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    hint_card.set_valign(gtk::Align::Center);
    hint_card.set_halign(gtk::Align::Center);
    hint_card.set_vexpand(true);
    hint_card.set_margin_start(24);
    hint_card.set_margin_end(24);

    let hint_icon = gtk::Image::from_icon_name("user-available-symbolic");
    hint_icon.set_pixel_size(28);
    hint_icon.add_css_class("dim-label");
    hint_card.append(&hint_icon);

    let hint_title = gtk::Label::new(Some(&crate::i18n::tr("Instant Messaging")));
    hint_title.add_css_class("heading");
    hint_card.append(&hint_title);

    let hint_lbl = gtk::Label::new(Some(&crate::i18n::tr(
        "Send instant messages via SIP MESSAGE (RFC 3428).\nMessages are ephemeral and cleared when closing.",
    )));
    hint_lbl.set_wrap(true);
    hint_lbl.set_justify(gtk::Justification::Center);
    hint_lbl.add_css_class("caption");
    hint_lbl.add_css_class("dim-label");
    hint_card.append(&hint_lbl);

    let msg_list = gtk::Box::new(gtk::Orientation::Vertical, 8);
    msg_list.set_margin_top(10);
    msg_list.set_margin_bottom(10);
    msg_list.set_margin_start(12);
    msg_list.set_margin_end(12);
    msg_list.set_visible(false);

    content_box.append(&hint_card);
    content_box.append(&msg_list);
    msg_scroll.set_child(Some(&content_box));
    main_box.append(&msg_scroll);

    // ==========================================
    // 3. Input Bar
    // ==========================================
    let input_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    input_bar.add_css_class("chat-input-bar");

    let text_entry = gtk::Entry::new();
    text_entry.set_hexpand(true);
    text_entry.set_valign(gtk::Align::Center);
    text_entry.add_css_class("chat-text-entry");
    text_entry.set_placeholder_text(Some(&crate::i18n::tr("Message")));
    text_entry.set_tooltip_text(Some(&crate::i18n::tr("Press Enter to send")));

    let send_btn = gtk::Button::from_icon_name("mail-send-symbolic");
    send_btn.add_css_class("chat-send-btn");
    send_btn.set_valign(gtk::Align::Center);
    send_btn.set_tooltip_text(Some(&crate::i18n::tr("Send")));

    input_bar.append(&text_entry);
    input_bar.append(&send_btn);
    main_box.append(&input_bar);

    window.set_content(Some(&main_box));

    // ==========================================
    // Nachrichten rendern & absenden
    // ==========================================
    let msg_list_ref = msg_list.clone();
    let scroll_ref = msg_scroll.clone();
    let hint_card_ref = hint_card.clone();

    let render_messages = Rc::new(move |msgs: &[ChatMessage]| {
        if msgs.is_empty() {
            hint_card_ref.set_visible(true);
            msg_list_ref.set_visible(false);
            return;
        }

        hint_card_ref.set_visible(false);
        msg_list_ref.set_visible(true);

        // Vorherige Nachrichtenblasen leeren
        while let Some(child) = msg_list_ref.first_child() {
            msg_list_ref.remove(&child);
        }

        // Alle Nachrichten aufbauen
        for msg in msgs {
            let row_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            let bubble = gtk::Box::new(gtk::Orientation::Vertical, 2);

            let txt_lbl = gtk::Label::new(Some(&msg.text));
            txt_lbl.set_wrap(true);
            txt_lbl.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            txt_lbl.set_max_width_chars(32);
            txt_lbl.set_xalign(0.0);

            let time_str = msg.timestamp.format("%H:%M").to_string();
            let time_lbl = gtk::Label::new(Some(&time_str));
            time_lbl.add_css_class("chat-timestamp");

            bubble.append(&txt_lbl);
            bubble.append(&time_lbl);

            if msg.is_self {
                row_box.set_halign(gtk::Align::End);
                bubble.add_css_class("chat-bubble-outgoing");
                time_lbl.set_halign(gtk::Align::End);
            } else if msg.sender == "System" {
                row_box.set_halign(gtk::Align::Center);
                bubble.add_css_class("card");
                time_lbl.set_halign(gtk::Align::Center);
            } else {
                row_box.set_halign(gtk::Align::Start);
                bubble.add_css_class("chat-bubble-incoming");
                time_lbl.set_halign(gtk::Align::Start);
            }

            row_box.append(&bubble);
            msg_list_ref.append(&row_box);
        }

        let vadj = scroll_ref.vadjustment();
        glib::idle_add_local_once(move || {
            vadj.set_value(vadj.upper());
        });
    });

    let target_number = if !contact.extension.trim().is_empty() {
        contact.extension.trim().to_string()
    } else {
        contact.phone.trim().to_string()
    };

    let contact_id = contact.id.clone();
    let msgs_send = messages.clone();
    let entry_send = text_entry.clone();
    let r_msgs_send = render_messages.clone();
    let eb_send = engine_bridge.clone();

    let do_send = Rc::new(move || {
        let text = entry_send.text().trim().to_string();
        if text.is_empty() {
            return;
        }
        entry_send.set_text("");

        let msg = ChatMessage {
            id: format!("msg-{}", Local::now().timestamp_millis()),
            sender: "Ich".into(),
            is_self: true,
            text: text.clone(),
            timestamp: Local::now(),
        };

        {
            let mut msgs = msgs_send.borrow_mut();
            msgs.push(msg);
            r_msgs_send(&msgs);
        }

        let eb = eb_send.clone();
        let tgt = if !contact_id.is_empty() {
            format!("{};id={}", target_number, contact_id)
        } else {
            target_number.clone()
        };
        let display_target = target_number.clone();
        let msgs_err = msgs_send.clone();
        let r_msgs_err = r_msgs_send.clone();

        glib::spawn_future_local(async move {
            if let Err(err) = eb.send_chat_message(&tgt, &text).await {
                let mut msgs = msgs_err.borrow_mut();
                msgs.push(ChatMessage {
                    id: format!("sys-{}", Local::now().timestamp_millis()),
                    sender: "System".into(),
                    is_self: false,
                    text: format!("⚠️ Nachricht an {} konnte nicht zugestellt werden: {}", display_target, err),
                    timestamp: Local::now(),
                });
                r_msgs_err(&msgs);
            }
        });
    });

    let send_act = do_send.clone();
    send_btn.connect_clicked(move |_| {
        send_act();
    });

    let send_enter = do_send.clone();
    text_entry.connect_activate(move |_| {
        send_enter();
    });

    let msgs_recv = messages.clone();
    let r_msgs_recv = render_messages.clone();
    let receive_message: Rc<dyn Fn(String, String)> = Rc::new(move |sender_name: String, text: String| {
        let mut msgs = msgs_recv.borrow_mut();
        msgs.push(ChatMessage {
            id: format!("in-{}", Local::now().timestamp_millis()),
            sender: sender_name,
            is_self: false,
            text,
            timestamp: Local::now(),
        });
        r_msgs_recv(&msgs);
    });

    // Esc-Taste schließt das Fenster sofort
    let key_controller = gtk::EventControllerKey::new();
    let win_esc = window.clone();
    key_controller.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == gtk::gdk::Key::Escape {
            win_esc.close();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

    window.present();
    text_entry.grab_focus();

    let ext_for_pres = contact.extension.clone();
    let sub_lbl_pres = sub_lbl.clone();
    let update_presence: Rc<dyn Fn(crate::models::PresenceState)> = Rc::new(move |new_p| {
        crate::ui::update_presence_dot(&avatar_dot, Some(new_p));
        let txt = if !ext_for_pres.is_empty() {
            format!("{} ({})", new_p.label(), ext_for_pres)
        } else {
            new_p.label().to_string()
        };
        sub_lbl_pres.set_text(&txt);
    });

    ChatWindowHandle {
        window,
        receive_message,
        update_presence,
    }
}
