pub mod chat_window;
pub mod contacts_view;
pub mod dialer_view;
pub mod header;
pub mod incall_view;
pub mod journal_view;
pub mod settings_dialog;
pub mod sidebar;
pub mod window;

use gtk4 as gtk;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

thread_local! {
    static PHOTO_CACHE: RefCell<HashMap<String, gtk::gdk::Texture>> = RefCell::new(HashMap::new());
}

pub fn avatar_cache_dir() -> PathBuf {
    let dir = gtk::glib::user_cache_dir().join("deskphone").join("avatars");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn sanitize_file_id(file_id: &str) -> String {
    file_id
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

fn avatar_file_path(file_id: &str) -> Option<PathBuf> {
    let clean = sanitize_file_id(file_id);
    if clean.is_empty() {
        return None;
    }
    Some(avatar_cache_dir().join(format!("{}.img", clean)))
}

fn own_avatar_cache_path() -> PathBuf {
    avatar_cache_dir().join("self.img")
}

fn normalize_photo_key(s: &str) -> String {
    s.trim().to_lowercase()
}

fn decode_texture_from_bytes(bytes: &[u8]) -> Option<gtk::gdk::Texture> {
    if bytes.is_empty() {
        return None;
    }
    let gbytes = gtk::glib::Bytes::from(bytes);
    gtk::gdk::Texture::from_bytes(&gbytes).ok()
}

fn register_texture_aliases(names: &[String], texture: &gtk::gdk::Texture) {
    PHOTO_CACHE.with(|cache| {
        let mut map = cache.borrow_mut();
        for name in names {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                continue;
            }
            map.insert(normalize_photo_key(trimmed), texture.clone());
            let clean = clean_name_for_avatar(trimmed);
            if !clean.trim().is_empty() {
                map.insert(normalize_photo_key(&clean), texture.clone());
            }
            let hk = sidebar::format_hotkey_display_name(trimmed);
            if !hk.trim().is_empty() {
                map.insert(normalize_photo_key(&hk), texture.clone());
            }
        }
    });
}

fn contact_aliases(contact: &crate::models::Contact) -> Vec<String> {
    let mut keys = Vec::new();
    if !contact.name.trim().is_empty() {
        keys.push(contact.name.clone());
    }
    if !contact.extension.trim().is_empty() {
        keys.push(contact.extension.clone());
    }
    if !contact.phone.trim().is_empty() {
        keys.push(contact.phone.clone());
    }
    keys
}

pub fn has_avatar_file_on_disk(file_id: &str) -> bool {
    avatar_file_path(file_id)
        .map(|p| p.exists())
        .unwrap_or(false)
}

/// Lädt ein bereits zwischengespeichertes Kontakt-Profilbild von der Festplatte in den Speicher-Cache.
pub fn preload_cached_contact_photo(contact: &crate::models::Contact) -> bool {
    let Some(ref fid) = contact.avatar_file_id else {
        return false;
    };
    let Some(path) = avatar_file_path(fid) else {
        return false;
    };
    if let Ok(bytes) = std::fs::read(&path) {
        if let Some(tex) = decode_texture_from_bytes(&bytes) {
            register_texture_aliases(&contact_aliases(contact), &tex);
            return true;
        }
    }
    false
}

/// Speichert heruntergeladene Swyx-Profilbild-Bytes lokal im Cache und macht sie sofort in der UI verfügbar.
pub fn store_contact_photo_bytes(
    contact: &crate::models::Contact,
    file_id: &str,
    bytes: &[u8],
) -> Option<gtk::gdk::Texture> {
    let tex = decode_texture_from_bytes(bytes)?;
    if let Some(path) = avatar_file_path(file_id) {
        let _ = std::fs::write(&path, bytes);
    }
    register_texture_aliases(&contact_aliases(contact), &tex);
    Some(tex)
}

/// Entfernt zwischengespeicherte Kontakt-Avatar-Einträge aus dem In-Memory-Cache (z. B. bei EDS-Opt-Out).
pub fn clear_contact_photo_cache(contact: &crate::models::Contact) {
    let aliases = contact_aliases(contact);
    PHOTO_CACHE.with(|cache| {
        let mut map = cache.borrow_mut();
        for name in aliases {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                continue;
            }
            map.remove(&normalize_photo_key(trimmed));
            let clean = clean_name_for_avatar(trimmed);
            if !clean.trim().is_empty() {
                map.remove(&normalize_photo_key(&clean));
            }
            let hk = sidebar::format_hotkey_display_name(trimmed);
            if !hk.trim().is_empty() {
                map.remove(&normalize_photo_key(&hk));
            }
        }
    });
}

/// Lädt das zwischengespeicherte eigene Profilbild (falls vorhanden).
pub fn load_own_avatar_texture() -> Option<gtk::gdk::Texture> {
    let path = own_avatar_cache_path();
    let bytes = std::fs::read(&path).ok()?;
    decode_texture_from_bytes(&bytes)
}

/// Speichert das eigene Profilbild im lokalen Cache und registriert es auch unter dem eigenen Namen.
pub fn store_own_avatar_bytes(bytes: &[u8], own_aliases: &[String]) -> Option<gtk::gdk::Texture> {
    let tex = decode_texture_from_bytes(bytes)?;
    let _ = std::fs::write(own_avatar_cache_path(), bytes);
    if !own_aliases.is_empty() {
        register_texture_aliases(own_aliases, &tex);
    }
    Some(tex)
}

/// Entfernt das zwischengespeicherte eigene Profilbild lokal (z. B. nach "Kein Bild" / Löschen).
pub fn clear_own_avatar_cache(own_aliases: &[String]) {
    let _ = std::fs::remove_file(own_avatar_cache_path());
    PHOTO_CACHE.with(|cache| {
        let mut map = cache.borrow_mut();
        for name in own_aliases {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                continue;
            }
            map.remove(&normalize_photo_key(trimmed));
            let clean = clean_name_for_avatar(trimmed);
            if !clean.trim().is_empty() {
                map.remove(&normalize_photo_key(&clean));
            }
            let hk = sidebar::format_hotkey_display_name(trimmed);
            if !hk.trim().is_empty() {
                map.remove(&normalize_photo_key(&hk));
            }
        }
    });
}

/// Konvertiert eine gewählte Bilddatei (JPEG/PNG/WebP/BMP) in ein sauberes quadratisches 256×256 JPEG
/// (inkl. automatischer EXIF-Drehung), das SwyxWare direkt per `PUT /user/{id}/image` akzeptiert.
pub fn prepare_avatar_jpeg_bytes(path: &Path) -> anyhow::Result<Vec<u8>> {
    let raw = gtk::gdk_pixbuf::Pixbuf::from_file(path)
        .map_err(|e| anyhow::anyhow!("Bild konnte nicht geladen werden: {}", e))?;
    let oriented = raw.apply_embedded_orientation().unwrap_or(raw);
    let w = oriented.width();
    let h = oriented.height();
    if w <= 0 || h <= 0 {
        return Err(anyhow::anyhow!("Ungültige Bildgröße"));
    }

    let side = w.min(h);
    let offset_x = (w - side) / 2;
    let offset_y = (h - side) / 2;
    let cropped = oriented.new_subpixbuf(offset_x, offset_y, side, side);

    let target_size = side.min(256);
    let scaled = if side > target_size {
        cropped
            .scale_simple(
                target_size,
                target_size,
                gtk::gdk_pixbuf::InterpType::Bilinear,
            )
            .unwrap_or(cropped)
    } else {
        cropped
    };

    let bytes = scaled
        .save_to_bufferv("jpeg", &[("quality", "88")])
        .map_err(|e| anyhow::anyhow!("JPEG-Kodierung fehlgeschlagen: {}", e))?;
    Ok(bytes)
}

pub fn get_contact_photo(name: &str) -> Option<gtk::gdk::Texture> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }

    PHOTO_CACHE.with(|cache| {
        let map = cache.borrow();
        if let Some(tex) = map.get(&normalize_photo_key(trimmed)) {
            return Some(tex.clone());
        }
        let clean = clean_name_for_avatar(trimmed);
        if let Some(tex) = map.get(&normalize_photo_key(&clean)) {
            return Some(tex.clone());
        }
        let hk = sidebar::format_hotkey_display_name(trimmed);
        if let Some(tex) = map.get(&normalize_photo_key(&hk)) {
            return Some(tex.clone());
        }
        None
    })
}

/// Prüft, ob ein Text eine reine Rufnummer ohne Kontaktnamen ist (z. B. "+49...", "089...", "0171...")
pub fn is_phone_number(text: &str) -> bool {
    let clean = text.trim();
    if clean.is_empty() {
        return true;
    }
    // Wenn kein einziger Buchstabe vorkommt (nur Ziffern, +, Leerzeichen, Klammern, etc.),
    // handelt es sich um eine reine Telefonnummer und keinen Namen.
    !clean.chars().any(|c| c.is_alphabetic())
}

/// Bereinigt Namen für den Avatar:
/// 1. Entfernt reine Durchwahlnummern am Ende (z. B. "Mustermann M. 100" -> "Mustermann M.")
/// 2. Dreht das Firmen-Namensschema ("Nachname Vorname" -> "Vorname Nachname"),
///    damit Libadwaita die Initialen in der natürlichen Reihenfolge "Vorname Nachname"
///    anzeigt (z. B. "Mustermann M." -> "MM", "Schmidt S." -> "SS").
pub fn clean_name_for_avatar(name: &str) -> String {
    let parts: Vec<&str> = name.split_whitespace().collect();
    if parts.is_empty() {
        return name.to_string();
    }
    // Filtert reine Ziffernblöcke / Durchwahlen am Ende heraus
    let mut filtered: Vec<String> = parts
        .into_iter()
        .filter(|part| part.chars().any(|c| c.is_alphabetic()))
        .map(|part| part.trim_matches(',').to_string())
        .filter(|part| !part.is_empty())
        .collect();

    if filtered.is_empty() {
        name.to_string()
    } else if filtered.len() >= 2 {
        // Reihenfolge umkehren: "Nachname Vorname" -> "Vorname Nachname"
        filtered.reverse();
        filtered.join(" ")
    } else {
        filtered.join(" ")
    }
}

pub fn update_presence_dot(dot: &gtk::Box, presence: Option<crate::models::PresenceState>) {
    use gtk4::prelude::*;

    dot.remove_css_class("presence-available");
    dot.remove_css_class("presence-busy");
    dot.remove_css_class("presence-away");
    dot.remove_css_class("presence-offline");
    dot.remove_css_class("presence-ringing");
    dot.remove_css_class("presence-empty");

    if let Some(p) = presence {
        dot.add_css_class(p.css_class());
        dot.set_visible(true);
    } else {
        dot.add_css_class("presence-empty");
        dot.set_visible(false);
    }
}

pub fn create_presence_avatar_with_dot(
    name: &str,
    presence: Option<crate::models::PresenceState>,
    size: i32,
) -> (gtk::Widget, gtk::Box) {
    use gtk4::prelude::*;
    use libadwaita as adw;
    use adw::prelude::BinExt;

    let is_number = is_phone_number(name);

    let avatar: gtk::Widget = if is_number {
        // Reine Rufnummer: Ruhiger neutral-grauer Kreis mit Person-Silhouette (ohne bunte AdwAvatar-Zufallsfarben).
        // WICHTIG: Niemals hexpand/vexpand setzen, da GTK 4 expand nach oben in AdwActionRow-Prefixes propagiert!
        let neutral = adw::Bin::new();
        neutral.add_css_class("avatar-neutral-number");
        neutral.set_size_request(size, size);
        neutral.set_halign(gtk::Align::Center);
        neutral.set_valign(gtk::Align::Center);
        neutral.set_hexpand(false);
        neutral.set_vexpand(false);

        let icon = gtk::Image::from_icon_name("avatar-default-symbolic");
        icon.set_pixel_size((size / 2).max(14));
        icon.set_halign(gtk::Align::Center);
        icon.set_valign(gtk::Align::Center);
        neutral.set_child(Some(&icon));
        neutral.upcast()
    } else {
        let clean = clean_name_for_avatar(name);
        let av = adw::Avatar::builder()
            .size(size)
            .text(&clean)
            .show_initials(true)
            .build();
        if let Some(texture) = get_contact_photo(name) {
            av.set_custom_image(Some(&texture));
        }
        av.upcast()
    };

    let overlay = gtk::Overlay::new();
    // Give 2px extra space (size + 2) so the badge sits cleanly on the outer circle boundary
    let overlay_size = size + 2;
    overlay.set_size_request(overlay_size, overlay_size);
    overlay.set_valign(gtk::Align::Center);
    overlay.set_halign(gtk::Align::Center);
    overlay.set_hexpand(false);
    overlay.set_vexpand(false);
    overlay.set_overflow(gtk::Overflow::Visible);
    overlay.set_child(Some(&avatar));

    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("presence-badge-dot");
    dot.set_halign(gtk::Align::End);
    dot.set_valign(gtk::Align::End);
    update_presence_dot(&dot, presence);
    overlay.add_overlay(&dot);

    (overlay.upcast(), dot)
}

#[allow(dead_code)]
pub fn create_presence_avatar(
    name: &str,
    presence: Option<crate::models::PresenceState>,
    size: i32,
) -> gtk::Widget {
    create_presence_avatar_with_dot(name, presence, size).0
}

thread_local! {
    static LAST_DROPPED_CALL_TAG: RefCell<String> = const { RefCell::new(String::new()) };
}

pub fn take_last_dropped_call_tag() -> String {
    LAST_DROPPED_CALL_TAG.with(|t| std::mem::take(&mut *t.borrow_mut()))
}

/// Macht eine Leitung oder aktive Gesprächskarte ziehbar (Drag & Drop),
/// sodass das Telefonat direkt auf einen Kontakt, Journal-Eintrag oder Hotkey gezogen werden kann.
pub fn attach_call_drag_source<W>(widget: &W)
where
    W: gtk::prelude::IsA<gtk::Widget> + Clone + 'static,
{
    attach_call_drag_source_with_tag(widget, "CALL_TRANSFER", || true);
}

pub fn attach_call_drag_source_with_tag<W, FCanDrag>(
    widget: &W,
    payload_tag: &'static str,
    can_drag: FCanDrag,
) where
    W: gtk::prelude::IsA<gtk::Widget> + Clone + 'static,
    FCanDrag: Fn() -> bool + 'static,
{
    use gtk4::prelude::*;

    let drag_source = gtk::DragSource::new();
    drag_source.set_actions(gtk::gdk::DragAction::MOVE | gtk::gdk::DragAction::COPY);
    drag_source.set_propagation_phase(gtk::PropagationPhase::Bubble);

    drag_source.connect_prepare(move |_, _, _| {
        if can_drag() {
            Some(gtk::gdk::ContentProvider::for_value(
                &payload_tag.to_value(),
            ))
        } else {
            None
        }
    });

    let w_begin = widget.clone();
    drag_source.connect_drag_begin(move |src, _| {
        w_begin.add_css_class("dragging-call");
        w_begin.set_cursor_from_name(Some("grabbing"));
        let paintable = gtk::WidgetPaintable::new(Some(&w_begin));
        src.set_icon(Some(&paintable), 16, 16);
    });

    let w_end = widget.clone();
    drag_source.connect_drag_end(move |_, _, _| {
        w_end.remove_css_class("dragging-call");
        w_end.set_cursor_from_name(None);
    });

    widget.add_controller(drag_source);
}

/// Macht einen Kontakt, Journal-Eintrag oder Kurzwahl-Hotkey zum Drop-Ziel für gezogene Leitungen/Anrufe.
pub fn attach_call_drop_target<W, FTransfer>(
    widget: &W,
    target_name: String,
    target_number: String,
    on_transfer: FTransfer,
) where
    W: gtk::prelude::IsA<gtk::Widget> + Clone + 'static,
    FTransfer: Fn(String, String) + 'static + Clone,
{
    use gtk4::prelude::*;

    let drop_target = gtk::DropTarget::new(
        gtk::glib::types::Type::STRING,
        gtk::gdk::DragAction::MOVE | gtk::gdk::DragAction::COPY,
    );

    let w_enter = widget.clone();
    drop_target.connect_enter(move |_, _, _| {
        w_enter.add_css_class("drop-target-transfer");
        gtk::gdk::DragAction::MOVE
    });

    let w_leave = widget.clone();
    drop_target.connect_leave(move |_| {
        w_leave.remove_css_class("drop-target-transfer");
    });

    let w_drop = widget.clone();
    drop_target.connect_drop(move |_, val, _, _| {
        w_drop.remove_css_class("drop-target-transfer");
        let tag = val.get::<String>().unwrap_or_default();
        let is_transfer = tag == "CALL_TRANSFER" || tag == "CALL_TRANSFER_HELD";
        if is_transfer && !target_number.is_empty() {
            LAST_DROPPED_CALL_TAG.with(|t| *t.borrow_mut() = tag);
            on_transfer(target_name.clone(), target_number.clone());
            true
        } else {
            false
        }
    });

    widget.add_controller(drop_target);
}

/// Ermöglicht das Zusammenziehen von Leitung 1 auf Leitung 2 (oder umgekehrt) für Attended Transfer.
pub fn attach_line_connect_drop_target<W, FConnect>(
    widget: &W,
    accepted_tag: &'static str,
    on_connect: FConnect,
) where
    W: gtk::prelude::IsA<gtk::Widget> + Clone + 'static,
    FConnect: Fn() + 'static + Clone,
{
    use gtk4::prelude::*;

    let drop_target = gtk::DropTarget::new(
        gtk::glib::types::Type::STRING,
        gtk::gdk::DragAction::MOVE | gtk::gdk::DragAction::COPY,
    );

    let w_enter = widget.clone();
    drop_target.connect_enter(move |_, _, _| {
        w_enter.add_css_class("drop-target-transfer");
        gtk::gdk::DragAction::MOVE
    });

    let w_leave = widget.clone();
    drop_target.connect_leave(move |_| {
        w_leave.remove_css_class("drop-target-transfer");
    });

    let w_drop = widget.clone();
    drop_target.connect_drop(move |_, val, _, _| {
        w_drop.remove_css_class("drop-target-transfer");
        let tag = val.get::<String>().unwrap_or_default();
        if tag == accepted_tag {
            on_connect();
            true
        } else {
            false
        }
    });

    widget.add_controller(drop_target);
}


