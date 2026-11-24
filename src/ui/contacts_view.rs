use gtk4 as gtk;
use gtk::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use crate::models::Contact;

#[derive(Clone)]
pub struct ContactsViewComponents {
    pub container: gtk::Box,
    pub reload: Rc<dyn Fn()>,
    pub refresh_presence: Rc<dyn Fn()>,
    pub search_bar: gtk::SearchBar,
}

pub fn create_contacts_view<FCall, FTransfer, FSelectNum, FChat, FFavToggle>(
    contacts: Rc<RefCell<Vec<Contact>>>,
    speed_dials: Rc<RefCell<Vec<crate::models::SpeedDialEntry>>>,
    toast_overlay: adw::ToastOverlay,
    on_trigger_call: FCall,
    on_transfer_call: FTransfer,
    on_select_number: FSelectNum,
    on_open_chat: FChat,
    on_fav_toggle: FFavToggle,
) -> ContactsViewComponents
where
    FCall: Fn(String, String) + 'static + Clone,
    FTransfer: Fn(String, String) + 'static + Clone,
    FSelectNum: Fn(String) + 'static + Clone,
    FChat: Fn(Contact) + 'static + Clone,
    FFavToggle: Fn() + 'static + Clone,
{
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);

    // 1. Sliding Search Bar (GNOME HIG Standard)
    let search_bar = gtk::SearchBar::new();
    let search_entry = gtk::SearchEntry::new();
    search_entry.set_placeholder_text(Some(&crate::i18n::tr("Search contacts")));
    search_entry.set_width_chars(10);
    search_entry.add_css_class("search-dial-entry");
    search_entry.set_hexpand(true);
    search_entry.set_margin_start(8);
    search_entry.set_margin_end(8);
    search_entry.set_margin_top(6);
    search_entry.set_margin_bottom(6);

    search_bar.set_child(Some(&search_entry));
    search_bar.connect_entry(&search_entry);
    search_bar.set_search_mode(false);
    search_bar.set_show_close_button(false);
    container.append(&search_bar);

    // 2. Contacts Direct List (Edge-to-edge, minimal & kompakt)
    let list_box = gtk::ListBox::new();
    list_box.add_css_class("direct-list");
    list_box.set_selection_mode(gtk::SelectionMode::None);
    list_box.set_valign(gtk::Align::Start);
    list_box.set_show_separators(false);

    struct ContactRowItem {
        id: String,
        extension: String,
        name_lower: String,
        ext_lower: String,
        dept_lower: String,
        dot: gtk::Box,
        row: adw::ActionRow,
        is_fav: bool,
    }

    let row_items: Rc<RefCell<Vec<ContactRowItem>>> = Rc::new(RefCell::new(Vec::new()));

    let lb_pop = list_box.clone();
    let contacts_pop = contacts.clone();
    let speed_dials_pop = speed_dials.clone();
    let toast_pop = toast_overlay.clone();
    let on_fav_toggle_pop = on_fav_toggle.clone();
    let on_call_pop = on_trigger_call.clone();
    let on_transfer_pop = on_transfer_call.clone();
    let on_sel_num_pop = on_select_number.clone();
    let on_chat_pop = on_open_chat.clone();
    let row_items_pop = row_items.clone();
    let pop_reload_holder: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let pop_reload_for_row = pop_reload_holder.clone();

    let empty_contacts_status = adw::StatusPage::builder()
        .icon_name("system-users-symbolic")
        .title(&crate::i18n::tr("No Contacts"))
        .description(&crate::i18n::tr("Contacts appear here once connected to your phone system."))
        .vexpand(true)
        .visible(false)
        .build();
    let empty_contacts_pop = empty_contacts_status.clone();

    let populate_rows = move || {
        while let Some(child) = lb_pop.first_child() {
            lb_pop.remove(&child);
        }
        row_items_pop.borrow_mut().clear();

        let prefix_size_group = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
        let suffix_size_group = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
        let is_dark = adw::StyleManager::default().is_dark();
        let star_color = if is_dark { "#f6d32d" } else { "#e5a50a" };
        let mut count = 0usize;

        for c in contacts_pop.borrow().iter() {
            let dial_target = if !c.extension.is_empty() {
                c.extension.clone()
            } else {
                c.phone.clone()
            };

            let is_fav = speed_dials_pop.borrow().iter().any(|sd| {
                (!sd.number.is_empty() && sd.number == dial_target)
                    || (!sd.name.is_empty() && sd.name == c.name)
            });

            let row = adw::ActionRow::new();
            row.set_activatable(true);
            row.set_title_lines(1);
            row.set_subtitle_lines(1);

            let ext_click = dial_target.clone();
            let on_sel = on_sel_num_pop.clone();
            row.connect_activated(move |_| {
                on_sel(ext_click.clone());
            });

            if is_fav {
                row.set_title(&format!("{} <span foreground=\"{}\">★</span>", gtk::glib::markup_escape_text(&c.name), star_color));
            } else {
                row.set_title(&c.name);
            }
            if !c.extension.is_empty() {
                row.set_subtitle(&crate::i18n::trf("Extension {ext}", &[("ext", &c.extension)]));
            } else if !c.department.is_empty() {
                row.set_subtitle(&format!("{} • {}", c.phone, c.department));
            } else {
                row.set_subtitle(&c.phone);
            }

            let status_color = match c.presence {
                crate::models::PresenceState::Available => "#2ec27e",
                crate::models::PresenceState::Busy => "#e01b24",
                crate::models::PresenceState::Away => "#f6d32d",
                crate::models::PresenceState::LoggedOut => "#77767b",
                crate::models::PresenceState::Ringing => "#ff7800",
            };

            let fav_text = if is_fav {
                format!(" • {}", crate::i18n::tr("Favorite (Speed Dial)"))
            } else {
                String::new()
            };
            let ext_lbl = crate::i18n::tr("Extension");
            let direct_lbl = crate::i18n::tr("Direct");
            let status_lbl = crate::i18n::tr("Status");
            let hint_lbl = crate::i18n::tr("Drag line here or right-click to transfer call");
            let tooltip_markup = format!(
                "<b>{}</b> • {}{}\n\
                 {}: {} • {}: {}\n\
                 {}: <span foreground=\"{}\">{}</span>\n\
                 <span size=\"small\" alpha=\"75%\">{}</span>",
                gtk::glib::markup_escape_text(&c.name),
                gtk::glib::markup_escape_text(&c.department),
                fav_text,
                ext_lbl,
                gtk::glib::markup_escape_text(&c.extension),
                direct_lbl,
                gtk::glib::markup_escape_text(&c.phone),
                status_lbl,
                status_color,
                c.presence.label(),
                hint_lbl
            );
            row.set_tooltip_markup(Some(&tooltip_markup));

            // Drag & Drop Ziel: Leitung / Gespräch direkt auf diesen Kontakt ziehen zum Weiterleiten
            crate::ui::attach_call_drop_target(
                &row,
                c.name.clone(),
                dial_target.clone(),
                on_transfer_pop.clone(),
            );

            // Profilbild / Initialen-Avatar mit rundem Live-Status-Punkt (nur bei PBX-Kontakten)
            let dot_presence = c.has_pbx_presence().then_some(c.presence);
            let (avatar, dot) = crate::ui::create_presence_avatar_with_dot(&c.name, dot_presence, 32);
            prefix_size_group.add_widget(&avatar);
            row.add_prefix(&avatar);

            let suffix_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            suffix_box.set_valign(gtk::Align::Center);
            suffix_box.set_halign(gtk::Align::End);

            // Chat button (Nur bei internen Swyx-Kollegen mit Durchwahl anzeigen)
            if c.is_internal() {
                let chat_btn = gtk::Button::from_icon_name("chat-message-new-symbolic");
                chat_btn.add_css_class("flat");
                chat_btn.add_css_class("circular");
                chat_btn.set_tooltip_text(Some(&crate::i18n::trf("Open chat with {name}", &[("name", &c.name)])));
                chat_btn.update_property(&[gtk::accessible::Property::Label(&crate::i18n::trf("Chat with {name}", &[("name", &c.name)]))]);

                let c_chat = c.clone();
                let on_c = on_chat_pop.clone();
                chat_btn.connect_clicked(move |_| {
                    on_c(c_chat.clone());
                });
                suffix_box.append(&chat_btn);
            }

            // Call button
            let call_btn = gtk::Button::from_icon_name("call-start-symbolic");
            call_btn.add_css_class("flat");
            call_btn.add_css_class("circular");
            call_btn.add_css_class("contact-call-btn");
            call_btn.set_tooltip_text(Some(&crate::i18n::trf("Call {name} ({number})", &[("name", &c.name), ("number", &dial_target)])));
            call_btn.update_property(&[gtk::accessible::Property::Label(&crate::i18n::trf("Call {name}", &[("name", &c.name)]))]);

            let name = c.name.clone();
            let ext = dial_target.clone();
            let trigger = on_call_pop.clone();
            call_btn.connect_clicked(move |_| {
                trigger(name.clone(), ext.clone());
            });
            suffix_box.append(&call_btn);
            suffix_size_group.add_widget(&suffix_box);
            row.add_suffix(&suffix_box);

            // Kontextmenü per Rechtsklick (sauber an suffix_box statt direkt an AdwActionRow)
            let popover = gtk::Popover::new();
            popover.set_parent(&suffix_box);
            popover.set_has_arrow(true);
            popover.set_position(gtk::PositionType::Bottom);
            let pop_unparent = popover.clone();
            suffix_box.connect_destroy(move |_| {
                pop_unparent.unparent();
            });

            let menu_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            menu_box.set_margin_top(4);
            menu_box.set_margin_bottom(4);
            menu_box.set_margin_start(4);
            menu_box.set_margin_end(4);

            // 0. Telefonat abgeben / umleiten (Direkt-Transfer ohne Modal)
            let transfer_menu_btn = gtk::Button::new();
            transfer_menu_btn.add_css_class("flat");
            transfer_menu_btn.add_css_class("transfer-context-btn");
            let trans_btn_box = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            let trans_icon = gtk::Image::from_icon_name("mail-forward-symbolic");
            trans_icon.set_pixel_size(14);
            let trans_lbl = gtk::Label::new(Some(&crate::i18n::tr("Transfer Call")));
            trans_lbl.add_css_class("caption");
            trans_lbl.add_css_class("heading");
            trans_btn_box.append(&trans_icon);
            trans_btn_box.append(&trans_lbl);
            transfer_menu_btn.set_child(Some(&trans_btn_box));
            transfer_menu_btn.set_tooltip_text(Some(&crate::i18n::trf(
                "Transfer active or ringing call directly to {name} ({number})",
                &[("name", &c.name), ("number", &dial_target)],
            )));

            let pop_trans = popover.clone();
            let on_trans_m = on_transfer_pop.clone();
            let c_name_trans = c.name.clone();
            let c_ext_trans = dial_target.clone();
            transfer_menu_btn.connect_clicked(move |_| {
                pop_trans.popdown();
                on_trans_m(c_name_trans.clone(), c_ext_trans.clone());
            });
            menu_box.append(&transfer_menu_btn);

            // 1. Favorit toggeln (Kompakter Stern Icon-Button)
            let fav_btn = gtk::Button::from_icon_name(if is_fav { "starred-symbolic" } else { "non-starred-symbolic" });
            fav_btn.add_css_class("flat");
            fav_btn.add_css_class("circular");
            if is_fav {
                fav_btn.add_css_class("favorite-star-gold");
            }
            let fav_tip = if is_fav {
                crate::i18n::tr("Remove from favorites")
            } else {
                crate::i18n::tr("Add to favorites")
            };
            fav_btn.set_tooltip_text(Some(&fav_tip));

            let pop_fav = popover.clone();
            let sd_fav = speed_dials_pop.clone();
            let toast_fav = toast_pop.clone();
            let on_fav_cb = on_fav_toggle_pop.clone();
            let pop_self = pop_reload_for_row.clone();
            let c_name_fav = c.name.clone();
            let c_ext_fav = dial_target.clone();

            fav_btn.connect_clicked(move |_| {
                pop_fav.popdown();
                if is_fav {
                    sd_fav.borrow_mut().retain(|sd| {
                        let num_match = !sd.number.is_empty() && (sd.number == c_ext_fav);
                        let name_match = !sd.name.is_empty() && (sd.name == c_name_fav);
                        !(num_match || name_match)
                    });
                    let toast = adw::Toast::new(&crate::i18n::trf("{name} removed from favorites", &[("name", &c_name_fav)]));
                    toast.set_timeout(2);
                    toast_fav.add_toast(toast);
                } else {
                    sd_fav.borrow_mut().push(crate::models::SpeedDialEntry {
                        id: chrono::Local::now().timestamp_millis(),
                        name: c_name_fav.clone(),
                        number: c_ext_fav.clone(),
                        state: 0,
                        entity_id: 0,
                    });
                    let toast = adw::Toast::new(&crate::i18n::trf("{name} added to favorites", &[("name", &c_name_fav)]));
                    toast.set_timeout(2);
                    toast_fav.add_toast(toast);
                }
                if let Some(r) = pop_self.borrow().as_ref() {
                    r();
                }
                on_fav_cb();
            });
            menu_box.append(&fav_btn);

            // 2. Chat (internal colleagues)
            if c.is_internal() {
                let chat_menu_btn = gtk::Button::from_icon_name("chat-message-new-symbolic");
                chat_menu_btn.add_css_class("flat");
                chat_menu_btn.add_css_class("circular");
                chat_menu_btn.set_tooltip_text(Some(&crate::i18n::trf("Send instant message to {name}", &[("name", &c.name)])));

                let pop_chat = popover.clone();
                let on_chat_m = on_chat_pop.clone();
                let c_copy = c.clone();
                chat_menu_btn.connect_clicked(move |_| {
                    pop_chat.popdown();
                    on_chat_m(c_copy.clone());
                });
                menu_box.append(&chat_menu_btn);
            }

            // 3. Copy number
            let copy_menu_btn = gtk::Button::from_icon_name("edit-copy-symbolic");
            copy_menu_btn.add_css_class("flat");
            copy_menu_btn.add_css_class("circular");
            copy_menu_btn.set_tooltip_text(Some(&crate::i18n::tr("Copy number")));

            let pop_copy = popover.clone();
            let on_sel_m = on_sel_num_pop.clone();
            let num_copy = dial_target.clone();
            copy_menu_btn.connect_clicked(move |_| {
                pop_copy.popdown();
                on_sel_m(num_copy.clone());
            });
            menu_box.append(&copy_menu_btn);

            popover.set_child(Some(&menu_box));

            let pop_click = popover.clone();
            let right_click = gtk::GestureClick::new();
            right_click.set_button(3); // Sekundäre Maustaste = Rechtsklick
            right_click.connect_pressed(move |gesture, _, _, _| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                pop_click.popup();
            });
            row.add_controller(right_click);

            lb_pop.append(&row);

            row_items_pop.borrow_mut().push(ContactRowItem {
                id: c.id.clone(),
                extension: c.extension.clone(),
                name_lower: c.name.to_lowercase(),
                ext_lower: dial_target.to_lowercase(),
                dept_lower: c.department.to_lowercase(),
                dot,
                row,
                is_fav,
            });
            count += 1;
        }
        empty_contacts_pop.set_visible(count == 0);
        lb_pop.set_visible(count > 0);
    };

    *pop_reload_holder.borrow_mut() = Some(Rc::new(populate_rows.clone()));
    populate_rows();

    let cur_query = Rc::new(RefCell::new(String::new()));
    let row_items_filter = row_items.clone();
    let cur_query_filter = cur_query.clone();

    list_box.set_filter_func(move |list_row| -> bool {
        let idx = list_row.index();
        if idx < 0 {
            return true;
        }
        let items = row_items_filter.borrow();
        if let Some(item) = items.get(idx as usize) {
            let q = cur_query_filter.borrow();
            if !q.is_empty() {
                return item.name_lower.contains(&*q)
                    || item.ext_lower.contains(&*q)
                    || item.dept_lower.contains(&*q);
            }
            true
        } else {
            true
        }
    });

    let lb_filter = list_box.clone();
    let cq_filter = cur_query.clone();
    let debounce_source: Rc<RefCell<Option<gtk::glib::SourceId>>> = Rc::new(RefCell::new(None));
    let db_ref = debounce_source.clone();

    search_entry.connect_search_changed(move |entry| {
        if let Some(src) = db_ref.borrow_mut().take() {
            src.remove();
        }
        let text = entry.text().trim().to_lowercase();
        let lb = lb_filter.clone();
        let cq = cq_filter.clone();
        let db = db_ref.clone();

        let src = gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(30), move || {
            db.borrow_mut().take();
            *cq.borrow_mut() = text;
            lb.invalidate_filter();
        });
        *db_ref.borrow_mut() = Some(src);
    });

    let list_stack_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list_stack_box.append(&list_box);
    list_stack_box.append(&empty_contacts_status);

    let scrolled = gtk::ScrolledWindow::builder()
        .child(&list_stack_box)
        .vexpand(true)
        .hexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(false)
        .min_content_width(200)
        .build();

    container.append(&scrolled);

    let lb_sm = list_box.clone();
    let cq_sm = cur_query.clone();
    let se_sm = search_entry.clone();
    search_bar.connect_search_mode_enabled_notify(move |sb| {
        let is_search = sb.is_search_mode();
        if !is_search {
            se_sm.set_text("");
            *cq_sm.borrow_mut() = String::new();
        }
        lb_sm.invalidate_filter();
    });

    // Pfeiltaste nach unten (Down Arrow): Sofort in das oberste sichtbare Suchergebnis springen
    let key_ctrl = gtk::EventControllerKey::new();
    let lb_key = list_box.clone();
    key_ctrl.connect_key_pressed(move |_, keyval, _, _| {
        if keyval == gtk::gdk::Key::Down {
            let mut curr = lb_key.first_child();
            while let Some(widget) = curr {
                if widget.is_visible() {
                    widget.grab_focus();
                    return gtk::glib::Propagation::Stop;
                }
                curr = widget.next_sibling();
            }
        }
        gtk::glib::Propagation::Proceed
    });
    search_entry.add_controller(key_ctrl);

    let refresh_presence: Rc<dyn Fn()> = {
        let items_ref = row_items.clone();
        let contacts_ref = contacts.clone();
        Rc::new(move || {
            let cs = contacts_ref.borrow();
            for item in items_ref.borrow().iter() {
                if let Some(c) = cs.iter().find(|c| {
                    (!item.extension.is_empty() && c.extension == item.extension) || c.id == item.id
                }) {
                    crate::ui::update_presence_dot(&item.dot, c.has_pbx_presence().then_some(c.presence));
                    let status_color = match c.presence {
                        crate::models::PresenceState::Available => "#2ec27e",
                        crate::models::PresenceState::Busy => "#e01b24",
                        crate::models::PresenceState::Away => "#f6d32d",
                        crate::models::PresenceState::LoggedOut => "#77767b",
                        crate::models::PresenceState::Ringing => "#ff7800",
                    };
                    let fav_text = if item.is_fav {
                        format!(" • {}", crate::i18n::tr("Favorite (Speed Dial)"))
                    } else {
                        String::new()
                    };
                    let tooltip_markup = if c.has_pbx_presence() {
                        format!(
                            "<b>{}</b> • {}{}\n\
                             {}: {} • {}: {}\n\
                             {}: <span foreground=\"{}\">{}</span>\n\
                             <span size=\"small\" alpha=\"75%\">{}</span>",
                            gtk::glib::markup_escape_text(&c.name),
                            gtk::glib::markup_escape_text(&c.department),
                            fav_text,
                            crate::i18n::tr("Extension"),
                            gtk::glib::markup_escape_text(&c.extension),
                            crate::i18n::tr("Direct"),
                            gtk::glib::markup_escape_text(&c.phone),
                            crate::i18n::tr("Status"),
                            status_color,
                            c.presence.label(),
                            crate::i18n::tr("Drag line onto a contact or right-click → Transfer Call"),
                        )
                    } else {
                        format!(
                            "<b>{}</b> • {}{}\n{}: {}",
                            gtk::glib::markup_escape_text(&c.name),
                            gtk::glib::markup_escape_text(&c.department),
                            fav_text,
                            crate::i18n::tr("Number"),
                            gtk::glib::markup_escape_text(&c.phone),
                        )
                    };
                    item.row.set_tooltip_markup(Some(&tooltip_markup));
                }
            }
        })
    };

    let pop_reload = populate_rows.clone();
    let se_reload = search_entry.clone();
    let cq_reload = cur_query.clone();
    let lb_reload = list_box.clone();
    let reload = Rc::new(move || {
        pop_reload();
        *cq_reload.borrow_mut() = se_reload.text().trim().to_lowercase();
        lb_reload.invalidate_filter();
    });

    ContactsViewComponents {
        container,
        reload,
        refresh_presence,
        search_bar,
    }
}

