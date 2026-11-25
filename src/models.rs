use chrono::{DateTime, Datelike, Local, TimeDelta, Weekday};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallDirection {
    Incoming,
    Outgoing,
    Missed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallJournalEntry {
    pub id: String,
    pub direction: CallDirection,
    pub name: String,
    pub number: String,
    pub called_target: String,
    pub timestamp: DateTime<Local>,
    pub connected_with: String,
    pub duration: String,
}

impl CallJournalEntry {
    /// Smarte relative Zeitangabe für die Liste:
    /// - Gleicher Tag: nur Uhrzeit (z. B. "09:20")
    /// - Letzte 7 Tage: Wochentag + Uhrzeit (z. B. "Do. 16:38")
    /// - Älter als 1 Woche: Datum (z. B. "28.09.2026")
    pub fn formatted_time(&self) -> String {
        let now = Local::now();
        format_smart_time(&self.timestamp, &now)
    }

    /// Vollständige Zeitangabe für den Hover-Tooltip
    pub fn formatted_full(&self) -> String {
        format_full_time(&self.timestamp)
    }
}

pub fn format_smart_time(dt: &DateTime<Local>, now: &DateTime<Local>) -> String {
    let dt_date = dt.date_naive();
    let now_date = now.date_naive();

    if dt_date == now_date {
        dt.format("%H:%M").to_string()
    } else {
        let days_diff = (now_date - dt_date).num_days();
        if days_diff > 0 && days_diff < 7 {
            use crate::i18n::tr;
            let wday = match dt.weekday() {
                Weekday::Mon => tr("Mon"),
                Weekday::Tue => tr("Tue"),
                Weekday::Wed => tr("Wed"),
                Weekday::Thu => tr("Thu"),
                Weekday::Fri => tr("Fri"),
                Weekday::Sat => tr("Sat"),
                Weekday::Sun => tr("Sun"),
            };
            format!("{} {}", wday, dt.format("%H:%M"))
        } else {
            dt.format("%d.%m.%Y").to_string()
        }
    }
}

pub fn format_full_time(dt: &DateTime<Local>) -> String {
    use crate::i18n::tr;
    let wday = match dt.weekday() {
        Weekday::Mon => tr("Monday"),
        Weekday::Tue => tr("Tuesday"),
        Weekday::Wed => tr("Wednesday"),
        Weekday::Thu => tr("Thursday"),
        Weekday::Fri => tr("Friday"),
        Weekday::Sat => tr("Saturday"),
        Weekday::Sun => tr("Sunday"),
    };
    format!("{}, {}", wday, dt.format("%Y-%m-%d %H:%M"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresenceState {
    Available, // Green
    Busy,      // Red (Im Gespräch)
    Away,      // Yellow (Abwesend)
    LoggedOut, // Gray (Abgemeldet)
    Ringing,   // Amber / Pulse (Klingelt / Anruf wird signalisiert)
}

impl PresenceState {
    pub fn label(&self) -> String {
        use crate::i18n::tr;
        match self {
            Self::Available => tr("Available"),
            Self::Busy => tr("In a Call"),
            Self::Away => tr("Away"),
            Self::LoggedOut => tr("Offline"),
            Self::Ringing => tr("Ringing"),
        }
    }

    pub fn css_class(&self) -> &'static str {
        match self {
            Self::Available => "presence-available",
            Self::Busy => "presence-busy",
            Self::Away => "presence-away",
            Self::LoggedOut => "presence-offline",
            Self::Ringing => "presence-ringing",
        }
    }

    /// Swyx UserState (verwendet in PhoneBook curState, userAppearanceChanged.presenceState, GetLoggedInUser):
    /// 0 = LoggedOff -> Abgemeldet / Offline (Grau)
    /// 1 = LoggedOn -> Erreichbar (GRÜN)
    /// 2 = Active -> Im Gespräch / Telefoniert (ROT / BUSY)
    /// 3 = NoStatus -> Abgemeldet / Offline (Grau)
    /// 4 = Away -> Abwesend (GELB)
    /// 5 = DoNotDisturb -> Bitte nicht stören (ROT / BUSY)
    /// 6 = NotificationCall -> Im Anruf / Klingelt (RINGING / PULSE)
    pub fn from_swyx_user_state(code: i64) -> Self {
        match code {
            1 => Self::Available,
            2 | 5 => Self::Busy,
            4 => Self::Away,
            6 => Self::Ringing,
            _ => Self::LoggedOut,
        }
    }

    /// Swyx CLMgrNameKeyStates (verwendet in GetSpeedDials state / Nametasten):
    /// 0 = NKUnknown -> Abgemeldet / Offline (Grau)
    /// 1 = NKLoggedOut -> Abgemeldet / Offline (Grau)
    /// 2 = NKLoggedIn -> Erreichbar (GRÜN)
    /// 3 = NKBusy -> Im Gespräch / Telefoniert (ROT / BUSY)
    /// 4 = NKGroupCallNotification -> Im Anruf / Klingelt (RINGING / PULSE)
    /// 5 = NKAway -> Abwesend (GELB)
    /// 6 = NKDoNotDisturb -> Bitte nicht stören (ROT / BUSY)
    pub fn from_swyx_clmgr_state(code: i64) -> Self {
        match code {
            2 => Self::Available,
            3 | 6 => Self::Busy,
            4 => Self::Ringing,
            5 => Self::Away,
            _ => Self::LoggedOut,
        }
    }

    /// Wandelt PresenceState zurück in den Swyx CLMgrNameKeyState-Code für SpeedDials
    pub fn to_swyx_clmgr_state(&self) -> i64 {
        match self {
            Self::Available => 2,
            Self::Busy => 3,
            Self::Ringing => 4,
            Self::Away => 5,
            Self::LoggedOut => 1,
        }
    }

    /// Alias für from_swyx_user_state
    #[allow(dead_code)]
    pub fn from_swyx_code(code: i64) -> Self {
        Self::from_swyx_user_state(code)
    }
}

/// Formatiert den Wählstring für das Heranholen (Call Pickup)
pub fn format_pickup_number(prefix: &str, extension: &str) -> String {
    let clean_prefix = if prefix.is_empty() { "*8" } else { prefix };
    if clean_prefix.contains("{ext}") {
        clean_prefix.replace("{ext}", extension)
    } else if !extension.is_empty() {
        format!("{}{}", clean_prefix, extension)
    } else {
        clean_prefix.to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    pub id: String,
    pub name: String,
    pub extension: String,
    pub department: String,
    pub presence: PresenceState,
    pub phone: String,
    #[serde(default)]
    pub avatar_file_id: Option<String>,
}

impl Contact {
    /// Prüft, ob es sich um einen internen Swyx-Kollegen mit Durchwahl handelt
    pub fn is_internal(&self) -> bool {
        !self.extension.is_empty()
    }

    /// Returns true if this contact belongs to a PBX directory with live presence.
    /// External address-book contacts (e.g. GNOME Contacts / EDS) do not carry PBX presence.
    pub fn has_pbx_presence(&self) -> bool {
        !self.id.starts_with("eds-")
    }
}

/// Prüft, ob eine Rufnummer eine interne Swyx-Nebenstelle/Durchwahl ist
/// (typischerweise 2 bis 4 Ziffern, keine führende Amts-Null '0' oder Ländervorwahl '+')
pub fn is_internal_extension(number: &str) -> bool {
    let clean = number.trim();
    !clean.is_empty()
        && !clean.starts_with('0')
        && !clean.starts_with('+')
        && clean.len() <= 4
        && clean.chars().all(|c| c.is_ascii_digit())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveCall {
    pub id: String,
    pub caller_name: String,
    pub extension: String,
    pub full_number: String,
    pub duration_secs: u64,
    pub is_muted: bool,
    pub is_held: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeldCall {
    pub id: String,
    pub caller_name: String,
    pub extension: String,
    pub duration_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub sender: String,
    pub is_self: bool,
    pub text: String,
    pub timestamp: DateTime<Local>,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioMode {
    Headset,
    Speaker,
}

impl AudioMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Headset => "Headset",
            Self::Speaker => "Freisprechen",
        }
    }

    pub fn icon_name(&self) -> &'static str {
        match self {
            Self::Headset => "audio-headphones-symbolic",
            Self::Speaker => "audio-speakers-symbolic",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum TelephonyMode {
    Softphone, // PC-Audio (Headset / Freisprechen)
    Cti,       // Fernsteuerung eines Tischtelefons (z. B. Yealink, Snom, SwyxPhone)
}

#[allow(dead_code)]
impl TelephonyMode {
    pub fn label(&self) -> String {
        use crate::i18n::tr;
        match self {
            Self::Softphone => tr("Softphone (PC Audio)"),
            Self::Cti => tr("CTI (Desk Phone)"),
        }
    }

    pub fn icon_name(&self) -> &'static str {
        match self {
            Self::Softphone => "preferences-desktop-display-symbolic",
            Self::Cti => "phone-old-symbolic",
        }
    }

    pub fn tooltip(&self) -> String {
        use crate::i18n::tr;
        match self {
            Self::Softphone => tr("Telephony: PC Audio active • Click for CTI (Desk Phone)"),
            Self::Cti => tr("Telephony: CTI (Desk Phone) active • Click for PC Audio"),
        }
    }

    pub fn is_cti(&self) -> bool {
        matches!(self, Self::Cti)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeedDialEntry {
    pub id: i64,
    pub name: String,
    pub number: String,
    pub state: i64,
    pub entity_id: i64,
}

#[allow(dead_code)]
pub fn parse_swyx_speeddials(json: &serde_json::Value) -> Vec<SpeedDialEntry> {
    let entries = if let Some(arr) = json.as_array() {
        arr
    } else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for item in entries {
        let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let number = item.get("number").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if name.is_empty() && number.is_empty() {
            continue;
        }
        let id = item.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        let state = item.get("state").and_then(|v| v.as_i64()).unwrap_or(0);
        let entity_id = item.get("userEntityId").and_then(|v| v.as_i64()).unwrap_or(0);

        result.push(SpeedDialEntry {
            id,
            name,
            number,
            state,
            entity_id,
        });
    }
    result
}

/// Parst die echten PhoneBook-Einträge aus der Swyx ComSocket API
#[allow(dead_code)]
pub fn parse_swyx_phonebook(json: &serde_json::Value) -> Vec<Contact> {
    let entries = if let Some(arr) = json.get("entries").and_then(|e| e.as_array()) {
        arr
    } else if let Some(arr) = json.as_array() {
        arr
    } else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for item in entries {
        let raw_name = item.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let number = item.get("number").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if raw_name.is_empty() && number.is_empty() {
            continue;
        }

        let entity_id = item.get("entityId").and_then(|v| v.as_i64()).unwrap_or(0);
        let cur_state = item.get("curState").and_then(|v| v.as_i64()).unwrap_or(0);

        let presence = PresenceState::from_swyx_user_state(cur_state);

        // Bereinige Name: Wenn am Ende " 451" steht, optisch sauber darstellen
        let display_name = if !number.is_empty() && raw_name.ends_with(&format!(" {}", number)) {
            raw_name[..raw_name.len() - (number.len() + 1)].trim().to_string()
        } else {
            raw_name.clone()
        };

        let phone = if is_internal_extension(&number) {
            format!("DW {}", number)
        } else {
            number.clone()
        };

        let contact_id = if entity_id > 0 {
            format!("swyx-{}", entity_id)
        } else {
            format!("num-{}", number)
        };

        result.push(Contact {
            id: contact_id,
            name: if display_name.is_empty() { number.clone() } else { display_name },
            extension: number.clone(),
            department: if is_internal_extension(&number) { "Swyx Intern".into() } else { "Extern".into() },
            presence,
            phone,
            avatar_file_id: None,
        });
    }

    result
}

/// Parst die echten Ruflisteneinträge aus der Swyx ComSocket API
#[allow(dead_code)]
pub fn parse_swyx_journal(json: &serde_json::Value) -> Vec<CallJournalEntry> {
    let entries = if let Some(arr) = json.as_array() {
        arr
    } else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for item in entries {
        let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let kind = item.get("kind").and_then(|v| v.as_i64()).unwrap_or(0);
        let call_state = item.get("callState").and_then(|v| v.as_i64()).unwrap_or(0);
        let call_duration = item.get("callDuration").and_then(|v| v.as_i64()).unwrap_or(0);

        let raw_name = item.get("name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let dialed_name = item.get("dialedName").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let connected_name = item.get("connectedName").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();

        let raw_number = item.get("number").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        let dialed_number = item.get("dialedNumber").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();

        // Direction:
        // kind == 1 -> Outgoing
        // kind == 0 -> Incoming (wenn angenommen) oder Missed (wenn nicht angenommen / callDuration == 0)
        let direction = if kind == 1 {
            CallDirection::Outgoing
        } else if call_state != 0 || call_duration == 0 {
            CallDirection::Missed
        } else {
            CallDirection::Incoming
        };

        let number = if !raw_number.is_empty() {
            raw_number.clone()
        } else if !dialed_number.is_empty() {
            dialed_number.clone()
        } else {
            "Unbekannt".to_string()
        };

        let name = if !raw_name.is_empty() {
            raw_name
        } else if !dialed_name.is_empty() {
            dialed_name.clone()
        } else {
            number.clone()
        };

        let called_target = if !dialed_name.is_empty() {
            dialed_name
        } else if !dialed_number.is_empty() {
            dialed_number
        } else {
            "Eigene Leitung".to_string()
        };

        let connected_with = if !connected_name.is_empty() {
            connected_name
        } else {
            name.clone()
        };

        // Timestamp parsing: "2026-10-09T13:09:05"
        let timestamp = if let Some(time_str) = item.get("callStart").and_then(|v| v.as_str()) {
            chrono::NaiveDateTime::parse_from_str(time_str, "%Y-%m-%dT%H:%M:%S")
                .ok()
                .and_then(|naive| naive.and_local_timezone(Local).single())
                .unwrap_or_else(Local::now)
        } else {
            Local::now()
        };

        let duration = format!("{:02}:{:02}", call_duration / 60, call_duration % 60);

        result.push(CallJournalEntry {
            id,
            direction,
            name,
            number,
            called_target,
            timestamp,
            connected_with,
            duration,
        });
    }

    // Neueste Anrufe immer zuerst anzeigen
    result.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    result
}

pub fn initial_mock_journal() -> Vec<CallJournalEntry> {
    let now = Local::now();

    vec![
        CallJournalEntry {
            id: "1".into(),
            direction: CallDirection::Incoming,
            name: "Max Mustermann".into(),
            number: "101".into(),
            called_target: "100".into(),
            timestamp: now - TimeDelta::minutes(135),
            connected_with: "100".into(),
            duration: "01:37".into(),
        },
        CallJournalEntry {
            id: "2".into(),
            direction: CallDirection::Missed,
            name: "Unbekannt".into(),
            number: "0160 1234567".into(),
            called_target: "100".into(),
            timestamp: now - TimeDelta::minutes(145),
            connected_with: "Nicht angenommen".into(),
            duration: "00:00".into(),
        },
        CallJournalEntry {
            id: "3".into(),
            direction: CallDirection::Outgoing,
            name: "Erika Musterfrau".into(),
            number: "102".into(),
            called_target: "102".into(),
            timestamp: now - TimeDelta::days(1) + TimeDelta::hours(5),
            connected_with: "102".into(),
            duration: "00:45".into(),
        },
        CallJournalEntry {
            id: "4".into(),
            direction: CallDirection::Missed,
            name: "Zentrale".into(),
            number: "100".into(),
            called_target: "100".into(),
            timestamp: now - TimeDelta::days(1) + TimeDelta::hours(4),
            connected_with: "Nicht angenommen".into(),
            duration: "00:00".into(),
        },
        CallJournalEntry {
            id: "5".into(),
            direction: CallDirection::Incoming,
            name: "Thomas Weber".into(),
            number: "103".into(),
            called_target: "100".into(),
            timestamp: now - TimeDelta::days(1) + TimeDelta::hours(3),
            connected_with: "100".into(),
            duration: "03:12".into(),
        },
        CallJournalEntry {
            id: "6".into(),
            direction: CallDirection::Incoming,
            name: "Sarah Schmidt".into(),
            number: "104".into(),
            called_target: "100".into(),
            timestamp: now - TimeDelta::days(2) + TimeDelta::hours(1),
            connected_with: "100".into(),
            duration: "00:58".into(),
        },
    ]
}

pub fn initial_mock_contacts() -> Vec<Contact> {
    vec![
        Contact {
            id: "1".into(),
            name: "Max Mustermann".into(),
            extension: "101".into(),
            department: "Vertrieb".into(),
            presence: PresenceState::Available,
            phone: "+49 89 123456-101".into(),
            avatar_file_id: None,
        },
        Contact {
            id: "2".into(),
            name: "Erika Musterfrau".into(),
            extension: "102".into(),
            department: "Marketing".into(),
            presence: PresenceState::Busy,
            phone: "+49 89 123456-102".into(),
            avatar_file_id: None,
        },
        Contact {
            id: "3".into(),
            name: "Thomas Weber".into(),
            extension: "103".into(),
            department: "Entwicklung".into(),
            presence: PresenceState::Away,
            phone: "+49 89 123456-103".into(),
            avatar_file_id: None,
        },
        Contact {
            id: "4".into(),
            name: "Sarah Schmidt".into(),
            extension: "104".into(),
            department: "IT-Support".into(),
            presence: PresenceState::Available,
            phone: "+49 89 123456-104".into(),
            avatar_file_id: None,
        },
        Contact {
            id: "5".into(),
            name: "Zentrale".into(),
            extension: "100".into(),
            department: "Empfang".into(),
            presence: PresenceState::Available,
            phone: "+49 89 123456-100".into(),
            avatar_file_id: None,
        },
    ]
}


#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_format_smart_time_same_day() {
        let now = Local.with_ymd_and_hms(2026, 10, 9, 14, 30, 0).unwrap();
        let call_same_day = Local.with_ymd_and_hms(2026, 10, 9, 9, 15, 0).unwrap();
        assert_eq!(format_smart_time(&call_same_day, &now), "09:15");
    }

    #[test]
    fn test_format_smart_time_within_week() {
        let now = Local.with_ymd_and_hms(2026, 10, 9, 14, 30, 0).unwrap(); // Friday
        let call_yesterday = Local.with_ymd_and_hms(2026, 10, 8, 16, 45, 0).unwrap(); // Thursday
        assert_eq!(format_smart_time(&call_yesterday, &now), "Thu 16:45");

        let call_3_days_ago = Local.with_ymd_and_hms(2026, 10, 6, 11, 20, 0).unwrap(); // Tuesday
        assert_eq!(format_smart_time(&call_3_days_ago, &now), "Tue 11:20");
    }

    #[test]
    fn test_format_smart_time_older_than_week() {
        let now = Local.with_ymd_and_hms(2026, 10, 9, 14, 30, 0).unwrap();
        let call_older = Local.with_ymd_and_hms(2026, 9, 25, 8, 10, 0).unwrap();
        assert_eq!(format_smart_time(&call_older, &now), "25.09.2026");
    }

    #[test]
    fn test_telephony_mode_helpers() {
        let soft = TelephonyMode::Softphone;
        assert_eq!(soft.icon_name(), "preferences-desktop-display-symbolic");
        assert!(!soft.is_cti());
        assert!(soft.tooltip().contains("PC Audio"));

        let cti = TelephonyMode::Cti;
        assert_eq!(cti.icon_name(), "phone-old-symbolic");
        assert!(cti.is_cti());
        assert!(cti.tooltip().contains("CTI"));
    }

    #[test]
    fn test_is_internal_extension() {
        assert!(is_internal_extension("431"));
        assert!(is_internal_extension("102"));
        assert!(is_internal_extension("10"));
        assert!(is_internal_extension("1000"));

        // External phone numbers (fictional 555 test fixtures)
        assert!(!is_internal_extension("00491705550199"));
        assert!(!is_internal_extension("089123456"));
        assert!(!is_internal_extension("+49170123456"));
        assert!(!is_internal_extension(""));
        assert!(!is_internal_extension("12345")); // > 4 digits
    }

    #[test]
    fn test_classify_phone_number_subtitle() {
        assert_eq!(classify_phone_number_subtitle("+4917655501234", ""), "Mobile");
        assert_eq!(classify_phone_number_subtitle("01511234567", "100"), "Mobile → 100");
        assert_eq!(classify_phone_number_subtitle("431", ""), "Internal");
        assert_eq!(classify_phone_number_subtitle("+49305550123", ""), "Landline");
        assert_eq!(classify_phone_number_subtitle("+14155550148", "200"), "External → 200");
    }
}

/// Classifies an unknown phone number for the journal row subtitle so the raw number
/// is not duplicated on both title and subtitle lines.
pub fn classify_phone_number_subtitle(number: &str, called_target: &str) -> String {
    use crate::i18n::tr;
    let clean: String = number.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    let kind = if clean.starts_with("+4915")
        || clean.starts_with("+4916")
        || clean.starts_with("+4917")
        || clean.starts_with("004915")
        || clean.starts_with("004916")
        || clean.starts_with("004917")
        || clean.starts_with("015")
        || clean.starts_with("016")
        || clean.starts_with("017")
    {
        tr("Mobile")
    } else if is_internal_extension(&clean) {
        tr("Internal")
    } else if clean.starts_with("+49") || clean.starts_with("0049") || clean.starts_with('0') {
        tr("Landline")
    } else {
        tr("External")
    };

    let target = called_target.trim();
    if !target.is_empty() && target != number && target != clean {
        format!("{kind} → {target}")
    } else {
        kind
    }
}
