use std::sync::Arc;
use anyhow::{anyhow, Result};
use tokio::sync::RwLock;

use deskphone_core::engine::SoftphoneEngine;
pub use deskphone_core::events::SoftphoneEvent;
use deskphone_core::provider::asterisk::AsteriskProvider;
use deskphone_core::provider::generic_sip::GenericSipProvider;
use deskphone_core::provider::swyx::SwyxProvider;
use deskphone_core::provider::PbxProvider;

use crate::swyx_bridge::ProviderKind;

#[allow(dead_code)]
#[derive(Clone)]
pub struct EngineBridge {
    engine: Arc<RwLock<Option<Arc<SoftphoneEngine>>>>,
    active_provider: Arc<RwLock<ProviderKind>>,
    runtime: Arc<tokio::runtime::Runtime>,
}

/// Parses a user-entered server field such as `fritz.box`, `sip:sipgate.de:5060`,
/// or `192.168.1.10:5062` into `(host, effective_port)`.
pub fn parse_server_host_and_port(raw: &str, configured_port: u16) -> (String, u16) {
    let fallback_port = if configured_port == 0 { 5060 } else { configured_port };
    let trimmed = raw.trim();
    let no_scheme = trimmed
        .strip_prefix("sips:")
        .or_else(|| trimmed.strip_prefix("sip:"))
        .or_else(|| trimmed.strip_prefix("https://"))
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let host_part = no_scheme.split('/').next().unwrap_or(no_scheme).trim();

    if let Some((h, p_str)) = host_part.rsplit_once(':') {
        if !h.is_empty() && !h.contains(':') {
            if let Ok(parsed_port) = p_str.parse::<u16>() {
                if parsed_port > 0 {
                    return (h.trim().to_string(), parsed_port);
                }
            }
        }
    }
    (host_part.to_string(), fallback_port)
}

/// Probes well-known PBX management ports in parallel (fast 600 ms timeout) to
/// automatically identify SwyxWare (REST 9100 / CDS 8094) or Asterisk (AMI 5038),
/// falling back cleanly to Standard SIP (`GenericSip`).
pub async fn detect_provider_kind(server: &str) -> ProviderKind {
    use deskphone_core::provider::network::check_tcp_port;
    let (swyx_rest, swyx_cds, asterisk_ami) = tokio::join!(
        check_tcp_port(server, 9100, 600),
        check_tcp_port(server, 8094, 600),
        check_tcp_port(server, 5038, 600),
    );
    if swyx_rest || swyx_cds {
        ProviderKind::Swyx
    } else if asterisk_ami {
        ProviderKind::Asterisk
    } else {
        ProviderKind::GenericSip
    }
}

#[allow(dead_code)]
impl EngineBridge {
    pub fn new() -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        Ok(Self {
            engine: Arc::new(RwLock::new(None)),
            active_provider: Arc::new(RwLock::new(ProviderKind::Auto)),
            runtime: Arc::new(runtime),
        })
    }

    pub fn runtime(&self) -> &Arc<tokio::runtime::Runtime> {
        &self.runtime
    }

    pub fn resolved_provider(&self) -> ProviderKind {
        self.active_provider
            .try_read()
            .map(|g| *g)
            .unwrap_or(ProviderKind::Auto)
    }

    /// Loads GNOME Contacts / GNOME Online Accounts contacts read-only from Evolution Data Server.
    pub async fn fetch_gnome_contacts(&self) -> Vec<crate::models::Contact> {
        let (tx, rx) = async_channel::bounded(1);
        self.runtime.spawn(async move {
            let list = crate::gnome_contacts::fetch_eds_contacts().await;
            let _ = tx.send(list).await;
        });
        rx.recv().await.unwrap_or_default()
    }

    /// Asynchron verbindet und registriert die SoftphoneEngine am gewählten SIP-/PBX-Server
    pub fn connect(
        &self,
        provider_kind: ProviderKind,
        server: String,
        sip_port: u16,
        sip_realm: String,
        user: String,
        password: String,
        event_sender: async_channel::Sender<SoftphoneEvent>,
    ) {
        let engine_lock = self.engine.clone();
        let active_provider_lock = self.active_provider.clone();
        let (clean_server, effective_port) = parse_server_host_and_port(&server, sip_port);

        self.runtime.spawn(async move {
            // Cleanly shut down any existing engine before reconnecting
            let previous = {
                let mut lock = engine_lock.write().await;
                lock.take()
            };
            if let Some(old_engine) = previous {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    old_engine.shutdown(),
                )
                .await;
            }

            if clean_server.is_empty() || user.is_empty() {
                let _ = event_sender
                    .send(SoftphoneEvent::RegistrationState {
                        registered: false,
                        account: String::new(),
                        realm: String::new(),
                        message: crate::i18n::tr(
                            "Please enter server and username in Preferences.",
                        ),
                    })
                    .await;
                return;
            }

            let resolved_kind = match provider_kind {
                ProviderKind::Auto => {
                    let detected = detect_provider_kind(&clean_server).await;
                    println!(
                        "[Deskphone] Auto-detected provider for {}: {:?}",
                        clean_server, detected
                    );
                    detected
                }
                explicit => explicit,
            };
            *active_provider_lock.write().await = resolved_kind;

            let provider: Box<dyn PbxProvider> = match resolved_kind {
                ProviderKind::Swyx => {
                    let mut p = SwyxProvider::new(&clean_server, &user, &password);
                    p.sip_port = effective_port;
                    Box::new(p)
                }
                ProviderKind::GenericSip | ProviderKind::Auto => {
                    let mut p = GenericSipProvider::new(&clean_server, &user, &password)
                        .with_port(effective_port);
                    if !sip_realm.trim().is_empty() {
                        p = p.with_realm(sip_realm.trim());
                    }
                    Box::new(p)
                }
                ProviderKind::Asterisk => {
                    let mut p = AsteriskProvider::new(&clean_server, &user, &password);
                    p.sip_port = effective_port;
                    Box::new(p)
                }
            };

            let engine = Arc::new(SoftphoneEngine::new(provider));
            let mut sub = engine.subscribe();

            // Weiterleitung aller Softphone-Events an den GTK-Event-Kanal
            let fwd_sender = event_sender.clone();
            tokio::spawn(async move {
                while let Ok(ev) = sub.recv().await {
                    let _ = fwd_sender.send(ev).await;
                }
            });

            let local_ip =
                deskphone_core::provider::network::get_local_ip_for(&clean_server, effective_port);
            let mut local_sip_port = 50600;
            for p in 50600..50650 {
                if std::net::UdpSocket::bind((local_ip.as_str(), p)).is_ok() {
                    local_sip_port = p;
                    break;
                }
            }

            let _ = event_sender
                .send(SoftphoneEvent::RegistrationState {
                    registered: false,
                    account: user.clone(),
                    realm: clean_server.clone(),
                    message: crate::i18n::trf(
                        "Connecting to {server}…",
                        &[("server", &clean_server)],
                    ),
                })
                .await;

            match engine.start(&local_ip, local_sip_port).await {
                Ok(_) => {
                    println!(
                        "[Deskphone] SoftphoneEngine ({:?}) erfolgreich registriert an {}",
                        resolved_kind, clean_server
                    );
                    let mut lock = engine_lock.write().await;
                    *lock = Some(engine);
                }
                Err(e) => {
                    eprintln!("[Deskphone] Hinweis bei Engine-Registrierung: {}", e);
                    let err_s = e.to_string();
                    let _ = event_sender
                        .send(SoftphoneEvent::RegistrationState {
                            registered: false,
                            account: user,
                            realm: clean_server,
                            message: crate::i18n::trf(
                                "Connection error: {error}",
                                &[("error", &err_s)],
                            ),
                        })
                        .await;
                }
            }
        });
    }

    /// Erzeugt eine neue Call-ID für einen ausgehenden Anruf (vor dem Wählen bekannt,
    /// damit UI-Zustand und Engine-Events eindeutig zugeordnet werden können).
    pub fn new_call_id() -> String {
        deskphone_core::sip::client::generate_call_id()
    }

    /// Startet einen ausgehenden Anruf im Hintergrund und kehrt sofort zurück.
    /// Klingeln / Verbunden / Beendet (inkl. Fehlergrund) kommen als `SoftphoneEvent`s
    /// mit derselben `call_id`. Während es klingelt, bricht `hangup(call_id)` den Ruf ab.
    pub async fn dial(&self, call_id: &str, number: &str) -> Result<String> {
        let engine = {
            let lock = self.engine.read().await;
            lock.clone()
        }
        .ok_or_else(|| anyhow!("Softphone-Engine nicht verbunden"))?;

        let cid = call_id.to_string();
        let num = number.to_string();
        self.runtime.spawn(async move {
            if let Err(e) = engine.dial_with_id(&cid, &num).await {
                eprintln!("[Deskphone] Anruf an {} fehlgeschlagen: {}", num, e);
            }
        });
        Ok(call_id.to_string())
    }

    pub async fn pickup_call(&self, extension: &str, prefix: &str) -> Result<String> {
        let dial_str = crate::models::format_pickup_number(prefix, extension);
        let cid = Self::new_call_id();
        self.dial(&cid, &dial_str).await
    }

    /// Beendet laufende Gespräche und meldet die SIP-Registrierung ab (beim Programmende),
    /// damit die Telefonanlage keine Rufe mehr an diesen Client zustellt.
    pub fn shutdown_blocking(&self) {
        let engine = self.engine.clone();
        self.runtime.block_on(async move {
            let e = engine.read().await.clone();
            if let Some(e) = e {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(3), e.shutdown()).await;
            }
        });
    }

    pub async fn hangup(&self, call_id: &str) -> Result<()> {
        let engine = self.engine.clone();
        let cid = call_id.to_string();
        self.runtime.spawn(async move {
            let lock = engine.read().await;
            if let Some(ref e) = *lock {
                e.hangup(&cid).await
            } else {
                Ok(())
            }
        }).await?
    }

    pub async fn answer(&self, call_id: &str) -> Result<()> {
        let engine = self.engine.clone();
        let cid = call_id.to_string();
        self.runtime.spawn(async move {
            let lock = engine.read().await;
            if let Some(ref e) = *lock {
                e.answer(&cid).await
            } else {
                Err(anyhow!("Softphone-Engine nicht verbunden"))
            }
        }).await?
    }

    pub async fn reject(&self, call_id: &str, code: u16) -> Result<()> {
        let engine = self.engine.clone();
        let cid = call_id.to_string();
        self.runtime.spawn(async move {
            let lock = engine.read().await;
            if let Some(ref e) = *lock {
                e.reject(&cid, code).await
            } else {
                Ok(())
            }
        }).await?
    }

    pub async fn hold(&self, call_id: &str) -> Result<()> {
        let engine = self.engine.clone();
        let cid = call_id.to_string();
        self.runtime.spawn(async move {
            let lock = engine.read().await;
            if let Some(ref e) = *lock {
                e.hold(&cid).await
            } else {
                Err(anyhow!("Softphone-Engine nicht verbunden"))
            }
        }).await?
    }

    pub async fn unhold(&self, call_id: &str) -> Result<()> {
        let engine = self.engine.clone();
        let cid = call_id.to_string();
        self.runtime.spawn(async move {
            let lock = engine.read().await;
            if let Some(ref e) = *lock {
                e.unhold(&cid).await
            } else {
                Err(anyhow!("Softphone-Engine nicht verbunden"))
            }
        }).await?
    }

    pub async fn send_dtmf(&self, call_id: &str, digit: char) -> Result<()> {
        let lock = self.engine.read().await;
        if let Some(ref engine) = *lock {
            engine.send_dtmf(call_id, digit).await
        } else {
            // Wenn offline, trotzdem als Erfolg loggen für Mock-/UI-Interaktion
            println!("[Deskphone Mock] DTMF-Ton '{}' für Anruf '{}' simuliert", digit, call_id);
            Ok(())
        }
    }

    pub async fn set_mute(&self, muted: bool) {
        let lock = self.engine.read().await;
        if let Some(ref engine) = *lock {
            engine.set_mute(muted).await;
        }
    }

    pub async fn set_dnd(&self, enabled: bool) {
        let lock = self.engine.read().await;
        if let Some(ref engine) = *lock {
            engine.set_dnd(enabled);
        }
    }

    pub fn set_user_presence(&self, presence: crate::models::PresenceState) {
        let engine = self.engine.clone();
        self.runtime.spawn(async move {
            let status = match presence {
                crate::models::PresenceState::Available => {
                    deskphone_core::cds::PresenceStatus::Available
                }
                crate::models::PresenceState::Away => deskphone_core::cds::PresenceStatus::Away,
                crate::models::PresenceState::Busy | crate::models::PresenceState::Ringing => {
                    deskphone_core::cds::PresenceStatus::DoNotDisturb
                }
                crate::models::PresenceState::LoggedOut => {
                    deskphone_core::cds::PresenceStatus::Offline
                }
            };
            let lock = engine.read().await;
            if let Some(ref e) = *lock {
                let _ = e.set_user_presence(status, &presence.label()).await;
            }
        });
    }

    pub fn set_call_forwarding(&self, enabled: bool, target: String) {
        let engine = self.engine.clone();
        self.runtime.spawn(async move {
            let lock = engine.read().await;
            if let Some(ref e) = *lock {
                let _ = e.set_call_forwarding(enabled, &target).await;
            }
        });
    }

    pub async fn fetch_voicemails(&self) -> Result<Vec<deskphone_core::cds::VoiceMessageEntry>> {
        let engine = self.engine.clone();
        self.runtime
            .spawn(async move {
                let lock = engine.read().await;
                if let Some(ref e) = *lock {
                    e.get_voicemails().await
                } else {
                    Ok(Vec::new())
                }
            })
            .await?
    }

    pub fn list_output_devices() -> Vec<deskphone_core::rtp::AudioDeviceInfo> {
        deskphone_core::rtp::AudioDeviceManager::list_output_devices()
    }

    pub fn list_input_devices() -> Vec<deskphone_core::rtp::AudioDeviceInfo> {
        deskphone_core::rtp::AudioDeviceManager::list_input_devices()
    }

    pub async fn transfer(&self, call_id: &str, target: &str) -> Result<()> {
        let engine = self.engine.clone();
        let cid = call_id.to_string();
        let tgt = target.to_string();
        self.runtime
            .spawn(async move {
                let lock = engine.read().await;
                if let Some(ref e) = *lock {
                    e.transfer(&cid, &tgt).await
                } else {
                    println!(
                        "[Deskphone Mock] Transfer für '{}' an '{}' simuliert",
                        cid, tgt
                    );
                    Ok(())
                }
            })
            .await?
    }

    pub async fn attended_transfer(&self, call_id_1: &str, call_id_2: &str) -> Result<()> {
        let engine = self.engine.clone();
        let cid1 = call_id_1.to_string();
        let cid2 = call_id_2.to_string();
        self.runtime
            .spawn(async move {
                let lock = engine.read().await;
                if let Some(ref e) = *lock {
                    e.attended_transfer(&cid1, &cid2).await
                } else {
                    println!(
                        "[Deskphone Mock] Attended Transfer zwischen '{}' und '{}' simuliert",
                        cid1, cid2
                    );
                    Ok(())
                }
            })
            .await?
    }

    /// Sendet eine Swyx-Sofortnachricht (SIP MESSAGE / RFC 3428) an eine Ziel-Durchwahl
    pub async fn send_chat_message(&self, target: &str, message: &str) -> Result<()> {
        let engine = self.engine.clone();
        let tgt = target.to_string();
        let msg = message.to_string();
        self.runtime
            .spawn(async move {
                let lock = engine.read().await;
                if let Some(ref e) = *lock {
                    e.send_chat_message(&tgt, &msg).await
                } else {
                    Err(anyhow!("Softphone-Engine nicht verbunden"))
                }
            })
            .await?
    }

    pub async fn fetch_phonebook(&self) -> Result<Vec<crate::models::Contact>> {
        let engine = self.engine.clone();
        self.runtime.spawn(async move {
            let lock = engine.read().await;
            if let Some(ref e) = *lock {
                let entries = e.get_phonebook().await?;
                let contacts = entries
                    .into_iter()
                    .enumerate()
                    .map(|(i, entry)| {
                        let presence = if let Some(code) = entry.presence_status {
                            crate::models::PresenceState::from_swyx_user_state(code as i64)
                        } else {
                            crate::models::PresenceState::LoggedOut
                        };
                        let ext = entry.extension.unwrap_or_default();
                        let phone = if !entry.number.is_empty() {
                            entry.number
                        } else if !ext.is_empty() {
                            format!("DW {}", ext)
                        } else {
                            String::new()
                        };
                        crate::models::Contact {
                            id: entry.id.unwrap_or_else(|| format!("pb-idx-{}", i)),
                            name: entry.name,
                            extension: ext,
                            department: entry.department.unwrap_or_else(|| "Swyx Intern".into()),
                            presence,
                            phone,
                            avatar_file_id: entry.avatar_file_id,
                        }
                    })
                    .collect();
                Ok(contacts)
            } else {
                Ok(Vec::new())
            }
        }).await?
    }

    pub async fn fetch_avatar_bytes(&self, file_id: &str) -> Result<Option<Vec<u8>>> {
        let engine = self.engine.clone();
        let fid = file_id.to_string();
        self.runtime
            .spawn(async move {
                let lock = engine.read().await;
                if let Some(ref e) = *lock {
                    e.download_avatar(&fid).await
                } else {
                    Ok(None)
                }
            })
            .await?
    }

    pub async fn fetch_own_avatar_bytes(&self) -> Result<Option<Vec<u8>>> {
        let engine = self.engine.clone();
        self.runtime
            .spawn(async move {
                let lock = engine.read().await;
                if let Some(ref e) = *lock {
                    e.get_own_avatar().await
                } else {
                    Ok(None)
                }
            })
            .await?
    }

    pub async fn upload_own_avatar_bytes(&self, image_bytes: Vec<u8>) -> Result<String> {
        let engine = self.engine.clone();
        self.runtime
            .spawn(async move {
                let lock = engine.read().await;
                if let Some(ref e) = *lock {
                    e.upload_own_avatar(image_bytes).await
                } else {
                    Err(anyhow!("Softphone-Engine nicht verbunden"))
                }
            })
            .await?
    }

    pub async fn delete_own_avatar(&self) -> Result<()> {
        let engine = self.engine.clone();
        self.runtime
            .spawn(async move {
                let lock = engine.read().await;
                if let Some(ref e) = *lock {
                    e.delete_own_avatar().await
                } else {
                    Err(anyhow!("Softphone-Engine nicht verbunden"))
                }
            })
            .await?
    }

    pub async fn fetch_journal(&self) -> Result<Vec<crate::models::CallJournalEntry>> {
        let engine = self.engine.clone();
        self.runtime.spawn(async move {
            let lock = engine.read().await;
            if let Some(ref e) = *lock {
                let entries = e.get_call_journal().await?;
                let journal = entries
                    .into_iter()
                    .map(|e| {
                        let dir = match e.direction {
                            deskphone_core::cds::CallDirection::Inbound => {
                                if e.was_connected {
                                    crate::models::CallDirection::Incoming
                                } else {
                                    crate::models::CallDirection::Missed
                                }
                            }
                            deskphone_core::cds::CallDirection::Outbound => {
                                crate::models::CallDirection::Outgoing
                            }
                            _ => crate::models::CallDirection::Missed,
                        };
                        let duration = format!("{:02}:{:02}", e.duration_seconds / 60, e.duration_seconds % 60);
                        let timestamp = parse_ms_timestamp(&e.timestamp)
                            .and_then(chrono::DateTime::from_timestamp_millis)
                            .map(|dt| dt.with_timezone(&chrono::Local))
                            .unwrap_or_else(chrono::Local::now);

                        crate::models::CallJournalEntry {
                            id: format!("core-j-{}", e.id),
                            direction: dir,
                            name: e.caller_name.unwrap_or_else(|| e.caller_number.clone()),
                            number: e.caller_number,
                            called_target: e.called_number,
                            timestamp,
                            connected_with: String::new(),
                            duration,
                        }
                    })
                    .collect();
                Ok(journal)
            } else {
                Ok(Vec::new())
            }
        }).await?
    }
}

fn parse_ms_timestamp(s: &str) -> Option<i64> {
    let stripped = s.trim_start_matches("/Date(").trim_end_matches(")/");
    let digits: String = stripped.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<i64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_bridge_initialization_and_dtmf() {
        let bridge = EngineBridge::new().expect("Failed to create EngineBridge");
        bridge.runtime().block_on(async {
            // send_dtmf in offline/mock mode succeeds without panic
            let res = bridge.send_dtmf("test-call-1", '1').await;
            assert!(res.is_ok());

            let res_star = bridge.send_dtmf("test-call-1", '*').await;
            assert!(res_star.is_ok());

            let res_hash = bridge.send_dtmf("test-call-1", '#').await;
            assert!(res_hash.is_ok());

            // transfer in offline/mock mode succeeds
            let res_trans = bridge.transfer("test-call-1", "102").await;
            assert!(res_trans.is_ok());

            // mute and dnd work without panic
            bridge.set_mute(true).await;
            bridge.set_mute(false).await;
            bridge.set_dnd(true).await;
            bridge.set_dnd(false).await;

            // send_chat_message returns clean Err when engine is not connected
            let res_chat = bridge.send_chat_message("101", "Hallo via SIP MESSAGE").await;
            assert!(res_chat.is_err());
        });
    }
}
