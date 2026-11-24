//! Pure-Rust GNOME Contacts / Evolution Data Server (EDS) D-Bus integration.
//!
//! Queries `org.gnome.evolution.dataserver.Sources5` and
//! `org.gnome.evolution.dataserver.AddressBook10` read-only over the session D-Bus.
//! Because GNOME Online Accounts (Google, Nextcloud, CardDAV, Microsoft 365)
//! automatically synchronizes contacts into EDS, this provides both local GNOME
//! Contacts and Online Account contacts without any C library dependencies.

use std::collections::{HashMap, HashSet};

use crate::models::{Contact, PresenceState};

pub const EDS_ID_PREFIX: &str = "eds-";
pub const EDS_PHOTO_PREFIX: &str = "eds-photo-";
const MAX_VCARD_PHOTO_BYTES: usize = 5 * 1024 * 1024;

/// Returns whether EDS integration is disabled (tests or explicit env opt-out).
fn eds_disabled() -> bool {
    cfg!(test) || std::env::var_os("DESKPHONE_DISABLE_EDS").is_some()
}

/// Normalizes a phone number to its ASCII digits.
pub fn normalize_phone_digits(raw: &str) -> String {
    raw.chars().filter(|c| c.is_ascii_digit()).collect()
}

/// Returns a canonical comparison key for phone numbers so national (`030 23125 192`)
/// and international (`+49 30 23125 192` / `0049 30 23125 192`) formats match cleanly.
pub fn canonical_phone_key(raw: &str) -> Option<String> {
    let digits = normalize_phone_digits(raw);
    if digits.len() < 5 {
        return None;
    }
    if digits.len() >= 8 {
        Some(digits[digits.len() - 8..].to_string())
    } else {
        Some(digits)
    }
}

/// Returns an order-insensitive canonical key for multi-word contact names so
/// `"Weber, Clara 201"` (Swyx) and `"Clara Weber"` (GNOME/Google Contacts) match.
/// Single-word names return `None` to avoid accidental false merges.
pub fn canonical_name_key(raw: &str) -> Option<String> {
    let mut words: Vec<String> = raw
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| w.chars().count() >= 2)
        .map(|w| w.to_lowercase())
        .collect();
    if words.len() < 2 {
        return None;
    }
    words.sort();
    words.dedup();
    if words.len() < 2 {
        return None;
    }
    Some(words.join(" "))
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Decodes standard or URL-safe Base64, ignoring embedded whitespace.
pub fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u8 = 0;

    for b in input.bytes() {
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }

    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn decode_percent_uri_path(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(val) = u8::from_str_radix(hex, 16) {
                    decoded.push(val);
                    i += 3;
                    continue;
                }
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn extract_vcard_photo_bytes(prop_part: &str, val_part: &str) -> Option<Vec<u8>> {
    let trimmed = val_part.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower_val = trimmed.to_ascii_lowercase();

    let bytes = if lower_val.starts_with("data:") {
        let idx = lower_val.find(";base64,")?;
        decode_base64(&trimmed[idx + 8..])?
    } else if lower_val.starts_with("file://") {
        let raw_path = &trimmed[7..];
        let local_path = raw_path.strip_prefix("localhost").unwrap_or(raw_path);
        let decoded_path = decode_percent_uri_path(local_path);
        let meta = std::fs::metadata(&decoded_path).ok()?;
        if meta.len() == 0 || meta.len() > MAX_VCARD_PHOTO_BYTES as u64 {
            return None;
        }
        std::fs::read(&decoded_path).ok()?
    } else {
        let upper_prop = prop_part.to_ascii_uppercase();
        if upper_prop.contains("ENCODING=B") || upper_prop.contains("ENCODING=BASE64") {
            decode_base64(trimmed)?
        } else {
            return None;
        }
    };

    if bytes.is_empty() || bytes.len() > MAX_VCARD_PHOTO_BYTES {
        return None;
    }
    Some(bytes)
}

/// Inspects an EDS `.source` INI blob and returns `Some(display_name)` if it is
/// an enabled `[Address Book]` source.
pub fn parse_enabled_address_book_source(ini: &str) -> Option<String> {
    let mut section = "";
    let mut has_address_book = false;
    let mut data_source_enabled = true;
    let mut address_book_enabled = true;
    let mut display_name: Option<String> = None;

    for raw_line in ini.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line;
            if section == "[Address Book]" {
                has_address_book = true;
            }
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            let val = v.trim();
            match (section, key) {
                ("[Data Source]", "Enabled") if val.eq_ignore_ascii_case("false") => {
                    data_source_enabled = false;
                }
                ("[Address Book]", "Enabled") if val.eq_ignore_ascii_case("false") => {
                    address_book_enabled = false;
                }
                ("[Data Source]", "DisplayName") if !val.is_empty() => {
                    display_name = Some(val.to_string());
                }
                _ => {}
            }
        }
    }

    if has_address_book && data_source_enabled && address_book_enabled {
        Some(display_name.unwrap_or_else(|| crate::i18n::tr("GNOME Contacts")))
    } else {
        None
    }
}

/// Unfolds RFC 2426 / RFC 6350 folded vCard lines (CRLF/LF followed by space or tab).
fn unfold_vcard(raw: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for part in raw.replace("\r\n", "\n").split('\n') {
        if (part.starts_with(' ') || part.starts_with('\t')) && !lines.is_empty() {
            if let Some(last) = lines.last_mut() {
                last.push_str(&part[1..]);
            }
        } else if !part.trim().is_empty() {
            lines.push(part.to_string());
        }
    }
    lines
}

/// Unescapes standard vCard backslash escapes (`\,`, `\;`, `\n`).
fn unescape_vcard_value(val: &str) -> String {
    val.replace("\\n", " ")
        .replace("\\N", " ")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
        .trim()
        .to_string()
}

/// Extracts one `Contact` per valid phone number from a single vCard string.
#[allow(dead_code)]
pub fn parse_vcard_contacts(
    vcard: &str,
    source_uid: &str,
    source_label: &str,
) -> Vec<Contact> {
    parse_vcard_contacts_with_photo(vcard, source_uid, source_label).0
}

/// Extracts contacts and any embedded/local `PHOTO` payload from a single vCard string.
pub fn parse_vcard_contacts_with_photo(
    vcard: &str,
    source_uid: &str,
    source_label: &str,
) -> (Vec<Contact>, Option<(String, Vec<u8>)>) {
    let mut uid = String::new();
    let mut full_name = String::new();
    let mut structured_name = String::new();
    let mut org = String::new();
    let mut phones: Vec<String> = Vec::new();
    let mut photo_bytes: Option<Vec<u8>> = None;

    for line in unfold_vcard(vcard) {
        let Some((prop_part, val_part)) = line.split_once(':') else {
            continue;
        };
        // Strip group prefix (e.g. "item1.TEL;TYPE=CELL" -> "TEL") and parameters
        let prop_no_group = prop_part.rsplit('.').next().unwrap_or(prop_part);
        let prop_name = prop_no_group
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_uppercase();

        match prop_name.as_str() {
            "UID" if uid.is_empty() => {
                uid = unescape_vcard_value(val_part);
            }
            "FN" if full_name.is_empty() => {
                full_name = unescape_vcard_value(val_part);
            }
            "N" if structured_name.is_empty() => {
                // N:Last;First;Middle;Prefix;Suffix
                let parts: Vec<&str> = val_part.split(';').collect();
                let last = parts.first().copied().unwrap_or("").trim();
                let first = parts.get(1).copied().unwrap_or("").trim();
                let combined = match (first.is_empty(), last.is_empty()) {
                    (false, false) => format!("{} {}", first, last),
                    (false, true) => first.to_string(),
                    (true, false) => last.to_string(),
                    (true, true) => String::new(),
                };
                structured_name = unescape_vcard_value(&combined);
            }
            "ORG" if org.is_empty() => {
                let first_org = val_part.split(';').next().unwrap_or("");
                org = unescape_vcard_value(first_org);
            }
            "TEL" => {
                let cleaned = unescape_vcard_value(val_part);
                let stripped = cleaned
                    .strip_prefix("tel:")
                    .or_else(|| cleaned.strip_prefix("TEL:"))
                    .unwrap_or(&cleaned)
                    .trim();
                if stripped.chars().any(|c| c.is_ascii_digit()) {
                    phones.push(stripped.to_string());
                }
            }
            "PHOTO" if photo_bytes.is_none() => {
                photo_bytes = extract_vcard_photo_bytes(prop_part, val_part);
            }
            _ => {}
        }
    }

    let name = if !full_name.is_empty() {
        full_name
    } else if !structured_name.is_empty() {
        structured_name
    } else {
        return (Vec::new(), None);
    };

    if phones.is_empty() {
        return (Vec::new(), None);
    }

    let dept = if !org.is_empty() {
        org
    } else if !source_label.trim().is_empty() {
        source_label.trim().to_string()
    } else {
        crate::i18n::tr("GNOME Contacts")
    };

    let photo_entry = photo_bytes.map(|bytes| {
        let fid = format!("{}{:016x}", EDS_PHOTO_PREFIX, fnv1a_64(&bytes));
        (fid, bytes)
    });
    let avatar_file_id = photo_entry.as_ref().map(|(fid, _)| fid.clone());

    let mut out = Vec::new();
    let mut seen_nums = HashSet::new();
    let id_base = if !uid.is_empty() { uid } else { name.clone() };

    for (idx, phone) in phones.into_iter().enumerate() {
        let norm = normalize_phone_digits(&phone);
        if norm.len() < 2 || !seen_nums.insert(norm) {
            continue;
        }
        out.push(Contact {
            id: format!("{}{}-{}-{}", EDS_ID_PREFIX, source_uid, id_base, idx),
            name: name.clone(),
            extension: String::new(),
            department: dept.clone(),
            presence: PresenceState::LoggedOut,
            phone,
            avatar_file_id: avatar_file_id.clone(),
        });
    }
    (out, photo_entry)
}

/// Merges freshly loaded EDS contacts into the live `contacts` list (read-only view):
/// - Preserves all PBX contacts (`swyx-*`, `pb-*`, etc.)
/// - Enriches matching PBX contacts (by canonical phone number or multi-word name) with
///   missing profile photos and direct phone numbers from GNOME Contacts / Online Accounts
/// - Deduplicates matching entries across local and online address books
/// Returns `true` if the list changed.
pub fn merge_eds_contacts(contacts: &mut Vec<Contact>, eds_contacts: Vec<Contact>) -> bool {
    let before = contacts.clone();

    // Remove previous EDS entries and clear borrowed EDS photo IDs on PBX contacts so
    // disabling or refreshing GNOME Contacts always yields a clean deterministic state.
    contacts.retain(|c| !c.id.starts_with(EDS_ID_PREFIX));
    for c in contacts.iter_mut() {
        if c.avatar_file_id
            .as_deref()
            .is_some_and(|fid| fid.starts_with(EDS_PHOTO_PREFIX))
        {
            c.avatar_file_id = None;
        }
    }

    // Build lookup indexes for existing PBX contacts (phone key -> index, multi-word name key -> index)
    let mut pbx_by_phone: HashMap<String, usize> = HashMap::new();
    let mut pbx_by_name: HashMap<String, usize> = HashMap::new();
    for (idx, c) in contacts.iter().enumerate() {
        for candidate in [&c.phone, &c.extension] {
            if let Some(key) = canonical_phone_key(candidate) {
                pbx_by_phone.entry(key).or_insert(idx);
            }
        }
        if let Some(name_key) = canonical_name_key(&c.name) {
            pbx_by_name.entry(name_key).or_insert(idx);
        }
    }

    // Track already-added EDS entries so duplicates across multiple address books
    // (e.g. Local + Google) collapse into a single entry and still retain any photo.
    let mut eds_by_phone: HashMap<String, usize> = HashMap::new();

    for ec in eds_contacts {
        let phone_key = canonical_phone_key(&ec.phone);
        let name_key = canonical_name_key(&ec.name);

        // 1. Check if this EDS contact matches an existing PBX colleague
        let pbx_match = phone_key
            .as_ref()
            .and_then(|k| pbx_by_phone.get(k).copied())
            .or_else(|| name_key.as_ref().and_then(|k| pbx_by_name.get(k).copied()));

        if let Some(pbx_idx) = pbx_match {
            let pbx = &mut contacts[pbx_idx];
            if pbx.avatar_file_id.is_none() && ec.avatar_file_id.is_some() {
                pbx.avatar_file_id = ec.avatar_file_id.clone();
            }
            if pbx.phone.trim().is_empty() && !ec.phone.trim().is_empty() {
                pbx.phone = ec.phone.clone();
                if let Some(k) = canonical_phone_key(&pbx.phone) {
                    pbx_by_phone.entry(k).or_insert(pbx_idx);
                }
            }
            continue;
        }

        // 2. Deduplicate across multiple EDS address books by canonical phone number
        if let Some(ref k) = phone_key {
            if let Some(&existing_idx) = eds_by_phone.get(k) {
                if contacts[existing_idx].avatar_file_id.is_none() && ec.avatar_file_id.is_some() {
                    contacts[existing_idx].avatar_file_id = ec.avatar_file_id;
                }
                continue;
            }
        }

        let new_idx = contacts.len();
        if let Some(k) = phone_key {
            eds_by_phone.insert(k, new_idx);
        }
        contacts.push(ec);
    }

    // Keep PBX contacts (with internal extensions) nicely grouped, then alphabetical
    contacts.sort_by(|a, b| {
        let a_eds = a.id.starts_with(EDS_ID_PREFIX);
        let b_eds = b.id.starts_with(EDS_ID_PREFIX);
        a_eds
            .cmp(&b_eds)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    *contacts != before
}

/// Queries Evolution Data Server over the user session D-Bus and returns all contacts
/// with phone numbers from enabled local & GNOME Online Accounts address books.
pub async fn fetch_eds_contacts() -> Vec<Contact> {
    if eds_disabled() {
        return Vec::new();
    }

    match fetch_eds_contacts_inner().await {
        Ok(list) => {
            tracing::info!("GNOME Contacts (EDS): loaded {} contact entries", list.len());
            list
        }
        Err(e) => {
            tracing::debug!("GNOME Contacts (EDS) unavailable: {}", e);
            Vec::new()
        }
    }
}

async fn fetch_eds_contacts_inner() -> Result<Vec<Contact>, Box<dyn std::error::Error + Send + Sync>> {
    use zbus::zvariant::{OwnedObjectPath, OwnedValue};

    let conn = zbus::Connection::session().await?;

    // 1. Discover all enabled [Address Book] sources via Sources5 ObjectManager
    let sources_proxy = zbus::Proxy::new(
        &conn,
        "org.gnome.evolution.dataserver.Sources5",
        "/org/gnome/evolution/dataserver/SourceManager",
        "org.freedesktop.DBus.ObjectManager",
    )
    .await?;

    type ManagedObjects =
        HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

    let managed: ManagedObjects = sources_proxy.call("GetManagedObjects", &()).await?;
    let mut address_books: Vec<(String, String)> = Vec::new();

    for (_obj_path, ifaces) in managed {
        let Some(props) = ifaces.get("org.gnome.evolution.dataserver.Source") else {
            continue;
        };
        let uid = props
            .get("UID")
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .unwrap_or_default();
        let data = props
            .get("Data")
            .and_then(|v| String::try_from(v.try_clone().ok()?).ok())
            .unwrap_or_default();

        if uid.is_empty() || data.is_empty() {
            continue;
        }
        if let Some(display_name) = parse_enabled_address_book_source(&data) {
            address_books.push((uid, display_name));
        }
    }

    if address_books.is_empty() {
        return Ok(Vec::new());
    }

    // 2. Open each address book read-only via AddressBookFactory and read vCards
    let factory_proxy = zbus::Proxy::new(
        &conn,
        "org.gnome.evolution.dataserver.AddressBook10",
        "/org/gnome/evolution/dataserver/AddressBookFactory",
        "org.gnome.evolution.dataserver.AddressBookFactory",
    )
    .await?;

    let mut all_contacts = Vec::new();

    for (uid, display_name) in address_books {
        let open_res: Result<(String, String), _> =
            factory_proxy.call("OpenAddressBook", &(uid.as_str(),)).await;
        let Ok((book_path, bus_name)) = open_res else {
            continue;
        };

        let Ok(book_proxy) = zbus::Proxy::new(
            &conn,
            bus_name.as_str(),
            book_path.as_str(),
            "org.gnome.evolution.dataserver.AddressBook",
        )
        .await
        else {
            continue;
        };

        let _ = book_proxy.call::<_, _, Vec<String>>("Open", &()).await;
        if let Ok(vcards) = book_proxy
            .call::<_, _, Vec<String>>(
                "GetContactList",
                &("(contains \"x-evolution-any-field\" \"\")",),
            )
            .await
        {
            let cache_dir = crate::ui::avatar_cache_dir();
            for vcard in vcards {
                let (parsed, photo) = parse_vcard_contacts_with_photo(&vcard, &uid, &display_name);
                if let Some((file_id, bytes)) = photo {
                    let target = cache_dir.join(format!("{}.img", file_id));
                    if !target.exists() {
                        let _ = std::fs::write(&target, &bytes);
                    }
                }
                all_contacts.extend(parsed);
            }
        }
        let _ = book_proxy.call::<_, _, ()>("Close", &()).await;
    }

    Ok(all_contacts)
}
