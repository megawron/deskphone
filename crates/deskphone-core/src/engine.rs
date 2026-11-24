use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use anyhow::{anyhow, Result};
use tokio::sync::{broadcast, Mutex, RwLock};
use tracing::info;

use crate::cds::{CallJournalEntry, RichPresence};
use crate::events::{PhonebookEntry, SoftphoneEvent};
use crate::provider::{AccountInfo, PbxProvider};
use crate::sip::ActiveCall;

/// Core Softphone Engine designed for embedded use in UI applications (GTK4, Slint, etc.)
/// and IPC daemon servers.
#[derive(Clone)]
pub struct SoftphoneEngine {
    provider: Arc<RwLock<Box<dyn PbxProvider>>>,
    event_tx: broadcast::Sender<SoftphoneEvent>,
    active_calls: Arc<Mutex<HashMap<String, ActiveCall>>>,
    dnd_enabled: Arc<AtomicBool>,
}

impl SoftphoneEngine {
    pub fn new(provider: Box<dyn PbxProvider>) -> Self {
        let (event_tx, _) = broadcast::channel(128);
        Self {
            provider: Arc::new(RwLock::new(provider)),
            event_tx,
            active_calls: Arc::new(Mutex::new(HashMap::new())),
            dnd_enabled: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Subscribes to softphone events (incoming calls, call state, registration, presence)
    pub fn subscribe(&self) -> broadcast::Receiver<SoftphoneEvent> {
        self.event_tx.subscribe()
    }

    /// Connects and registers to the PBX
    pub async fn start(&self, local_ip: &str, local_sip_port: u16) -> Result<()> {
        info!("Starting SoftphoneEngine...");
        let mut prov = self.provider.write().await;
        prov.connect(local_ip, local_sip_port).await?;

        // Spawn central SIP Inbound Dispatcher listener if provider supports incoming events
        if let Some(mut rx) = prov.take_event_receiver() {
            let event_tx = self.event_tx.clone();
            let dnd_enabled = self.dnd_enabled.clone();
            let provider = self.provider.clone();
            let active_calls = self.active_calls.clone();

            tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    match event {
                        crate::sip::SipEvent::IncomingCall { call_id, from, to, sdp: _ } => {
                            if dnd_enabled.load(Ordering::SeqCst) {
                                info!("DND is enabled: automatically rejecting incoming call {} with 486 Busy Here", call_id);
                                let prov = provider.read().await;
                                let _ = prov.reject(&call_id, 486).await;
                                continue;
                            }

                            let (caller_number, caller_name) = parse_caller_info(&from);
                            let (raw_called_num, called_name) = parse_caller_info(&to);
                            let called_number = if raw_called_num.is_empty() {
                                None
                            } else {
                                Some(raw_called_num)
                            };
                            info!(
                                "Dispatching IncomingCall event: call_id={}, caller={}, name={:?}, target={:?}",
                                call_id, caller_number, caller_name, called_number
                            );
                            let _ = event_tx.send(SoftphoneEvent::IncomingCall {
                                call_id,
                                caller_number,
                                caller_name,
                                called_number,
                                called_name,
                            });
                        }
                        crate::sip::SipEvent::CallEnded { call_id, reason } => {
                            info!("Call ended event: {} ({})", call_id, reason);
                            {
                                let mut calls = active_calls.lock().await;
                                if let Some(call) = calls.remove(&call_id) {
                                    if let Some(ref rtp) = call.rtp_session {
                                        rtp.stop();
                                    }
                                }
                            }
                            let _ = event_tx.send(SoftphoneEvent::CallEnded {
                                call_id,
                                reason,
                            });
                        }
                        crate::sip::SipEvent::RegistrationChanged { registered, message } => {
                            info!("SIP registration changed: registered={} ({})", registered, message);
                            let (account, realm) = {
                                let prov = provider.read().await;
                                match prov.get_account_info().await {
                                    Ok(i) => (i.sip_user, i.sip_realm),
                                    Err(_) => (String::new(), String::new()),
                                }
                            };
                            let _ = event_tx.send(SoftphoneEvent::RegistrationState {
                                registered,
                                account,
                                realm,
                                message,
                            });
                        }
                        crate::sip::SipEvent::CallRinging { call_id } => {
                            let _ = event_tx.send(SoftphoneEvent::CallRinging { call_id });
                        }
                        crate::sip::SipEvent::CallAnswered { call_id, .. } => {
                            let _ = event_tx.send(SoftphoneEvent::CallConnected {
                                call_id,
                                target_number: String::new(),
                            });
                        }
                        crate::sip::SipEvent::CallHoldState { call_id, on_hold } => {
                            let _ = event_tx.send(SoftphoneEvent::CallHoldState { call_id, on_hold });
                        }
                        crate::sip::SipEvent::CallTransferred { call_id, target } => {
                            let _ = event_tx.send(SoftphoneEvent::CallTransferred { call_id, target });
                        }
                        crate::sip::SipEvent::IncomingMessage { from, content } => {
                            let (sender_number, sender_name) = parse_caller_info(&from);
                            let from_display = if let Some(name) = sender_name {
                                format!("{} ({})", name, sender_number)
                            } else {
                                sender_number
                            };
                            let now = chrono::Utc::now().to_rfc3339();
                            info!("Dispatching ChatMessageReceived: from={}, text={}", from_display, content);
                            let _ = event_tx.send(SoftphoneEvent::ChatMessageReceived {
                                from: from_display,
                                message: content,
                                timestamp: now,
                            });
                        }
                        crate::sip::SipEvent::VoicemailSummary {
                            waiting,
                            new_messages,
                            old_messages,
                            account,
                        } => {
                            let _ = event_tx.send(SoftphoneEvent::VoicemailUpdate {
                                waiting,
                                new_messages,
                                old_messages,
                                account,
                            });
                        }
                    }
                }
            });
        }

        let info = prov.get_account_info().await?;
        let sip_ok = prov.sip_registered().unwrap_or(true);
        let _ = self.event_tx.send(SoftphoneEvent::RegistrationState {
            registered: sip_ok,
            account: info.sip_user.clone(),
            realm: info.sip_realm.clone(),
            message: if sip_ok {
                format!("Successfully registered with {}", prov.name())
            } else {
                "Telefonanlage erreichbar, aber SIP-Registrierung fehlgeschlagen - Telefonie nicht verfügbar (neuer Versuch läuft)".to_string()
            },
        });

        info!(
            "SoftphoneEngine started as {} (SIP registered: {})",
            info.sip_user, sip_ok
        );
        Ok(())
    }

    /// Dials an outbound phone number or extension and waits until the call is answered
    /// (or fails). Returns the Call-ID.
    pub async fn dial(&self, target_number: &str) -> Result<String> {
        let call_id = crate::sip::client::generate_call_id();
        self.dial_with_id(&call_id, target_number).await
    }

    /// Dials with a caller-provided Call-ID (see [`crate::sip::client::generate_call_id`]).
    ///
    /// Emits `CallRinging` while the destination rings, `CallConnected` when answered and
    /// `CallEnded` (with a human readable reason) if the call fails or is cancelled.
    /// While ringing, the call can be cancelled with [`SoftphoneEngine::hangup`].
    pub async fn dial_with_id(&self, call_id: &str, target_number: &str) -> Result<String> {
        let result = {
            let prov = self.provider.read().await;
            prov.dial_with_id(call_id, target_number).await
        };

        match result {
            Ok(call) => {
                let cid = call.call_id.clone();
                {
                    let mut calls = self.active_calls.lock().await;
                    calls.insert(cid.clone(), call);
                }
                let _ = self.event_tx.send(SoftphoneEvent::CallConnected {
                    call_id: cid.clone(),
                    target_number: target_number.to_string(),
                });
                Ok(cid)
            }
            Err(e) => {
                info!("Outgoing call {} to {} failed: {}", call_id, target_number, e);
                let _ = self.event_tx.send(SoftphoneEvent::CallEnded {
                    call_id: call_id.to_string(),
                    reason: e.to_string(),
                });
                Err(e)
            }
        }
    }

    /// Answers a pending incoming call by call_id
    pub async fn answer(&self, call_id: &str) -> Result<()> {
        let prov = self.provider.read().await;
        let call = prov.answer(call_id).await?;
        let target_number = call.target_number.clone();

        {
            let mut calls = self.active_calls.lock().await;
            calls.insert(call_id.to_string(), call);
        }

        let _ = self.event_tx.send(SoftphoneEvent::CallConnected {
            call_id: call_id.to_string(),
            target_number,
        });

        Ok(())
    }

    /// Rejects an incoming call with status code (e.g. 486 Busy Here or 603 Decline)
    pub async fn reject(&self, call_id: &str, code: u16) -> Result<()> {
        let prov = self.provider.read().await;
        prov.reject(call_id, code).await?;

        let _ = self.event_tx.send(SoftphoneEvent::CallEnded {
            call_id: call_id.to_string(),
            reason: format!("Call rejected with code {}", code),
        });

        Ok(())
    }

    /// Hangs up an active call, cancels a ringing outgoing call, or declines an incoming ringing call
    pub async fn hangup(&self, call_id: &str) -> Result<()> {
        let active = {
            let mut calls = self.active_calls.lock().await;
            calls.remove(call_id)
        };

        let prov = self.provider.read().await;
        if let Some(call) = active {
            prov.hangup(&call).await?;
        } else if prov.cancel_dial(call_id).await.unwrap_or(false) {
            // Ausgehender Ruf klingelt noch -> CANCEL; der Dial-Task meldet CallEnded
            info!("Outgoing call {} cancelled by user", call_id);
            return Ok(());
        } else {
            // Might be an incoming ringing call being declined by user
            let _ = prov.reject(call_id, 603).await;
        }

        let _ = self.event_tx.send(SoftphoneEvent::CallEnded {
            call_id: call_id.to_string(),
            reason: "User hangup".to_string(),
        });

        Ok(())
    }

    /// Puts an active call on hold
    pub async fn hold(&self, call_id: &str) -> Result<()> {
        let mut calls = self.active_calls.lock().await;
        let call = calls
            .get_mut(call_id)
            .ok_or_else(|| anyhow!("Call not found: {}", call_id))?;

        let prov = self.provider.read().await;
        prov.hold(call).await?;

        let _ = self.event_tx.send(SoftphoneEvent::CallHoldState {
            call_id: call_id.to_string(),
            on_hold: true,
        });

        Ok(())
    }

    /// Resumes a held call
    pub async fn unhold(&self, call_id: &str) -> Result<()> {
        let mut calls = self.active_calls.lock().await;
        let call = calls
            .get_mut(call_id)
            .ok_or_else(|| anyhow!("Call not found: {}", call_id))?;

        let prov = self.provider.read().await;
        prov.unhold(call).await?;

        let _ = self.event_tx.send(SoftphoneEvent::CallHoldState {
            call_id: call_id.to_string(),
            on_hold: false,
        });

        Ok(())
    }

    /// Transfers an active call to a new destination (Blind Transfer RFC 3515).
    /// A still ringing incoming call is deflected instead (302 redirect).
    pub async fn transfer(&self, call_id: &str, target: &str) -> Result<()> {
        let removed = {
            let mut calls = self.active_calls.lock().await;
            calls.remove(call_id)
        };
        let Some(mut call) = removed else {
            let prov = self.provider.read().await;
            prov.deflect(call_id, target)
                .await
                .map_err(|e| anyhow!("Call not found: {} ({})", call_id, e))?;
            let _ = self.event_tx.send(SoftphoneEvent::CallTransferred {
                call_id: call_id.to_string(),
                target: target.to_string(),
            });
            let _ = self.event_tx.send(SoftphoneEvent::CallEnded {
                call_id: call_id.to_string(),
                reason: format!("Deflected to {}", target),
            });
            return Ok(());
        };

        let prov = self.provider.read().await;
        if let Err(e) = prov.transfer(&mut call, target).await {
            // Gespräch bleibt bestehen, wenn die Übergabe scheitert
            self.active_calls.lock().await.insert(call_id.to_string(), call);
            return Err(e);
        }

        let _ = self.event_tx.send(SoftphoneEvent::CallTransferred {
            call_id: call_id.to_string(),
            target: target.to_string(),
        });
        let _ = self.event_tx.send(SoftphoneEvent::CallEnded {
            call_id: call_id.to_string(),
            reason: format!("Transferred to {}", target),
        });

        Ok(())
    }

    /// Performs an attended transfer (consultative transfer): connects call_id_1 with call_id_2 (RFC 3891)
    pub async fn attended_transfer(&self, call_id_1: &str, call_id_2: &str) -> Result<()> {
        let mut calls = self.active_calls.lock().await;

        let mut call1 = calls
            .remove(call_id_1)
            .ok_or_else(|| anyhow!("Call not found: {}", call_id_1))?;

        let call2 = match calls.remove(call_id_2) {
            Some(c) => c,
            None => {
                calls.insert(call_id_1.to_string(), call1);
                return Err(anyhow!("Consultation call not found: {}", call_id_2));
            }
        };

        let prov = self.provider.read().await;
        prov.attended_transfer(&mut call1, &call2).await?;

        let _ = self.event_tx.send(SoftphoneEvent::CallTransferred {
            call_id: call_id_1.to_string(),
            target: call2.target_number.clone(),
        });
        let _ = self.event_tx.send(SoftphoneEvent::CallEnded {
            call_id: call_id_1.to_string(),
            reason: format!("Attended transfer to {}", call2.target_number),
        });
        let _ = self.event_tx.send(SoftphoneEvent::CallEnded {
            call_id: call_id_2.to_string(),
            reason: "Attended transfer completed".to_string(),
        });

        Ok(())
    }

    /// Sets DND (Do Not Disturb) mode
    pub fn set_dnd(&self, enabled: bool) {
        self.dnd_enabled.store(enabled, Ordering::SeqCst);
        let _ = self.event_tx.send(SoftphoneEvent::DndState { enabled });
    }

    /// Ends all calls and removes the SIP registration at the PBX
    pub async fn shutdown(&self) -> Result<()> {
        let calls: Vec<ActiveCall> = {
            let mut map = self.active_calls.lock().await;
            map.drain().map(|(_, c)| c).collect()
        };
        let prov = self.provider.read().await;
        for call in calls {
            let _ = prov.hangup(&call).await;
        }
        prov.shutdown().await
    }

    /// Returns true if DND mode is enabled
    pub fn is_dnd(&self) -> bool {
        self.dnd_enabled.load(Ordering::SeqCst)
    }

    /// Sends an RFC 4733 DTMF digit ('0'-'9', '*', '#')
    pub async fn send_dtmf(&self, call_id: &str, digit: char) -> Result<()> {
        let calls = self.active_calls.lock().await;
        let call = calls
            .get(call_id)
            .ok_or_else(|| anyhow!("Call not found: {}", call_id))?;

        call.send_dtmf(digit).await?;

        let _ = self.event_tx.send(SoftphoneEvent::Dtmf {
            call_id: call_id.to_string(),
            digit,
        });

        Ok(())
    }

    /// Queries the account info
    pub async fn get_account_info(&self) -> Result<AccountInfo> {
        self.provider.read().await.get_account_info().await
    }

    /// Queries call history / journal
    pub async fn get_call_journal(&self) -> Result<Vec<CallJournalEntry>> {
        self.provider.read().await.get_call_journal().await
    }

    /// Queries user presence
    pub async fn get_presence(&self) -> Result<Option<RichPresence>> {
        self.provider.read().await.get_presence().await
    }

    /// Sets user presence on the PBX and syncs local DND state
    pub async fn set_user_presence(
        &self,
        status: crate::cds::PresenceStatus,
        status_text: &str,
    ) -> Result<()> {
        let is_dnd = matches!(
            status,
            crate::cds::PresenceStatus::DoNotDisturb | crate::cds::PresenceStatus::Busy
        );
        self.set_dnd(is_dnd);
        let res = self
            .provider
            .read()
            .await
            .set_presence(status, status_text)
            .await;
        let _ = self.event_tx.send(SoftphoneEvent::PresenceUpdate {
            user_id: "self".into(),
            status: format!("{:?}", status),
            status_text: status_text.to_string(),
        });
        res
    }

    /// Sets unconditional call forwarding on the PBX
    pub async fn set_call_forwarding(&self, enabled: bool, target: &str) -> Result<()> {
        let res = self
            .provider
            .read()
            .await
            .set_call_forwarding(enabled, target)
            .await;
        let _ = self.event_tx.send(SoftphoneEvent::CallForwardState {
            enabled,
            target: target.to_string(),
        });
        res
    }

    /// Queries voicemail messages from the PBX
    pub async fn get_voicemails(&self) -> Result<Vec<crate::cds::VoiceMessageEntry>> {
        self.provider.read().await.get_voicemails().await
    }

    /// Queries phonebook / contacts
    pub async fn get_phonebook(&self) -> Result<Vec<PhonebookEntry>> {
        self.provider.read().await.get_phonebook().await
    }

    /// Downloads a colleague avatar bitmap by Swyx FileId
    pub async fn download_avatar(&self, file_id: &str) -> Result<Option<Vec<u8>>> {
        self.provider.read().await.download_avatar(file_id).await
    }

    /// Downloads the authenticated user's own avatar bitmap
    pub async fn get_own_avatar(&self) -> Result<Option<Vec<u8>>> {
        self.provider.read().await.get_own_avatar().await
    }

    /// Uploads a new avatar bitmap for the authenticated user
    pub async fn upload_own_avatar(&self, image_bytes: Vec<u8>) -> Result<String> {
        self.provider
            .read()
            .await
            .upload_own_avatar(image_bytes)
            .await
    }

    /// Removes the authenticated user's avatar bitmap on the PBX
    pub async fn delete_own_avatar(&self) -> Result<()> {
        self.provider.read().await.delete_own_avatar().await
    }

    /// Sends an instant text message (SIP MESSAGE / RFC 3428) to target
    pub async fn send_chat_message(&self, target: &str, message: &str) -> Result<()> {
        let prov = self.provider.read().await;
        prov.send_message(target, message).await
    }

    /// Mutes or unmutes the microphone for all active calls
    pub async fn set_mute(&self, muted: bool) {
        let calls = self.active_calls.lock().await;
        for call in calls.values() {
            call.set_muted(muted);
        }
        let _ = self.event_tx.send(SoftphoneEvent::AudioState {
            input_device: "Default".into(),
            output_device: "Default".into(),
            muted,
        });
    }

    /// Returns whether any active call's microphone is muted
    pub async fn is_muted(&self) -> bool {
        let calls = self.active_calls.lock().await;
        calls.values().any(|c| c.is_muted())
    }
}

/// Helper to parse caller number and display name from SIP "From" header
fn parse_caller_info(from: &str) -> (String, Option<String>) {
    let caller_name = if let Some(idx) = from.find('<') {
        let name_part = from[..idx].trim().trim_matches('"').trim();
        if !name_part.is_empty() {
            Some(name_part.to_string())
        } else {
            None
        }
    } else {
        None
    };

    let caller_number = if let Some(sip_idx) = from.find("sip:") {
        let after_sip = &from[sip_idx + 4..];
        let end_idx = after_sip
            .find(|c| c == '@' || c == '>' || c == ';' || c == ':')
            .unwrap_or(after_sip.len());
        after_sip[..end_idx].to_string()
    } else {
        from.to_string()
    };

    (caller_number, caller_name)
}

