use gtk4 as gtk;
use gtk::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::engine_bridge::EngineBridge;
use crate::swyx_bridge::{SwyxBridge, SwyxDataStore};
use crate::ui::clean_name_for_avatar;

pub fn open_settings_dialog(
    parent: &adw::ApplicationWindow,
    bridge: Rc<SwyxBridge>,
    engine_bridge: Rc<EngineBridge>,
    store_ref: Rc<RefCell<SwyxDataStore>>,
    toast_overlay: adw::ToastOverlay,
    user_label: gtk::Label,
    user_avatar: adw::Avatar,
    on_avatar_updated: Rc<dyn Fn()>,
    on_gnome_contacts_toggled: Rc<dyn Fn(bool)>,
    on_reconnect: Rc<dyn Fn()>,
) {
    use crate::i18n::{tr, trf};
    let dialog = adw::PreferencesDialog::builder()
        .title(&tr("Preferences"))
        .build();

    let page = adw::PreferencesPage::new();
    let current_store = store_ref.borrow().clone();

    // 0. Profile Picture (Swyx server avatar)
    let profile_group = adw::PreferencesGroup::builder()
        .title(&tr("Profile"))
        .build();

    let initial_short = if !current_store.user_name.is_empty() {
        clean_name_for_avatar(&current_store.user_name)
    } else {
        "Deskphone".to_string()
    };

    let preview_avatar = adw::Avatar::builder()
        .size(48)
        .text(&initial_short)
        .show_initials(true)
        .build();
    preview_avatar.set_valign(gtk::Align::Center);
    preview_avatar.set_margin_top(6);
    preview_avatar.set_margin_bottom(6);
    let has_initial_avatar = if let Some(own_tex) = crate::ui::load_own_avatar_texture() {
        preview_avatar.set_custom_image(Some(&own_tex));
        true
    } else if let Some(tex) = crate::ui::get_contact_photo(&current_store.user_name) {
        preview_avatar.set_custom_image(Some(&tex));
        true
    } else {
        false
    };

    let row_avatar = adw::ActionRow::builder()
        .title(&tr("Profile Picture"))
        .subtitle(&if has_initial_avatar {
            tr("Stored on Swyx server")
        } else {
            tr("No profile picture set")
        })
        .build();
    row_avatar.add_prefix(&preview_avatar);

    let upload_spinner = gtk::Spinner::new();
    upload_spinner.set_valign(gtk::Align::Center);
    upload_spinner.set_visible(false);

    let pick_avatar_btn = gtk::Button::with_label(&tr("Change…"));
    pick_avatar_btn.set_valign(gtk::Align::Center);
    pick_avatar_btn.set_tooltip_text(Some(&tr("Choose a new profile picture (JPEG, PNG, or WebP)")));

    let remove_avatar_btn = gtk::Button::from_icon_name("user-trash-symbolic");
    remove_avatar_btn.add_css_class("flat");
    remove_avatar_btn.add_css_class("circular");
    remove_avatar_btn.set_valign(gtk::Align::Center);
    remove_avatar_btn.set_tooltip_text(Some(&tr("Remove profile picture")));
    remove_avatar_btn.set_visible(has_initial_avatar);

    let avatar_suffix_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    avatar_suffix_box.set_valign(gtk::Align::Center);
    avatar_suffix_box.append(&upload_spinner);
    avatar_suffix_box.append(&pick_avatar_btn);
    avatar_suffix_box.append(&remove_avatar_btn);
    row_avatar.add_suffix(&avatar_suffix_box);

    {
        let parent_pick = parent.clone();
        let eb_pick = engine_bridge.clone();
        let store_pick = store_ref.clone();
        let toast_pick = toast_overlay.clone();
        let prev_av = preview_avatar.clone();
        let side_av = user_avatar.clone();
        let spin_pick = upload_spinner.clone();
        let btn_pick = pick_avatar_btn.clone();
        let btn_rem = remove_avatar_btn.clone();
        let row_av_pick = row_avatar.clone();
        let on_av_upd = on_avatar_updated.clone();

        pick_avatar_btn.connect_clicked(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(&tr("Images (PNG, JPEG, WebP, BMP)")));
            filter.add_mime_type("image/png");
            filter.add_mime_type("image/jpeg");
            filter.add_mime_type("image/webp");
            filter.add_mime_type("image/bmp");

            let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);

            let file_dialog = gtk::FileDialog::builder()
                .title(&tr("Choose Profile Picture"))
                .filters(&filters)
                .default_filter(&filter)
                .modal(true)
                .build();

            let eb_open = eb_pick.clone();
            let store_open = store_pick.clone();
            let toast_open = toast_pick.clone();
            let prev_open = prev_av.clone();
            let side_open = side_av.clone();
            let spin_open = spin_pick.clone();
            let btn_open = btn_pick.clone();
            let btn_rem_open = btn_rem.clone();
            let row_open = row_av_pick.clone();
            let on_upd_open = on_av_upd.clone();

            file_dialog.open(Some(&parent_pick), gtk::gio::Cancellable::NONE, move |res| {
                let Ok(file) = res else {
                    return;
                };
                let Some(path) = file.path() else {
                    return;
                };

                let jpeg_bytes = match crate::ui::prepare_avatar_jpeg_bytes(&path) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        let err_s = e.to_string();
                        let toast = adw::Toast::new(&trf("Could not process image: {error}", &[("error", &err_s)]));
                        toast.set_timeout(4);
                        toast_open.add_toast(toast);
                        return;
                    }
                };

                btn_open.set_sensitive(false);
                btn_rem_open.set_sensitive(false);
                spin_open.set_visible(true);
                spin_open.start();
                row_open.set_subtitle(&tr("Uploading to Swyx server…"));

                let own_aliases = {
                    let st = store_open.borrow();
                    let mut v = Vec::new();
                    if !st.user_name.trim().is_empty() {
                        v.push(st.user_name.clone());
                    }
                    if !st.extension.trim().is_empty() {
                        v.push(st.extension.clone());
                    }
                    v
                };

                gtk::glib::MainContext::default().spawn_local(async move {
                    let upload_res = eb_open.upload_own_avatar_bytes(jpeg_bytes.clone()).await;
                    spin_open.stop();
                    spin_open.set_visible(false);
                    btn_open.set_sensitive(true);
                    btn_rem_open.set_sensitive(true);

                    match upload_res {
                        Ok(_file_id) => {
                            if let Some(tex) = crate::ui::store_own_avatar_bytes(&jpeg_bytes, &own_aliases) {
                                prev_open.set_custom_image(Some(&tex));
                                side_open.set_custom_image(Some(&tex));
                            }
                            btn_rem_open.set_visible(true);
                            row_open.set_subtitle(&tr("Stored on Swyx server"));
                            on_upd_open();
                            let toast = adw::Toast::new(&tr("Profile picture updated"));
                            toast.set_timeout(3);
                            toast_open.add_toast(toast);
                        }
                        Err(e) => {
                            row_open.set_subtitle(&tr("Stored on Swyx server"));
                            let err_s = e.to_string();
                            let toast = adw::Toast::new(&trf("Upload failed: {error}", &[("error", &err_s)]));
                            toast.set_timeout(4);
                            toast_open.add_toast(toast);
                        }
                    }
                });
            });
        });
    }

    {
        let eb_del = engine_bridge.clone();
        let store_del = store_ref.clone();
        let toast_del = toast_overlay.clone();
        let prev_del = preview_avatar.clone();
        let side_del = user_avatar.clone();
        let spin_del = upload_spinner.clone();
        let btn_pick_del = pick_avatar_btn.clone();
        let btn_rem_del = remove_avatar_btn.clone();
        let row_del = row_avatar.clone();
        let on_upd_del = on_avatar_updated.clone();

        remove_avatar_btn.connect_clicked(move |_| {
            btn_pick_del.set_sensitive(false);
            btn_rem_del.set_sensitive(false);
            spin_del.set_visible(true);
            spin_del.start();
            row_del.set_subtitle(&tr("Removing profile picture…"));

            let own_aliases = {
                let st = store_del.borrow();
                let mut v = Vec::new();
                if !st.user_name.trim().is_empty() {
                    v.push(st.user_name.clone());
                }
                if !st.extension.trim().is_empty() {
                    v.push(st.extension.clone());
                }
                v
            };

            let eb_run = eb_del.clone();
            let toast_run = toast_del.clone();
            let prev_run = prev_del.clone();
            let side_run = side_del.clone();
            let spin_run = spin_del.clone();
            let btn_pick_run = btn_pick_del.clone();
            let btn_rem_run = btn_rem_del.clone();
            let row_run = row_del.clone();
            let on_upd_run = on_upd_del.clone();

            gtk::glib::MainContext::default().spawn_local(async move {
                let res = eb_run.delete_own_avatar().await;
                spin_run.stop();
                spin_run.set_visible(false);
                btn_pick_run.set_sensitive(true);
                btn_rem_run.set_sensitive(true);

                match res {
                    Ok(()) => {
                        crate::ui::clear_own_avatar_cache(&own_aliases);
                        prev_run.set_custom_image(None::<&gtk::gdk::Texture>);
                        side_run.set_custom_image(None::<&gtk::gdk::Texture>);
                        btn_rem_run.set_visible(false);
                        row_run.set_subtitle(&tr("No profile picture set"));
                        on_upd_run();
                        let toast = adw::Toast::new(&tr("Profile picture removed"));
                        toast.set_timeout(3);
                        toast_run.add_toast(toast);
                    }
                    Err(e) => {
                        row_run.set_subtitle(&tr("Stored on Swyx server"));
                        let err_s = e.to_string();
                        let toast = adw::Toast::new(&trf("Remove failed: {error}", &[("error", &err_s)]));
                        toast.set_timeout(4);
                        toast_run.add_toast(toast);
                    }
                }
            });
        });
    }

    profile_group.add(&row_avatar);
    let initially_swyx = current_store.provider.supports_server_avatar()
        || engine_bridge.resolved_provider().supports_server_avatar();
    profile_group.set_visible(initially_swyx);

    // 1. Account & Server (3 simple fields by default; optional provider/port/realm under Advanced)
    let conn_group = adw::PreferencesGroup::builder()
        .title(&tr("Account & Server"))
        .build();

    let row_server = adw::EntryRow::builder()
        .title(&tr("Server / Hostname"))
        .text(&current_store.server_ip)
        .build();

    let row_user = adw::EntryRow::builder()
        .title(&tr("Username"))
        .text(&current_store.user_name)
        .build();

    let row_password = adw::PasswordEntryRow::builder()
        .title(&tr("Password"))
        .text(&current_store.password)
        .build();

    let prov_auto_lbl = tr("Automatic");
    let prov_sip_lbl = tr("Standard SIP (FRITZ!Box, Sipgate, …)");
    let provider_labels = gtk::StringList::new(&[
        prov_auto_lbl.as_str(),
        prov_sip_lbl.as_str(),
        "SwyxWare",
        "Asterisk / FreePBX",
    ]);
    let row_provider = adw::ComboRow::builder()
        .title(&tr("Phone System"))
        .model(&provider_labels)
        .selected(current_store.provider.index())
        .build();

    let row_sip_port = adw::EntryRow::builder()
        .title(&tr("SIP Port"))
        .text(&current_store.sip_port.to_string())
        .visible(current_store.provider != crate::swyx_bridge::ProviderKind::Swyx)
        .build();

    let row_sip_realm = adw::EntryRow::builder()
        .title(&tr("SIP Domain / Realm (optional)"))
        .text(&current_store.sip_realm)
        .visible(matches!(
            current_store.provider,
            crate::swyx_bridge::ProviderKind::Auto | crate::swyx_bridge::ProviderKind::GenericSip
        ))
        .build();

    let advanced_expander = adw::ExpanderRow::builder()
        .title(&tr("Advanced"))
        .subtitle(&tr("Phone system, SIP port, and domain"))
        .expanded(false)
        .build();
    advanced_expander.add_row(&row_provider);
    advanced_expander.add_row(&row_sip_port);
    advanced_expander.add_row(&row_sip_realm);

    conn_group.add(&row_server);
    conn_group.add(&row_user);
    conn_group.add(&row_password);
    conn_group.add(&advanced_expander);

    // 2. Audio
    let audio_group = adw::PreferencesGroup::builder()
        .title(&tr("Audio"))
        .build();

    let output_devices = crate::engine_bridge::EngineBridge::list_output_devices();
    let input_devices = crate::engine_bridge::EngineBridge::list_input_devices();

    let default_dev_lbl = tr("System Default");
    let out_labels: Vec<&str> = output_devices
        .iter()
        .map(|d| {
            if d.id == "default" {
                default_dev_lbl.as_str()
            } else {
                d.description.as_str()
            }
        })
        .collect();
    let in_labels: Vec<&str> = input_devices
        .iter()
        .map(|d| {
            if d.id == "default" {
                default_dev_lbl.as_str()
            } else {
                d.description.as_str()
            }
        })
        .collect();

    let out_list_call = gtk::StringList::new(&out_labels);
    let out_list_ring = gtk::StringList::new(&out_labels);
    let in_list = gtk::StringList::new(&in_labels);

    let selected_out_idx = output_devices
        .iter()
        .position(|d| d.id == current_store.audio_output_device)
        .unwrap_or(0) as u32;
    let selected_ring_idx = output_devices
        .iter()
        .position(|d| d.id == current_store.audio_ring_device)
        .unwrap_or(0) as u32;
    let selected_in_idx = input_devices
        .iter()
        .position(|d| d.id == current_store.audio_input_device)
        .unwrap_or(0) as u32;

    let row_audio_out = adw::ComboRow::builder()
        .title(&tr("Headphones / Speakers"))
        .subtitle(&tr("For calls"))
        .model(&out_list_call)
        .selected(selected_out_idx)
        .build();

    let row_audio_in = adw::ComboRow::builder()
        .title(&tr("Microphone"))
        .model(&in_list)
        .selected(selected_in_idx)
        .build();

    let row_audio_ring = adw::ComboRow::builder()
        .title(&tr("Ringtone"))
        .subtitle(&tr("For incoming calls"))
        .model(&out_list_ring)
        .selected(selected_ring_idx)
        .build();

    let test_ring_btn = gtk::Button::from_icon_name("audio-volume-high-symbolic");
    test_ring_btn.add_css_class("flat");
    test_ring_btn.set_valign(gtk::Align::Center);
    test_ring_btn.set_tooltip_text(Some(&tr("Test ringtone")));
    let out_devs_for_test = output_devices.clone();
    let ring_row_for_test = row_audio_ring.clone();
    test_ring_btn.connect_clicked(move |_| {
        let idx = ring_row_for_test.selected() as usize;
        let dev_id = out_devs_for_test
            .get(idx)
            .map(|d| d.id.clone())
            .unwrap_or_else(|| "default".to_string());
        std::thread::spawn(move || {
            let candidates = [
                "/usr/share/sounds/freedesktop/stereo/phone-incoming-call.oga",
                "/usr/share/sounds/freedesktop/stereo/bell.oga",
            ];
            let mut played = false;
            if let Some(sound) = candidates.iter().find(|p| std::path::Path::new(p).exists()) {
                let mut cmd = std::process::Command::new("paplay");
                if !dev_id.is_empty() && dev_id != "default" {
                    cmd.arg(format!("--device={}", dev_id));
                }
                cmd.arg(sound);
                if let Ok(status) = cmd
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                {
                    played = status.success();
                }
            }
            if !played {
                deskphone_core::rtp::AudioDeviceManager::play_ringtone_burst(None, 0.28);
            }
        });
    });
    row_audio_ring.add_suffix(&test_ring_btn);

    audio_group.add(&row_audio_out);
    audio_group.add(&row_audio_in);
    audio_group.add(&row_audio_ring);

    // 3. General (GNOME Contacts & Background Operation)
    let general_group = adw::PreferencesGroup::builder()
        .title(&tr("General"))
        .build();

    let row_gnome_contacts = adw::SwitchRow::builder()
        .title(&tr("Sync GNOME Contacts"))
        .subtitle(&tr("Include local address books and GNOME Online Accounts"))
        .active(current_store.sync_gnome_contacts)
        .build();

    let row_background = adw::SwitchRow::builder()
        .title(&tr("Run in Background"))
        .subtitle(&tr("Keep Deskphone active for incoming calls when closing the window"))
        .active(current_store.background_on_close)
        .build();

    general_group.add(&row_gnome_contacts);
    general_group.add(&row_background);

    // Automatische Echtzeit-Speicherung bei jeder Änderung (GNOME / Libadwaita Standard)
    let out_devs_save = output_devices.clone();
    let in_devs_save = input_devices.clone();
    let save_changes = {
        let prov_row = row_provider.clone();
        let prof_grp = profile_group.clone();
        let port_row = row_sip_port.clone();
        let realm_row = row_sip_realm.clone();
        let s_row = row_server.clone();
        let u_row = row_user.clone();
        let p_row = row_password.clone();
        let ao_row = row_audio_out.clone();
        let ai_row = row_audio_in.clone();
        let ar_row = row_audio_ring.clone();
        let gc_row = row_gnome_contacts.clone();
        let bg_row = row_background.clone();
        let store_save = store_ref.clone();
        let bridge_save = bridge.clone();
        let u_lbl_save = user_label.clone();
        let u_av_save = user_avatar.clone();
        let eb_save = engine_bridge.clone();

        Rc::new(move || {
            let new_provider = crate::swyx_bridge::ProviderKind::from_index(prov_row.selected());
            prof_grp.set_visible(
                new_provider.supports_server_avatar()
                    || (new_provider == crate::swyx_bridge::ProviderKind::Auto
                        && eb_save.resolved_provider().supports_server_avatar()),
            );
            port_row.set_visible(new_provider != crate::swyx_bridge::ProviderKind::Swyx);
            realm_row.set_visible(matches!(
                new_provider,
                crate::swyx_bridge::ProviderKind::Auto | crate::swyx_bridge::ProviderKind::GenericSip
            ));

            let new_server = s_row.text().trim().to_string();
            let new_port = port_row.text().trim().parse::<u16>().unwrap_or(5060);
            let new_realm = realm_row.text().trim().to_string();
            let new_user = u_row.text().trim().to_string();
            let new_pass = p_row.text().to_string();

            let out_id = out_devs_save
                .get(ao_row.selected() as usize)
                .map(|d| d.id.clone())
                .unwrap_or_else(|| "default".to_string());
            let in_id = in_devs_save
                .get(ai_row.selected() as usize)
                .map(|d| d.id.clone())
                .unwrap_or_else(|| "default".to_string());
            let ring_id = out_devs_save
                .get(ar_row.selected() as usize)
                .map(|d| d.id.clone())
                .unwrap_or_else(|| "default".to_string());

            let new_ext = {
                let candidate = new_user.split_whitespace().last().unwrap_or("").to_string();
                if candidate.chars().all(|c| c.is_ascii_digit()) && !candidate.is_empty() {
                    candidate
                } else if new_provider == crate::swyx_bridge::ProviderKind::Swyx {
                    store_save.borrow().extension.clone()
                } else {
                    String::new()
                }
            };

            {
                let mut store = store_save.borrow_mut();
                store.provider = new_provider;
                store.sip_port = new_port;
                store.sip_realm = new_realm;
                store.server_ip = new_server.clone();
                store.user_name = new_user.clone();
                store.extension = new_ext.clone();
                store.password = new_pass;
                store.pickup_prefix = "*8".to_string();
                store.passive_mode = false;
                store.background_on_close = bg_row.is_active();
                store.audio_output_device = out_id;
                store.audio_input_device = in_id;
                store.audio_ring_device = ring_id;
                store.sync_gnome_contacts = gc_row.is_active();
                bridge_save.save_data(&store);
            }

            // Sidebar Widgets aktualisieren
            let short = if !new_user.is_empty() {
                clean_name_for_avatar(&new_user)
            } else {
                "Deskphone".to_string()
            };
            let label_text = if !new_ext.is_empty() && !short.contains(&new_ext) {
                format!("{} ({})", short, new_ext)
            } else {
                short.clone()
            };

            u_lbl_save.set_text(&label_text);
            u_av_save.set_text(Some(&short));
        })
    };

    let sc_prov = save_changes.clone();
    row_provider.connect_selected_notify(move |_| sc_prov());

    let sc1 = save_changes.clone();
    row_server.connect_changed(move |_| sc1());

    let sc_port = save_changes.clone();
    row_sip_port.connect_changed(move |_| sc_port());

    let sc_realm = save_changes.clone();
    row_sip_realm.connect_changed(move |_| sc_realm());

    let sc2 = save_changes.clone();
    row_user.connect_changed(move |_| sc2());

    let sc4 = save_changes.clone();
    row_password.connect_changed(move |_| sc4());

    let sc_ao = save_changes.clone();
    row_audio_out.connect_selected_notify(move |_| sc_ao());

    let sc_ai = save_changes.clone();
    row_audio_in.connect_selected_notify(move |_| sc_ai());

    let sc_ar = save_changes.clone();
    row_audio_ring.connect_selected_notify(move |_| sc_ar());

    let sc_gc = save_changes.clone();
    row_gnome_contacts.connect_active_notify(move |row| {
        sc_gc();
        on_gnome_contacts_toggled(row.is_active());
    });

    let sc_bg = save_changes.clone();
    row_background.connect_active_notify(move |_| sc_bg());

    let initial_conn_sig = (
        current_store.provider,
        current_store.server_ip.clone(),
        current_store.sip_port,
        current_store.sip_realm.clone(),
        current_store.user_name.clone(),
        current_store.password.clone(),
    );
    let store_on_close = store_ref.clone();
    dialog.connect_closed(move |_| {
        let st = store_on_close.borrow();
        let current_sig = (
            st.provider,
            st.server_ip.clone(),
            st.sip_port,
            st.sip_realm.clone(),
            st.user_name.clone(),
            st.password.clone(),
        );
        if current_sig != initial_conn_sig
            && !st.server_ip.trim().is_empty()
            && !st.user_name.trim().is_empty()
        {
            on_reconnect();
        }
    });

    page.add(&profile_group);
    page.add(&conn_group);
    page.add(&audio_group);
    page.add(&general_group);
    dialog.add(&page);

    dialog.present(Some(parent));
}
