use std::fs;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

use crate::models::{
    CallJournalEntry, Contact, initial_mock_contacts, initial_mock_journal,
};
use crate::safety::{SafetyError, SafetyGate};

/// Unterstützte Telefonie-Anbieter / PBX-Protokolle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderKind {
    Auto,
    GenericSip,
    Swyx,
    Asterisk,
}

impl Default for ProviderKind {
    fn default() -> Self {
        // Abwärtskompatibel für bestehende Deskphone-Konfigurationen ohne `provider`-Feld
        Self::Swyx
    }
}

impl ProviderKind {
    pub fn index(&self) -> u32 {
        match self {
            Self::Auto => 0,
            Self::GenericSip => 1,
            Self::Swyx => 2,
            Self::Asterisk => 3,
        }
    }

    pub fn from_index(idx: u32) -> Self {
        match idx {
            0 => Self::Auto,
            1 => Self::GenericSip,
            2 => Self::Swyx,
            3 => Self::Asterisk,
            _ => Self::Auto,
        }
    }

    pub fn supports_server_avatar(&self) -> bool {
        matches!(self, Self::Swyx)
    }
}

/// Data container stored in ~/.config/deskphone/data.json for persistence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwyxDataStore {
    #[serde(default)]
    pub provider: ProviderKind,
    #[serde(default = "default_sip_port")]
    pub sip_port: u16,
    #[serde(default)]
    pub sip_realm: String,
    #[serde(default)]
    pub server_ip: String,
    #[serde(default)]
    pub user_name: String,
    #[serde(default)]
    pub extension: String,
    #[serde(default)]
    pub site_id: i64,
    #[serde(default)]
    pub user_id: i64,
    #[serde(default)]
    pub passive_mode: bool,
    #[serde(default)]
    pub contacts: Vec<Contact>,
    #[serde(default)]
    pub journal: Vec<CallJournalEntry>,
    #[serde(default)]
    pub speed_dials: Vec<crate::models::SpeedDialEntry>,
    #[serde(default)]
    pub forward_number: String,
    #[serde(default)]
    pub forward_enabled: bool,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub seen_call_ids: Vec<String>,
    #[serde(default = "default_pickup_prefix")]
    pub pickup_prefix: String,
    #[serde(default = "default_background_on_close")]
    pub background_on_close: bool,
    #[serde(default = "default_audio_device")]
    pub audio_output_device: String,
    #[serde(default = "default_audio_device")]
    pub audio_input_device: String,
    #[serde(default = "default_audio_device")]
    pub audio_ring_device: String,
    #[serde(default)]
    pub voicemail_number: String,
    #[serde(default = "default_background_on_close")]
    pub show_hotkeys_sidebar: bool,
    #[serde(default = "default_background_on_close")]
    pub show_dialpad: bool,
    #[serde(default = "default_background_on_close")]
    pub sync_gnome_contacts: bool,
}

fn default_sip_port() -> u16 {
    5060
}

fn default_pickup_prefix() -> String {
    "*8".to_string()
}

fn default_background_on_close() -> bool {
    true
}

fn default_audio_device() -> String {
    "default".to_string()
}

impl Default for SwyxDataStore {
    fn default() -> Self {
        Self {
            provider: ProviderKind::Auto,
            sip_port: default_sip_port(),
            sip_realm: String::new(),
            server_ip: String::new(),
            user_name: String::new(),
            extension: String::new(),
            password: String::new(),
            site_id: 0,
            user_id: 0,
            passive_mode: false,
            contacts: initial_mock_contacts(),
            journal: initial_mock_journal(),
            speed_dials: Vec::new(),
            forward_number: String::new(),
            forward_enabled: false,
            seen_call_ids: Vec::new(),
            pickup_prefix: default_pickup_prefix(),
            background_on_close: true,
            audio_output_device: default_audio_device(),
            audio_input_device: default_audio_device(),
            audio_ring_device: default_audio_device(),
            voicemail_number: String::new(),
            show_hotkeys_sidebar: true,
            show_dialpad: true,
            sync_gnome_contacts: true,
        }
    }
}

/// Builds an offline, privacy-safe fictional dataset for `--demo` showcase & review mode.
pub fn build_demo_store() -> SwyxDataStore {
    use chrono::{Local, TimeDelta};
    use crate::models::{CallDirection, CallJournalEntry, PresenceState, SpeedDialEntry};

    let now = Local::now();
    let contacts = vec![
        Contact {
            id: "demo-1".into(),
            name: "Clara Weber".into(),
            extension: "201".into(),
            department: "Design".into(),
            presence: PresenceState::Available,
            phone: "+49 30 23125 201".into(),
            avatar_file_id: None,
        },
        Contact {
            id: "demo-2".into(),
            name: "Jonas Lindner".into(),
            extension: "202".into(),
            department: "Engineering".into(),
            presence: PresenceState::Busy,
            phone: "+49 30 23125 202".into(),
            avatar_file_id: None,
        },
        Contact {
            id: "demo-3".into(),
            name: "Maya Patel".into(),
            extension: "203".into(),
            department: "Product".into(),
            presence: PresenceState::Available,
            phone: "+49 30 23125 203".into(),
            avatar_file_id: None,
        },
        Contact {
            id: "demo-4".into(),
            name: "Lukas Berger".into(),
            extension: "205".into(),
            department: "Operations".into(),
            presence: PresenceState::Away,
            phone: "+49 30 23125 205".into(),
            avatar_file_id: None,
        },
        Contact {
            id: "demo-5".into(),
            name: "Sofia Rossi".into(),
            extension: "206".into(),
            department: "Support".into(),
            presence: PresenceState::Available,
            phone: "+49 30 23125 206".into(),
            avatar_file_id: None,
        },
        Contact {
            id: "eds-demo-1".into(),
            name: "Northwind Studio".into(),
            extension: String::new(),
            department: "GNOME Contacts".into(),
            presence: PresenceState::LoggedOut,
            phone: "+49 30 23125 192".into(),
            avatar_file_id: None,
        },
    ];

    let speed_dials = vec![
        SpeedDialEntry { id: 1, name: "Clara Weber".into(), number: "201".into(), state: 2, entity_id: 201 },
        SpeedDialEntry { id: 2, name: "Jonas Lindner".into(), number: "202".into(), state: 3, entity_id: 202 },
        SpeedDialEntry { id: 3, name: "Maya Patel".into(), number: "203".into(), state: 2, entity_id: 203 },
        SpeedDialEntry { id: 4, name: "Lukas Berger".into(), number: "205".into(), state: 5, entity_id: 205 },
    ];

    let journal = vec![
        CallJournalEntry {
            id: "demo-j1".into(),
            direction: CallDirection::Incoming,
            name: "Clara Weber".into(),
            number: "201".into(),
            called_target: "204".into(),
            timestamp: now - TimeDelta::minutes(18),
            connected_with: "204".into(),
            duration: "04:12".into(),
        },
        CallJournalEntry {
            id: "demo-j2".into(),
            direction: CallDirection::Missed,
            name: "+49 30 23125 192".into(),
            number: "+49 30 23125 192".into(),
            called_target: "204".into(),
            timestamp: now - TimeDelta::minutes(46),
            connected_with: String::new(),
            duration: "00:00".into(),
        },
        CallJournalEntry {
            id: "demo-j3".into(),
            direction: CallDirection::Outgoing,
            name: "Jonas Lindner".into(),
            number: "202".into(),
            called_target: "202".into(),
            timestamp: now - TimeDelta::hours(2),
            connected_with: "202".into(),
            duration: "02:38".into(),
        },
        CallJournalEntry {
            id: "demo-j4".into(),
            direction: CallDirection::Incoming,
            name: "Maya Patel".into(),
            number: "203".into(),
            called_target: "204".into(),
            timestamp: now - TimeDelta::hours(4),
            connected_with: "204".into(),
            duration: "06:15".into(),
        },
        CallJournalEntry {
            id: "demo-j5".into(),
            direction: CallDirection::Missed,
            name: "+1 415 555 0148".into(),
            number: "+1 415 555 0148".into(),
            called_target: "200".into(),
            timestamp: now - TimeDelta::days(1) - TimeDelta::hours(1),
            connected_with: String::new(),
            duration: "00:00".into(),
        },
        CallJournalEntry {
            id: "demo-j6".into(),
            direction: CallDirection::Outgoing,
            name: "Sofia Rossi".into(),
            number: "206".into(),
            called_target: "206".into(),
            timestamp: now - TimeDelta::days(1) - TimeDelta::hours(3),
            connected_with: "206".into(),
            duration: "01:44".into(),
        },
    ];

    SwyxDataStore {
        provider: ProviderKind::GenericSip,
        sip_port: 5060,
        sip_realm: "sip.example.org".into(),
        server_ip: "sip.example.org".into(),
        user_name: "Gawron, Martin".into(),
        extension: "204".into(),
        password: String::new(),
        site_id: 0,
        user_id: 0,
        passive_mode: false,
        contacts,
        journal,
        speed_dials,
        forward_number: String::new(),
        forward_enabled: false,
        seen_call_ids: vec!["demo-j5".into()],
        pickup_prefix: "*8".into(),
        background_on_close: false,
        audio_output_device: default_audio_device(),
        audio_input_device: default_audio_device(),
        audio_ring_device: default_audio_device(),
        voicemail_number: "900".into(),
        show_hotkeys_sidebar: true,
        show_dialpad: false,
        sync_gnome_contacts: false,
    }
}

pub struct SwyxBridge {
    config_dir: PathBuf,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum SwyxPassiveEvent {
    UserLoaded {
        user_name: String,
        extension: String,
        site_id: i64,
        user_id: i64,
        presence_state: i64,
    },
    ContactsLoaded(Vec<Contact>),
    JournalLoaded(Vec<CallJournalEntry>),
    SpeedDialsLoaded(Vec<crate::models::SpeedDialEntry>),
    PresenceUpdated {
        entity_id: i64,
        state: i64,
    },
    NameKeyStateChanged {
        key_index: i64,
        new_state: i64,
    },
    LineStateChanged {
        line_index: i64,
        line_state: i64,
    },
    LineDetailsChanged {
        line_id: i64,
        line_state: i64,
        peer_name: String,
        peer_number: String,
        is_incoming: bool,
        is_group_call: bool,
        called_name: String,
        called_extension: String,
    },
}

impl Default for SwyxBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl SwyxBridge {
    pub fn new() -> Self {
        let xdg_config = libadwaita::glib::user_config_dir();
        let config_dir = xdg_config.join("deskphone");
        let old_config_dir = xdg_config.join("swyxgtk");
        let _ = fs::create_dir_all(&config_dir);

        // Nahtlose Migration vorhandener Konfigurationen
        let old_data = old_config_dir.join("data.json");
        let new_data = config_dir.join("data.json");
        if old_data.exists() && !new_data.exists() {
            println!("[Deskphone] Migriere Konfigurationsdaten nach {:?}...", config_dir);
            let _ = fs::copy(&old_data, &new_data);
        }

        Self {
            config_dir,
        }
    }

    /// Loads persisted data or initializes from default PBX data if not yet present.
    /// Automatically migrates legacy plaintext passwords into GNOME Keyring (`oo7`).
    pub fn load_or_init_data(&self) -> SwyxDataStore {
        let file_path = self.config_dir.join("data.json");
        let mut store = if file_path.exists() {
            if let Ok(content) = fs::read_to_string(&file_path) {
                serde_json::from_str::<SwyxDataStore>(&content).unwrap_or_default()
            } else {
                SwyxDataStore::default()
            }
        } else {
            SwyxDataStore::default()
        };

        // Smart GNOME HIG Defaults erzwingen
        store.passive_mode = false;
        if store.pickup_prefix.is_empty() {
            store.pickup_prefix = "*8".to_string();
        }

        // Sichere Passwort-Verwaltung über GNOME Keyring / Flatpak Secret Portal (oo7)
        if !store.password.is_empty() {
            // Einmal-Migration: Klartext-Passwort sicher im Schlüsselbund speichern und aus data.json entfernen
            if crate::portals::store_password(&store.server_ip, &store.user_name, &store.password) {
                let mut sanitized = store.clone();
                sanitized.password.clear();
                if let Ok(json) = serde_json::to_string_pretty(&sanitized) {
                    let _ = fs::write(&file_path, json);
                }
            }
        } else if !store.server_ip.is_empty() && !store.user_name.is_empty() {
            if let Some(secret) = crate::portals::load_password(&store.server_ip, &store.user_name) {
                store.password = secret;
            }
        }

        if !file_path.exists() {
            self.save_data(&store);
        }
        store
    }

    /// Saves the current dataset safely (credentials stored encrypted in Keyring, omitted from data.json)
    pub fn save_data(&self, store: &SwyxDataStore) {
        let file_path = self.config_dir.join("data.json");
        let mut to_disk = store.clone();
        if !to_disk.password.is_empty()
            && crate::portals::store_password(&to_disk.server_ip, &to_disk.user_name, &to_disk.password)
        {
            to_disk.password.clear();
        }
        if let Ok(json) = serde_json::to_string_pretty(&to_disk) {
            let _ = fs::write(&file_path, json);
        }
    }

    /// Strict Passive Dial Check: Always asserts SafetyGate.
    /// Under NO circumstances will an external call be initiated.
    #[allow(dead_code)]
    pub fn assert_can_call(target: &str) -> Result<(), SafetyError> {
        SafetyGate::assert_can_call(target)
    }
}
