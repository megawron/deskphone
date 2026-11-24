use serde::{Deserialize, Serialize};

/// Credentials used for logging into Swyx CDS (ConfigDataStore)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

/// Authentication result returned by Swyx CDS ILogin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticationResult {
    pub access_token: String,
    pub refresh_token: String,
    pub user_id: i32,
    pub login_succeeded: bool,
}

/// SIP Credentials retrieved from Swyx CDS for a user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSipCredentials {
    pub user_id: i32,
    pub sip_user_name: String,
    pub sip_user_id: String,
    pub sip_realm: String,
    pub psk: Option<String>,
}

/// User telephone numbers and extensions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserNumberInfo {
    pub user_id: i32,
    pub internal_number: Option<String>,
    pub public_number: Option<String>,
    pub is_fax: bool,
}

/// Call journal / history entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallJournalEntry {
    pub id: i64,
    pub caller_number: String,
    pub called_number: String,
    pub caller_name: Option<String>,
    pub timestamp: String,
    pub duration_seconds: u32,
    pub direction: CallDirection,
    pub was_connected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallDirection {
    Inbound,
    Outbound,
    Missed,
    Unknown,
}

/// Voicemail message entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceMessageEntry {
    pub id: i64,
    pub caller_number: String,
    pub caller_name: Option<String>,
    pub timestamp: String,
    pub duration_seconds: u32,
    pub is_new: bool,
}

/// Rich Presence status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RichPresence {
    pub user_id: i32,
    pub status: PresenceStatus,
    pub status_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresenceStatus {
    Available = 0,
    Away = 1,
    DoNotDisturb = 2,
    Busy = 3,
    Offline = 4,
}

impl PresenceStatus {
    pub fn from_i32(val: i32) -> Self {
        match val {
            0 => PresenceStatus::Available,
            1 => PresenceStatus::Away,
            2 => PresenceStatus::DoNotDisturb,
            3 => PresenceStatus::Busy,
            _ => PresenceStatus::Offline,
        }
    }
}
