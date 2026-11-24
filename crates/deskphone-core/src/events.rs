use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum SoftphoneEvent {
    /// Informs the UI of an incoming call
    IncomingCall {
        call_id: String,
        caller_number: String,
        caller_name: Option<String>,
        #[serde(default)]
        called_number: Option<String>,
        #[serde(default)]
        called_name: Option<String>,
    },
    /// The remote party is ringing
    CallRinging {
        call_id: String,
    },
    /// Call has been answered and media is flowing
    CallConnected {
        call_id: String,
        target_number: String,
    },
    /// Call has terminated
    CallEnded {
        call_id: String,
        reason: String,
    },
    /// Call hold state changed (held or resumed)
    CallHoldState {
        call_id: String,
        on_hold: bool,
    },
    /// Call transferred to another number
    CallTransferred {
        call_id: String,
        target: String,
    },
    /// Registration status changed
    RegistrationState {
        registered: bool,
        account: String,
        realm: String,
        message: String,
    },
    /// User or peer presence changed
    PresenceUpdate {
        user_id: String,
        status: String,
        status_text: String,
    },
    /// DTMF digit received or sent
    Dtmf {
        call_id: String,
        digit: char,
    },
    /// Audio device state
    AudioState {
        input_device: String,
        output_device: String,
        muted: bool,
    },
    /// DND (Do Not Disturb) mode state
    DndState {
        enabled: bool,
    },
    /// Instant chat message received (SIP MESSAGE / RFC 3428)
    ChatMessageReceived {
        from: String,
        message: String,
        timestamp: String,
    },
    /// Voicemail / Message Waiting Indicator (RFC 3842 MWI / Swyx Voice Box)
    VoicemailUpdate {
        waiting: bool,
        new_messages: u32,
        old_messages: u32,
        account: Option<String>,
    },
    /// Call Forwarding state update
    CallForwardState {
        enabled: bool,
        target: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", content = "params")]
pub enum SoftphoneCommand {
    /// Make an outbound call
    Dial { number: String },
    /// Send an instant chat message (SIP MESSAGE / RFC 3428)
    SendChatMessage { target: String, message: String },
    /// Answer an incoming call
    Answer { call_id: String },
    /// Hang up an active or incoming call
    Hangup { call_id: String },
    /// Put an active call on hold
    Hold { call_id: String },
    /// Resume an active call from hold
    Unhold { call_id: String },
    /// Transfer an active call to another extension or number (Blind Transfer)
    Transfer { call_id: String, target: String },
    /// Transfer an active call to another active consultation call (Attended Transfer RFC 3891)
    AttendedTransfer { call_id: String, target_call_id: String },
    /// Set DND (Do Not Disturb) mode
    SetDnd { enabled: bool },
    /// Set user rich presence state on the PBX
    SetPresence { status: i32, status_text: String },
    /// Set unconditional call forwarding on the PBX
    SetCallForward { enabled: bool, target: String },
    /// Send DTMF digit during call
    SendDtmf { call_id: String, digit: char },
    /// Mute or unmute microphone
    SetMute { muted: bool },
    /// Query account info
    GetInfo,
    /// Query call history
    GetJournal,
    /// Query phonebook
    GetPhonebook,
    /// Query voicemails
    GetVoicemails,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PhonebookEntry {
    pub name: String,
    pub number: String,
    pub extension: Option<String>,
    pub department: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub presence_status: Option<i32>,
    #[serde(default)]
    pub avatar_file_id: Option<String>,
}
