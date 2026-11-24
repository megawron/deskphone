use anyhow::Result;
use async_trait::async_trait;

use crate::cds::{CallJournalEntry, RichPresence};
use crate::sip::ActiveCall;

pub mod asterisk;
pub mod generic_sip;
pub mod network;
pub mod swyx;
pub mod template;

#[derive(Debug, Clone)]
pub struct ProbeCheck {
    pub port: u16,
    pub protocol: &'static str,
    pub service: &'static str,
    pub open: bool,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProbeReport {
    pub provider: &'static str,
    pub server: String,
    pub local_ip: String,
    pub checks: Vec<ProbeCheck>,
}

#[derive(Debug, Clone)]
pub struct AccountInfo {
    pub user_id: String,
    pub display_name: Option<String>,
    pub extensions: Vec<String>,
    pub sip_user: String,
    pub sip_realm: String,
}

/// Unified interface for any PBX / softphone backend provider.
///
/// Designed to be lightweight and modular so that different telephone systems
/// (Swyx, Generic SIP, Asterisk, FRITZ!Box, STARFACE, innovaphone, 3CX, etc.)
/// can be seamlessly integrated into the softphone core.
#[async_trait]
pub trait PbxProvider: Send + Sync {
    /// Human-readable name of this provider (e.g., "SwyxWare", "Generic SIP / FRITZ!Box / Asterisk")
    fn name(&self) -> &'static str;

    /// Non-invasive probe of ports, services, and reachability without authenticating
    async fn probe(&self, local_ip: &str) -> Result<ProbeReport>;

    /// Connect and authenticate to the PBX (SOAP, REST, and/or SIP Registration)
    async fn connect(&mut self, local_ip: &str, local_sip_port: u16) -> Result<()>;

    /// Fetch account details (user ID, extensions, assigned numbers)
    async fn get_account_info(&self) -> Result<AccountInfo>;

    /// Initiate an outbound phone call with live RTP audio
    async fn dial(&self, target_number: &str) -> Result<ActiveCall>;

    /// Initiate an outbound call using a caller-provided Call-ID (so the UI can correlate
    /// ringing/connected/ended events and cancel the call while it is still ringing).
    async fn dial_with_id(&self, call_id: &str, target_number: &str) -> Result<ActiveCall> {
        let _ = call_id;
        self.dial(target_number).await
    }

    /// Cancels a still ringing outgoing call (SIP CANCEL). Returns false if unknown.
    async fn cancel_dial(&self, _call_id: &str) -> Result<bool> {
        Ok(false)
    }

    /// Deflects a still ringing incoming call to another destination
    async fn deflect(&self, _call_id: &str, _target: &str) -> Result<()> {
        Err(anyhow::anyhow!("Call deflection is not supported by this provider"))
    }

    /// Current SIP registration state (None = unknown / not applicable)
    fn sip_registered(&self) -> Option<bool> {
        None
    }

    /// Removes the SIP registration (REGISTER Expires: 0) before shutting down
    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    /// Answers a pending incoming call by call_id
    async fn answer(&self, _call_id: &str) -> Result<ActiveCall> {
        Err(anyhow::anyhow!("Incoming calls not supported by this provider"))
    }

    /// Rejects an incoming call by call_id with status code (e.g. 486 Busy Here or 603 Decline)
    async fn reject(&self, _call_id: &str, _code: u16) -> Result<()> {
        Err(anyhow::anyhow!("Incoming call rejection not supported by this provider"))
    }

    /// Takes the SIP event receiver channel (if available) for event dispatching
    fn take_event_receiver(&mut self) -> Option<tokio::sync::mpsc::Receiver<crate::sip::SipEvent>> {
        None
    }

    /// Terminate an active phone call
    async fn hangup(&self, call: &ActiveCall) -> Result<()>;

    /// Fetch call history / journal (optional, defaults to empty vec if unsupported)
    async fn get_call_journal(&self) -> Result<Vec<CallJournalEntry>> {
        Ok(Vec::new())
    }

    /// Fetch user presence state (optional, defaults to None if unsupported)
    async fn get_presence(&self) -> Result<Option<RichPresence>> {
        Ok(None)
    }

    /// Sends an RFC 4733 DTMF tone during an active call
    async fn send_dtmf(&self, call: &ActiveCall, digit: char) -> Result<()> {
        call.send_dtmf(digit).await
    }

    /// Puts an active call on hold (re-INVITE a=sendonly and pause RTP)
    async fn hold(&self, _call: &mut ActiveCall) -> Result<()> {
        Err(anyhow::anyhow!("Hold is not supported by this provider"))
    }

    /// Resumes a held call (re-INVITE a=sendrecv and resume RTP)
    async fn unhold(&self, _call: &mut ActiveCall) -> Result<()> {
        Err(anyhow::anyhow!("Unhold is not supported by this provider"))
    }

    /// Transfers an active call to a new destination via SIP REFER (RFC 3515)
    async fn transfer(&self, _call: &mut ActiveCall, _target: &str) -> Result<()> {
        Err(anyhow::anyhow!("Call transfer is not supported by this provider"))
    }

    /// Performs an attended transfer connecting call1 to call2 (RFC 3891)
    async fn attended_transfer(&self, _call1: &mut ActiveCall, _call2: &ActiveCall) -> Result<()> {
        Err(anyhow::anyhow!("Attended transfer is not supported by this provider"))
    }

    /// Fetch central phonebook / directory contacts (optional, defaults to empty vec)
    async fn get_phonebook(&self) -> Result<Vec<crate::events::PhonebookEntry>> {
        Ok(Vec::new())
    }

    /// Sends an instant text message (SIP MESSAGE / RFC 3428) to target recipient
    async fn send_message(&self, _target: &str, _message: &str) -> Result<()> {
        Err(anyhow::anyhow!("Instant messaging is not supported by this provider"))
    }

    /// Updates user presence state on the PBX (optional, defaults to Ok)
    async fn set_presence(
        &self,
        _status: crate::cds::PresenceStatus,
        _status_text: &str,
    ) -> Result<()> {
        Ok(())
    }

    /// Updates unconditional call forwarding on the PBX (optional, defaults to Ok)
    async fn set_call_forwarding(&self, _enabled: bool, _target: &str) -> Result<()> {
        Ok(())
    }

    /// Fetch voicemail messages from the PBX (optional, defaults to empty vec)
    async fn get_voicemails(&self) -> Result<Vec<crate::cds::VoiceMessageEntry>> {
        Ok(Vec::new())
    }

    /// Download a contact avatar bitmap by provider file ID (optional)
    async fn download_avatar(&self, _file_id: &str) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }

    /// Download the authenticated user's own avatar bitmap (optional)
    async fn get_own_avatar(&self) -> Result<Option<Vec<u8>>> {
        Ok(None)
    }

    /// Upload a new avatar bitmap for the authenticated user (optional)
    async fn upload_own_avatar(&self, _image_bytes: Vec<u8>) -> Result<String> {
        Err(anyhow::anyhow!("Avatar upload is not supported by this provider"))
    }

    /// Remove the authenticated user's avatar bitmap on the PBX (optional)
    async fn delete_own_avatar(&self) -> Result<()> {
        Err(anyhow::anyhow!("Avatar removal is not supported by this provider"))
    }
}

