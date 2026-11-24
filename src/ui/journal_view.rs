use gtk4 as gtk;
use gtk::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use crate::models::{CallDirection, CallJournalEntry};

#[derive(Clone)]
pub struct JournalViewComponents {
    pub container: gtk::Box,
    pub reload: Rc<dyn Fn()>,
    pub refresh_presence: Rc<dyn Fn()>,
    pub mark_all_seen: Rc<dyn Fn()>,
    pub search_bar: gtk::SearchBar,
}

pub fn create_journal_view<FCall, FTransfer, FSelectNum, FChat, FFavToggle, FSeen>(
    entries: Rc<RefCell<Vec<CallJournalEntry>>>,
    contacts: Rc<RefCell<Vec<crate::models::Contact>>>,
    speed_dials: Rc<RefCell<Vec<crate::models::SpeedDialEntry>>>,
    toast_overlay: adw::ToastOverlay,
    initial_seen_ids: Vec<String>,
    on_trigger_call: FCall,
    on_transfer_call: FTransfer,
    on_select_number: FSelectNum,
    on_open_chat: FChat,
    on_fav_toggle: FFavToggle,
    on_seen_changed: FSeen,
) -> JournalViewComponents
where
    FCall: Fn(String, String) + 'static + Clone,
    FTransfer: Fn(String, String) + 'static + Clone,
    FSelectNum: Fn(String) + 'static + Clone,
    FChat: Fn(String, String) + 'static + Clone,
    FFavToggle: Fn() + 'static + Clone,
    FSeen: Fn(Vec<String>) + 'static + Clone,
{
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);

    // Track which missed calls have been seen by the user (bold when unseen, persisted in data.json)
    let initial_set: HashSet<String> = initial_seen_ids.into_iter().collect();
    let seen_missed_ids: Rc<RefCell<HashSet<String>>> = Rc::new(RefCell::new(initial_set));
    let cur_search_query: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));

    // Sliding Search Bar für Rufjournal (GNOME HIG Standard)
    let search_bar = gtk::SearchBar::new();
    let search_entry = gtk::SearchEntry::new();
    search_entry.set_placeholder_text(Some(&crate::i18n::tr("Search call journal")));
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

    // 1. Natives Libadwaita AdwToggleGroup: [ Alle | Verpasst | Eingehend | Ausgehend ]
    let filter_center_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    filter_center_box.set_halign(gtk::Align::Center);
    filter_center_box.set_margin_top(4);
    filter_center_box.set_margin_bottom(8);
    filter_center_box.set_margin_start(8);
    filter_center_box.set_margin_end(8);

    let filter_group = adw::ToggleGroup::builder()
        .homogeneous(false)
        .can_shrink(true)
        .build();
    filter_group.add(adw::Toggle::builder().name("all").label(&crate::i18n::tr("All")).build());
    filter_group.add(adw::Toggle::builder().name("missed").label(&crate::i18n::tr("Missed")).build());
    filter_group.add(adw::Toggle::builder().name("incoming").label(&crate::i18n::tr("Incoming")).build());
    filter_group.add(adw::Toggle::builder().name("outgoing").label(&crate::i18n::tr("Outgoing")).build());
    filter_group.set_active_name(Some("all"));
    filter_center_box.append(&filter_group);

    container.append(&filter_center_box);

    // 3. Journal Direct List in a ScrolledWindow (Edge-to-edge, minimal & kompakt)
    let list_box = gtk::ListBox::new();
    list_box.add_css_class("direct-list");
    list_box.set_selection_mode(gtk::SelectionMode::None);
    list_box.set_valign(gtk::Align::Start);
    list_box.set_show_separators(false);

    let list_box_ref = list_box.clone();
    let entries_for_filter = entries.clone();
    let contacts_for_filter = contacts.clone();
    let speed_dials_for_filter = speed_dials.clone();
    let toast_overlay_for_filter = toast_overlay.clone();
    let on_fav_toggle_action = on_fav_toggle.clone();
    let seen_missed_ids_for_filter = seen_missed_ids.clone();
    let cur_query_for_filter = cur_search_query.clone();
    let on_call_action = on_trigger_call.clone();
    let on_transfer_action = on_transfer_call.clone();
    let on_select_num_action = on_select_number.clone();
    let on_chat_action = on_open_chat.clone();
    let cur_filter = Rc::new(RefCell::new(None::<CallDirection>));
    let rl_holder: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let rl_holder_for_filter = rl_holder.clone();
    let on_seen_for_list = on_seen_changed.clone();

    struct JournalPresenceBinding {
        number: String,
        name: String,
        dot: gtk::Box,
    }
    let presence_bindings: Rc<RefCell<Vec<JournalPresenceBinding>>> = Rc::new(RefCell::new(Vec::new()));
    let presence_bindings_for_list = presence_bindings.clone();

    let empty_status = adw::StatusPage::builder()
        .icon_name("document-open-recent-symbolic")
        .title(&crate::i18n::tr("No Calls"))
        .description(&crate::i18n::tr("Incoming, outgoing, and missed calls will appear here."))
        .vexpand(true)
        .visible(false)
        .build();
    let empty_status_ref = empty_status.clone();

    // Helper to rebuild list rows based on filter
    let rebuild_list = move |filter_type: Option<CallDirection>| {
        // Clear existing rows & live presence bindings
        while let Some(child) = list_box_ref.first_child() {
            list_box_ref.remove(&child);
        }
        presence_bindings_for_list.borrow_mut().clear();

        // Gemeinsame horizontale SizeGroups für exakt fluchtende Spalten über alle Zeilen
        let prefix_size_group = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
        let meta_size_group = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
        let mut rendered_count = 0usize;

        let q = cur_query_for_filter.borrow().to_lowercase();
        let is_searching = !q.is_empty();

        for entry in entries_for_filter.borrow().iter() {
            if !is_searching {
                if let Some(dir) = filter_type {
                    if entry.direction != dir {
                        continue;
                    }
                }
            } else {
                let name_match = entry.name.to_lowercase().contains(&q);
                let num_match = entry.number.to_lowercase().contains(&q);
                let target_match = entry.called_target.to_lowercase().contains(&q);
                let connected_match = entry.connected_with.to_lowercase().contains(&q);
                if !name_match && !num_match && !target_match && !connected_match {
                    continue;
                }
            }

            // Bekannten internen Kontakt suchen (für Live-Status)
            let entry_fmt_name = crate::ui::sidebar::format_hotkey_display_name(&entry.name);
            let matching_contact = contacts_for_filter.borrow().iter().find(|c| {
                (!c.extension.is_empty() && c.extension == entry.number)
                    || (!c.phone.is_empty() && c.phone == entry.number)
                    || (!entry.name.is_empty()
                        && (c.name == entry.name
                            || crate::ui::sidebar::format_hotkey_display_name(&c.name) == entry_fmt_name))
            }).cloned();

            // Favoriten-Status ermitteln (in SpeedDials vorhanden?)
            let is_fav = speed_dials_for_filter.borrow().iter().any(|sd| {
                (!sd.number.is_empty() && (sd.number == entry.number || matching_contact.as_ref().map(|c| c.extension == sd.number || c.phone == sd.number).unwrap_or(false)))
                || (!sd.name.is_empty() && (sd.name == entry.name || matching_contact.as_ref().map(|c| c.name == sd.name).unwrap_or(false)))
            });

            let is_dark = adw::StyleManager::default().is_dark();
            let star_color = if is_dark { "#f6d32d" } else { "#e5a50a" };
            let star_span = format!(" <span foreground=\"{}\">★</span>", star_color);

            let row = adw::ActionRow::new();
            row.set_activatable(true);
            row.set_title_lines(1);
            row.set_subtitle_lines(1);

            // 1. Title & Subtitle (keine doppelten Rufnummern untereinander)
            let resolved_name = if !entry.name.is_empty() && entry.name != entry.number {
                Some(entry.name.as_str())
            } else {
                matching_contact
                    .as_ref()
                    .filter(|c| !c.name.is_empty() && c.name != entry.number)
                    .map(|c| c.name.as_str())
            };

            let raw_title = resolved_name.unwrap_or(&entry.number);
            let escaped_title = gtk::glib::markup_escape_text(raw_title);

            let base_sub = if resolved_name.is_some() {
                entry.number.clone()
            } else {
                crate::models::classify_phone_number_subtitle(&entry.number, &entry.called_target)
            };

            let raw_subtitle = if entry.connected_with.contains("[Herangeholt]") {
                let who = entry.connected_with.replace(" [Herangeholt]", "");
                crate::i18n::trf("{base} • Picked up by: {who}", &[("base", &base_sub), ("who", &who)])
            } else if entry.connected_with.contains("[Vertretung]") {
                let who = entry.connected_with.replace(" [Vertretung]", "");
                crate::i18n::trf("{base} • Answered by: {who}", &[("base", &base_sub), ("who", &who)])
            } else {
                base_sub
            };
            let escaped_subtitle = gtk::glib::markup_escape_text(&raw_subtitle);

            let formatted_title = if is_fav {
                format!("{}{}", escaped_title, star_span)
            } else {
                escaped_title.to_string()
            };

            row.set_title(&formatted_title);
            row.set_subtitle(&escaped_subtitle);

            let num_click = entry.number.clone();
            let on_sel_num = on_select_num_action.clone();
            let s_seen = seen_missed_ids_for_filter.clone();
            let e_id = entry.id.clone();
            let on_seen_click = on_seen_for_list.clone();
            row.connect_activated(move |_| {
                if s_seen.borrow_mut().insert(e_id.clone()) {
                    on_seen_click(s_seen.borrow().iter().cloned().collect());
                }
                on_sel_num(num_click.clone());
            });

            // 2. Extra-Informationen per Hover: Minimalistisch, prägnant und professionell
            let fav_text = if is_fav {
                format!(" • {}", crate::i18n::tr("Favorite (Speed Dial)"))
            } else {
                String::new()
            };
            let header = if entry.name.is_empty() || entry.name == entry.number {
                format!("<b>{}</b>{}", gtk::glib::markup_escape_text(&entry.number), fav_text)
            } else {
                format!("<b>{}</b> ({}){}", gtk::glib::markup_escape_text(&entry.name), gtk::glib::markup_escape_text(&entry.number), fav_text)
            };

            let routing = if entry.connected_with.contains("[Herangeholt]") {
                let who = entry.connected_with.replace(" [Herangeholt]", "");
                crate::i18n::trf("Target: {target} • Picked up by {who}", &[("target", &entry.called_target), ("who", &who)])
            } else if entry.connected_with.contains("[Vertretung]") {
                let who = entry.connected_with.replace(" [Vertretung]", "");
                crate::i18n::trf("Target: {target} • Answered by {who}", &[("target", &entry.called_target), ("who", &who)])
            } else if entry.direction == CallDirection::Missed {
                crate::i18n::trf("Target: {target} • Not answered", &[("target", &entry.called_target)])
            } else {
                crate::i18n::trf("Target: {target}", &[("target", &entry.called_target)])
            };

            let time_and_status = if entry.direction == CallDirection::Missed {
                format!("{} • <span foreground=\"#e01b24\">{}</span>", entry.formatted_full(), crate::i18n::tr("Missed"))
            } else {
                format!("{} • {}: {}", entry.formatted_full(), crate::i18n::tr("Duration"), entry.duration)
            };

            let status_info = if let Some(ref c) = matching_contact.as_ref().filter(|c| c.has_pbx_presence()) {
                let color = match c.presence {
                    crate::models::PresenceState::Available => "#2ec27e",
                    crate::models::PresenceState::Busy => "#e01b24",
                    crate::models::PresenceState::Away => "#f6d32d",
                    crate::models::PresenceState::LoggedOut => "#77767b",
                    crate::models::PresenceState::Ringing => "#ff7800",
                };
                format!("\n{}: <span foreground=\"{}\">{}</span>", crate::i18n::tr("Status"), color, c.presence.label())
            } else {
                String::new()
            };

            let tooltip_markup = format!(
                "{}\n{}\n{}{}",
                header,
                gtk::glib::markup_escape_text(&routing),
                time_and_status,
                status_info
            );
            row.set_tooltip_markup(Some(&tooltip_markup));

            // 3. Direction icon + Profilbild / Initialen-Avatar mit integriertem Status-Ring
            let prefix_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            prefix_box.set_valign(gtk::Align::Center);

            let (icon_name, icon_class) = match entry.direction {
                CallDirection::Incoming => ("call-incoming-symbolic", "icon-call-incoming"),
                CallDirection::Outgoing => ("call-outgoing-symbolic", "icon-call-outgoing"),
                CallDirection::Missed => ("call-missed-symbolic", "icon-call-missed"),
            };
            let dir_icon = gtk::Image::from_icon_name(icon_name);
            dir_icon.add_css_class(icon_class);
            dir_icon.set_pixel_size(18);
            prefix_box.append(&dir_icon);

            let avatar_name = if !entry.name.is_empty() && entry.name != entry.number {
                &entry.name
            } else if let Some(ref c) = matching_contact {
                &c.name
            } else {
                &entry.number
            };

            let presence = matching_contact
                .as_ref()
                .filter(|c| c.has_pbx_presence())
                .map(|c| c.presence);
            let (avatar, dot) = crate::ui::create_presence_avatar_with_dot(avatar_name, presence, 32);
            presence_bindings_for_list.borrow_mut().push(JournalPresenceBinding {
                number: entry.number.clone(),
                name: entry.name.clone(),
                dot,
            });
            prefix_box.append(&avatar);
            prefix_size_group.add_widget(&prefix_box);

            row.add_prefix(&prefix_box);

            // 4. Suffix: Wann & Wie lange + Callback-Button (kompakt & exakt fluchtend)
            let suffix_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            suffix_box.set_valign(gtk::Align::Center);

            let meta_col = gtk::Box::new(gtk::Orientation::Vertical, 2);
            meta_col.set_valign(gtk::Align::Center);
            meta_col.set_halign(gtk::Align::End);

            // Wann: Smarte Zeitangabe (Tabellenziffern über .numeric)
            let time_lbl = gtk::Label::new(Some(&entry.formatted_time()));
            time_lbl.add_css_class("caption");
            time_lbl.add_css_class("dim-label");
            time_lbl.add_css_class("numeric");
            time_lbl.set_halign(gtk::Align::End);
            meta_col.append(&time_lbl);

            // Wie lange: Echtes SVG Icon (preferences-system-time-symbolic) + Dauer-Label (nur bei geführten Anrufen)
            if entry.direction != CallDirection::Missed {
                let dur_box = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                dur_box.set_halign(gtk::Align::End);
                dur_box.set_valign(gtk::Align::Center);

                let clock_icon = gtk::Image::from_icon_name("preferences-system-time-symbolic");
                clock_icon.set_pixel_size(14);
                clock_icon.add_css_class("dim-label");
                dur_box.append(&clock_icon);

                let dur_lbl = gtk::Label::new(Some(&entry.duration));
                dur_lbl.add_css_class("caption");
                dur_lbl.add_css_class("dim-label");
                dur_lbl.add_css_class("numeric");
                dur_box.append(&dur_lbl);

                meta_col.append(&dur_box);
            }

            meta_size_group.add_widget(&meta_col);
            suffix_box.append(&meta_col);

            // Callback Button
            let cb_btn = gtk::Button::from_icon_name("call-start-symbolic");
            cb_btn.add_css_class("flat");
            cb_btn.add_css_class("circular");
            cb_btn.add_css_class("journal-call-btn");
            cb_btn.set_tooltip_text(Some(&crate::i18n::trf("Call back: {name}", &[("name", &entry.name)])));
            cb_btn.update_property(&[gtk::accessible::Property::Label(&crate::i18n::trf("Call back {name}", &[("name", &entry.name)]))]);

            let name = entry.name.clone();
            let num = entry.number.clone();
            let trigger = on_call_action.clone();
            cb_btn.connect_clicked(move |_| {
                trigger(name.clone(), num.clone());
            });

            suffix_box.append(&cb_btn);
            row.add_suffix(&suffix_box);

            // 5. Kontextmenü per Rechtsklick (sauber an suffix_box angehängt statt direkt an AdwActionRow)
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

            let target_display_name = if !entry.name.is_empty() && entry.name != entry.number {
                entry.name.clone()
            } else if let Some(ref c) = matching_contact {
                c.name.clone()
            } else {
                entry.number.clone()
            };

            let target_number = if let Some(ref c) = matching_contact {
                if !c.extension.is_empty() {
                    c.extension.clone()
                } else {
                    entry.number.clone()
                }
            } else {
                entry.number.clone()
            };

            // Drag & Drop Ziel: Leitung / Gespräch direkt auf diesen Eintrag ziehen zum Weiterleiten
            crate::ui::attach_call_drop_target(
                &row,
                target_display_name.clone(),
                target_number.clone(),
                on_transfer_action.clone(),
            );

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
                &[("name", &target_display_name), ("number", &target_number)],
            )));

            let pop_trans = popover.clone();
            let on_trans_m = on_transfer_action.clone();
            let j_name_trans = target_display_name.clone();
            let j_num_trans = target_number.clone();
            transfer_menu_btn.connect_clicked(move |_| {
                pop_trans.popdown();
                on_trans_m(j_name_trans.clone(), j_num_trans.clone());
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
            let sd_fav = speed_dials_for_filter.clone();
            let toast_fav = toast_overlay_for_filter.clone();
            let on_fav_cb = on_fav_toggle_action.clone();
            let rl_cb = rl_holder_for_filter.clone();
            let e_num = entry.number.clone();
            let e_name = entry.name.clone();
            let m_contact = matching_contact.clone();
            let disp_name = target_display_name.clone();
            let tgt_num = target_number.clone();

            fav_btn.connect_clicked(move |_| {
                pop_fav.popdown();
                if is_fav {
                    sd_fav.borrow_mut().retain(|sd| {
                        let num_match = !sd.number.is_empty() && (sd.number == e_num || m_contact.as_ref().map(|c| c.extension == sd.number || c.phone == sd.number).unwrap_or(false));
                        let name_match = !sd.name.is_empty() && (sd.name == e_name || m_contact.as_ref().map(|c| c.name == sd.name).unwrap_or(false));
                        !(num_match || name_match)
                    });
                    let toast = adw::Toast::new(&crate::i18n::trf("{name} removed from favorites", &[("name", &disp_name)]));
                    toast.set_timeout(2);
                    toast_fav.add_toast(toast);
                } else {
                    sd_fav.borrow_mut().push(crate::models::SpeedDialEntry {
                        id: chrono::Local::now().timestamp_millis(),
                        name: disp_name.clone(),
                        number: tgt_num.clone(),
                        state: 0,
                        entity_id: 0,
                    });
                    let toast = adw::Toast::new(&crate::i18n::trf("{name} added to favorites", &[("name", &disp_name)]));
                    toast.set_timeout(2);
                    toast_fav.add_toast(toast);
                }
                if let Some(r) = rl_cb.borrow().as_ref() {
                    r();
                }
                on_fav_cb();
            });
            menu_box.append(&fav_btn);

            // 2. Chat (internal colleagues)
            let is_swyx_internal = crate::models::is_internal_extension(&entry.number)
                || matching_contact.as_ref().map(|c| c.is_internal()).unwrap_or(false);

            if is_swyx_internal {
                let chat_menu_btn = gtk::Button::from_icon_name("chat-message-new-symbolic");
                chat_menu_btn.add_css_class("flat");
                chat_menu_btn.add_css_class("circular");
                chat_menu_btn.set_tooltip_text(Some(&crate::i18n::trf("Send instant message to {name}", &[("name", &target_display_name)])));

                let pop_chat = popover.clone();
                let on_chat_m = on_chat_action.clone();
                let name_c = entry.name.clone();
                let num_c = entry.number.clone();
                chat_menu_btn.connect_clicked(move |_| {
                    pop_chat.popdown();
                    on_chat_m(name_c.clone(), num_c.clone());
                });
                menu_box.append(&chat_menu_btn);
            }

            // 3. Copy number
            let copy_menu_btn = gtk::Button::from_icon_name("edit-copy-symbolic");
            copy_menu_btn.add_css_class("flat");
            copy_menu_btn.add_css_class("circular");
            copy_menu_btn.set_tooltip_text(Some(&crate::i18n::tr("Copy number")));

            let pop_copy = popover.clone();
            let on_sel_m = on_select_num_action.clone();
            let num_copy = entry.number.clone();
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
            list_box_ref.append(&row);
            rendered_count += 1;
        }

        if rendered_count == 0 {
            if is_searching {
                empty_status_ref.set_icon_name(Some("edit-find-symbolic"));
                empty_status_ref.set_title(&crate::i18n::tr("No Results"));
                empty_status_ref.set_description(Some(&crate::i18n::tr("No matching entries found in the call journal.")));
            } else {
                empty_status_ref.set_icon_name(Some("document-open-recent-symbolic"));
                empty_status_ref.set_title(&crate::i18n::tr("No Calls"));
                empty_status_ref.set_description(Some(&crate::i18n::tr("Incoming, outgoing, and missed calls will appear here.")));
            }
            empty_status_ref.set_visible(true);
            list_box_ref.set_visible(false);
        } else {
            empty_status_ref.set_visible(false);
            list_box_ref.set_visible(true);
        }
    };

    // Initial build
    rebuild_list(None);

    let list_stack_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list_stack_box.append(&list_box);
    list_stack_box.append(&empty_status);

    let scrolled = gtk::ScrolledWindow::builder()
        .child(&list_stack_box)
        .vexpand(true)
        .hexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(false)
        .min_content_width(200)
        .build();

    container.append(&scrolled);

    let cf_tg = cur_filter.clone();
    let rl_tg = rebuild_list.clone();
    filter_group.connect_active_name_notify(move |group| {
        let next = match group.active_name().as_deref() {
            Some("missed") => Some(CallDirection::Missed),
            Some("incoming") => Some(CallDirection::Incoming),
            Some("outgoing") => Some(CallDirection::Outgoing),
            _ => None,
        };
        *cf_tg.borrow_mut() = next;
        rl_tg(next);
    });

    let rl_reload = rebuild_list.clone();
    let cf_reload = cur_filter.clone();
    let reload = Rc::new(move || {
        rl_reload(*cf_reload.borrow());
    });
    *rl_holder.borrow_mut() = Some(reload.clone());

    let entries_seen = entries.clone();
    let seen_ids_all = seen_missed_ids.clone();
    let on_seen_all = on_seen_changed.clone();

    let mark_all_seen = Rc::new(move || {
        let mut modified = false;
        let mut seen = seen_ids_all.borrow_mut();
        for e in entries_seen.borrow().iter() {
            if e.direction == CallDirection::Missed {
                if seen.insert(e.id.clone()) {
                    modified = true;
                }
            }
        }
        let current_seen: Vec<String> = seen.iter().cloned().collect();
        drop(seen);
        if modified {
            on_seen_all(current_seen);
        }
    });

    let rl_search = rebuild_list.clone();
    let cf_search = cur_filter.clone();
    let cq_search = cur_search_query.clone();
    let db_ref: Rc<RefCell<Option<gtk::glib::SourceId>>> = Rc::new(RefCell::new(None));
    let db_clone = db_ref.clone();

    search_entry.connect_search_changed(move |entry| {
        if let Some(src) = db_clone.borrow_mut().take() {
            src.remove();
        }
        let text = entry.text().trim().to_lowercase();
        let rl = rl_search.clone();
        let cf = cf_search.clone();
        let cq = cq_search.clone();
        let db = db_clone.clone();

        let src = gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(30), move || {
            db.borrow_mut().take();
            *cq.borrow_mut() = text;
            rl(*cf.borrow());
        });
        *db_clone.borrow_mut() = Some(src);
    });

    let se_sm = search_entry.clone();
    let cq_sm = cur_search_query.clone();
    let rl_sm = rebuild_list.clone();
    let cf_sm = cur_filter.clone();
    let fc_sm = filter_center_box.clone();
    search_bar.connect_search_mode_enabled_notify(move |sb| {
        let is_search = sb.is_search_mode();
        fc_sm.set_visible(!is_search);
        if !is_search {
            se_sm.set_text("");
            *cq_sm.borrow_mut() = String::new();
            rl_sm(*cf_sm.borrow());
        }
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
        let bindings_ref = presence_bindings.clone();
        let contacts_ref = contacts.clone();
        Rc::new(move || {
            let cs = contacts_ref.borrow();
            for b in bindings_ref.borrow().iter() {
                let b_fmt_name = crate::ui::sidebar::format_hotkey_display_name(&b.name);
                let presence = cs
                    .iter()
                    .find(|c| {
                        (!c.extension.is_empty() && c.extension == b.number)
                            || (!c.phone.is_empty() && c.phone == b.number)
                            || (!b.name.is_empty()
                                && (c.name == b.name
                                    || crate::ui::sidebar::format_hotkey_display_name(&c.name) == b_fmt_name))
                    })
                    .filter(|c| c.has_pbx_presence())
                    .map(|c| c.presence);
                crate::ui::update_presence_dot(&b.dot, presence);
            }
        })
    };

    JournalViewComponents {
        container,
        reload,
        refresh_presence,
        mark_all_seen,
        search_bar,
    }
}
