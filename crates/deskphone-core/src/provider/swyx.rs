use std::sync::Arc;
use std::time::{Duration, Instant};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::cds::{
    AuthenticationResult, CallJournalEntry, CdsClient, RichPresence, SwyxRestClient, UserNumberInfo,
    UserSipCredentials,
};
use crate::events::PhonebookEntry;
use crate::provider::network::{check_tcp_port, check_udp_sip};
use crate::provider::{AccountInfo, PbxProvider, ProbeCheck, ProbeReport};
use crate::sip::{ActiveCall, SipClient, SipConfig, SipEvent, SwyxLineContext};

/// Proactively refresh Swyx REST JWT access tokens after 45 minutes (server TTL is 60 min).
const REST_TOKEN_PROACTIVE_REFRESH_SECS: u64 = 2700;
/// Cache static global phonebook entries for 5 minutes while polling live employees every tick.
const GLOBAL_PHONEBOOK_CACHE_SECS: u64 = 300;

pub enum SwyxBackendMode {
    Rest(SwyxRestClient),
    Soap(CdsClient),
}

pub struct SwyxSession {
    pub mode: SwyxBackendMode,
    pub auth: RwLock<(AuthenticationResult, Instant)>,
    pub user_id: i32,
    pub sip_creds: UserSipCredentials,
    pub numbers: Vec<UserNumberInfo>,
    pub display_name: Option<String>,
    pub sip_client: Option<Arc<SipClient>>,
    #[allow(dead_code)]
    pub sip_events: Option<tokio::sync::mpsc::Receiver<SipEvent>>,
    pub global_phonebook_cache: RwLock<Option<(Vec<PhonebookEntry>, Instant)>>,
}

pub struct SwyxProvider {
    pub server_host: String,
    pub cds_port: u16,
    pub rest_port: u16,
    pub sip_port: u16,
    pub use_tls: bool,
    pub username: String,
    pub password: String,
    session: Option<SwyxSession>,
}

fn is_unauthorized_err(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    msg.contains("HTTP 401") || msg.contains("401 Unauthorized")
}

impl SwyxProvider {
    pub fn new(server_host: &str, username: &str, password: &str) -> Self {
        Self {
            server_host: server_host.to_string(),
            cds_port: 8094,
            rest_port: 9100,
            sip_port: 5060,
            use_tls: false,
            username: username.to_string(),
            password: password.to_string(),
            session: None,
        }
    }

    pub fn with_ports(mut self, cds_port: u16, sip_port: u16, use_tls: bool) -> Self {
        self.cds_port = cds_port;
        self.sip_port = sip_port;
        self.use_tls = use_tls;
        self
    }

    pub fn with_rest_port(mut self, rest_port: u16) -> Self {
        self.rest_port = rest_port;
        self
    }

    /// Returns a valid REST access token, proactively refreshing it before Swyx's 60-min JWT expiry.
    async fn valid_rest_token(&self, sess: &SwyxSession, rest: &SwyxRestClient) -> Result<String> {
        {
            let guard = sess.auth.read().await;
            if guard.1.elapsed() < Duration::from_secs(REST_TOKEN_PROACTIVE_REFRESH_SECS) {
                return Ok(guard.0.access_token.clone());
            }
        }
        self.force_refresh_rest_token(sess, rest, false).await
    }

    /// Re-acquires a fresh Swyx REST access token via login.
    async fn force_refresh_rest_token(
        &self,
        sess: &SwyxSession,
        rest: &SwyxRestClient,
        force: bool,
    ) -> Result<String> {
        let mut guard = sess.auth.write().await;
        if !force && guard.1.elapsed() < Duration::from_secs(REST_TOKEN_PROACTIVE_REFRESH_SECS) {
            return Ok(guard.0.access_token.clone());
        }
        info!("Refreshing Swyx REST AccessToken (force={})...", force);
        let new_auth = rest.login(&self.username, &self.password).await?;
        let token = new_auth.access_token.clone();
        *guard = (new_auth, Instant::now());
        info!("Swyx REST AccessToken successfully refreshed.");
        Ok(token)
    }

    async fn current_soap_token(sess: &SwyxSession) -> String {
        sess.auth.read().await.0.access_token.clone()
    }
}

#[async_trait]
impl PbxProvider for SwyxProvider {
    fn name(&self) -> &'static str {
        "SwyxWare / SwyxIt!"
    }

    async fn probe(&self, local_ip: &str) -> Result<ProbeReport> {
        let mut checks = Vec::new();

        // 1. Port 8094 TCP (CDS SOAP)
        let soap_ok = check_tcp_port(&self.server_host, self.cds_port, 1500).await;
        checks.push(ProbeCheck {
            port: self.cds_port,
            protocol: "TCP",
            service: "CDS SOAP WebService",
            open: soap_ok,
            note: if soap_ok {
                Some("SOAP endpoint open".into())
            } else {
                Some("Port closed or filtered".into())
            },
        });

        // 2. Port 9094 TCP (CDS Net.TCP WCF)
        let wcf_ok = check_tcp_port(&self.server_host, 9094, 1500).await;
        checks.push(ProbeCheck {
            port: 9094,
            protocol: "TCP",
            service: "CDS Net.TCP WCF Service",
            open: wcf_ok,
            note: Some("Native Windows SwyxIt! Port".into()),
        });

        // 3. Port 9100 TCP (CDS REST API HTTPS)
        let rest_ok = check_tcp_port(&self.server_host, 9100, 1500).await;
        checks.push(ProbeCheck {
            port: 9100,
            protocol: "TCP",
            service: "CDS Modern REST API",
            open: rest_ok,
            note: None,
        });

        // 4. Port 5060 UDP (SIP)
        let sip_ok = check_udp_sip(&self.server_host, self.sip_port, 1500).await;
        checks.push(ProbeCheck {
            port: self.sip_port,
            protocol: "UDP",
            service: "SIP Telephony Registrar",
            open: sip_ok,
            note: if sip_ok {
                Some("Active SIP response".into())
            } else {
                Some("No direct OPTIONS reply (normal for strict PBX)".into())
            },
        });

        Ok(ProbeReport {
            provider: self.name(),
            server: self.server_host.clone(),
            local_ip: local_ip.to_string(),
            checks,
        })
    }

    async fn connect(&mut self, local_ip: &str, local_sip_port: u16) -> Result<()> {
        info!("Connecting to SwyxServer at {}", self.server_host);

        // 1. Try modern Swyx REST API first (Port 9100)
        let rest = SwyxRestClient::new(self.server_host.clone(), self.rest_port);
        let (mode, auth, sip_creds, numbers, own_extension, display_name, site_id) = match rest
            .login(&self.username, &self.password)
            .await
        {
            Ok(auth) => {
                info!("Swyx REST login successful (User ID: {})", auth.user_id);
                let sip_realm = rest
                    .get_sip_realm()
                    .await
                    .unwrap_or_else(|| self.server_host.clone());
                let profile = rest.get_user_profile(auth.user_id, &auth.access_token).await.ok();
                let numbers = rest
                    .get_numbers(auth.user_id, &auth.access_token)
                    .await
                    .unwrap_or_default();
                for n in &numbers {
                    info!(
                        "Assigned Extension: {:?}, Public: {:?}",
                        n.internal_number, n.public_number
                    );
                }

                let own_ext = profile
                    .as_ref()
                    .and_then(|p| p.own_extension.clone())
                    .or_else(|| numbers.first().and_then(|n| n.internal_number.clone()))
                    .unwrap_or_default();
                let disp_name = profile
                    .as_ref()
                    .and_then(|p| p.display_name.clone())
                    .or_else(|| Some(self.username.clone()));
                let site_id = profile.as_ref().map(|p| p.site_id).unwrap_or(1);

                let sip_user = if !own_ext.is_empty() {
                    own_ext.clone()
                } else {
                    self.username.clone()
                };

                let sip_creds = match rest.get_sip_register_token(auth.user_id, &auth.access_token).await {
                    Ok(tok) => {
                        let psk = String::from_utf8(tok.preshared_key).ok();
                        UserSipCredentials {
                            user_id: auth.user_id,
                            sip_user_name: sip_user,
                            sip_user_id: tok.sip_register_id,
                            sip_realm,
                            psk,
                        }
                    }
                    Err(e) => {
                        info!("REST get_sip_register_token note: {} (using password)", e);
                        UserSipCredentials {
                            user_id: auth.user_id,
                            sip_user_name: sip_user,
                            sip_user_id: String::new(),
                            sip_realm,
                            psk: Some(self.password.clone()),
                        }
                    }
                };

                (
                    SwyxBackendMode::Rest(rest),
                    auth,
                    sip_creds,
                    numbers,
                    own_ext,
                    disp_name,
                    site_id,
                )
            }
            Err(rest_err) => {
                let rest_msg = rest_err.to_string();
                warn!("Swyx REST login failed on port {}: {}", self.rest_port, rest_msg);
                // If Swyx REST answered with an HTTP status (e.g. 401 Unauthorized / 403 / 400),
                // do NOT mask the real auth error by falling back to closed legacy SOAP port 8094.
                if rest_msg.contains("REST Login failed with HTTP") {
                    return Err(rest_err);
                }
                // Only attempt legacy CDS SOAP fallback if the SOAP port is actually reachable.
                if !check_tcp_port(&self.server_host, self.cds_port, 1000).await {
                    return Err(anyhow!(
                        "Swyx REST API (port {}) error: {}",
                        self.rest_port,
                        rest_msg
                    ));
                }
                info!(
                    "REST login unavailable ({}). Falling back to CDS SOAP on port {}...",
                    rest_msg, self.cds_port
                );
                let cds = CdsClient::new(self.server_host.clone(), self.cds_port, self.use_tls);
                let auth = cds
                    .login(&self.username, &self.password)
                    .await
                    .map_err(|soap_err| anyhow!("REST: {} | CDS SOAP: {}", rest_msg, soap_err))?;
                let sip_creds = cds
                    .get_sip_credentials(auth.user_id, &self.username, &auth.access_token)
                    .await?;
                info!(
                    "Fetched SIP credentials: user='{}', realm='{}', has_psk={}",
                    sip_creds.sip_user_name,
                    sip_creds.sip_realm,
                    sip_creds.psk.is_some()
                );
                let numbers = cds
                    .get_numbers(auth.user_id, &self.username, &auth.access_token)
                    .await
                    .unwrap_or_default();
                for n in &numbers {
                    info!(
                        "Assigned Extension: {:?}, Public: {:?}",
                        n.internal_number, n.public_number
                    );
                }
                let own_ext = numbers
                    .first()
                    .and_then(|n| n.internal_number.clone())
                    .unwrap_or_else(|| sip_creds.sip_user_name.clone());
                (
                    SwyxBackendMode::Soap(cds),
                    auth,
                    sip_creds,
                    numbers,
                    own_ext,
                    Some(self.username.clone()),
                    1,
                )
            }
        };

        // 2. Register SIP Client
        let sip_pass = sip_creds
            .psk
            .clone()
            .unwrap_or_else(|| self.password.clone());

        let sip_cfg = SipConfig {
            server_ip: self.server_host.clone(),
            server_port: self.sip_port,
            username: if !sip_creds.sip_user_id.is_empty() {
                sip_creds.sip_user_id.clone()
            } else if !sip_creds.sip_user_name.is_empty() {
                sip_creds.sip_user_name.clone()
            } else {
                self.username.clone()
            },
            password: sip_pass,
            realm: if !sip_creds.sip_realm.is_empty() {
                sip_creds.sip_realm.clone()
            } else {
                self.server_host.clone()
            },
            local_ip: local_ip.to_string(),
            local_port: local_sip_port,
        };

        let (sip_client, event_rx) = SipClient::new(sip_cfg).await?;
        sip_client.set_swyx_context(SwyxLineContext {
            own_extension: own_extension.clone(),
            display_name: display_name.clone(),
            user_id: auth.user_id,
            site_id,
        });
        info!(
            "Configured Swyx SIP line context: ext='{}', name={:?}, user_id={}, site_id={}, realm='{}'",
            own_extension, display_name, auth.user_id, site_id, sip_creds.sip_realm
        );
        let sip_client_arc = Arc::new(sip_client);

        match sip_client_arc.register(3600).await {
            Ok(_) => {
                if let Err(e) = sip_client_arc.subscribe_swyx_events(3600).await {
                    tracing::debug!("Swyx Event: x-ippbx subscription note: {}", e);
                }
                let _ = sip_client_arc.subscribe_voicemail().await;
            }
            Err(e) => {
                tracing::warn!(
                    "SIP registration failed: {} - incoming/outgoing calls are unavailable until it succeeds (retrying in background)",
                    e
                );
            }
        }
        // Registrierung dauerhaft aktuell halten (Refresh nach halber Laufzeit, Retry bei Fehlern)
        sip_client_arc.start_registration_keeper(3600);

        let user_id = auth.user_id;
        self.session = Some(SwyxSession {
            mode,
            auth: RwLock::new((auth, Instant::now())),
            user_id,
            sip_creds,
            numbers,
            display_name,
            sip_client: Some(sip_client_arc),
            sip_events: Some(event_rx),
            global_phonebook_cache: RwLock::new(None),
        });

        info!("SwyxProvider connected successfully!");
        Ok(())
    }

    async fn get_account_info(&self) -> Result<AccountInfo> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;

        let extensions: Vec<String> = sess
            .numbers
            .iter()
            .filter_map(|n| n.internal_number.clone())
            .collect();

        Ok(AccountInfo {
            user_id: sess.user_id.to_string(),
            display_name: sess.display_name.clone(),
            extensions,
            sip_user: sess.sip_creds.sip_user_name.clone(),
            sip_realm: sess.sip_creds.sip_realm.clone(),
        })
    }

    async fn dial(&self, target_number: &str) -> Result<ActiveCall> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;
        let sip = sess
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("SIP client not initialized"))?;
        sip.dial(target_number).await
    }

    async fn dial_with_id(&self, call_id: &str, target_number: &str) -> Result<ActiveCall> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;
        let sip = sess
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("SIP client not initialized"))?;
        if !sip.is_registered() {
            return Err(anyhow!(
                "Nicht an der Telefonanlage registriert (SIP) - bitte Verbindung prüfen"
            ));
        }
        sip.dial_with_call_id(call_id, target_number).await
    }

    async fn cancel_dial(&self, call_id: &str) -> Result<bool> {
        match self.session.as_ref().and_then(|s| s.sip_client.as_ref()) {
            Some(sip) => Ok(sip.cancel_outgoing(call_id).await),
            None => Ok(false),
        }
    }

    async fn deflect(&self, call_id: &str, target: &str) -> Result<()> {
        let sip = self
            .session
            .as_ref()
            .and_then(|s| s.sip_client.as_ref())
            .ok_or_else(|| anyhow!("SIP client not initialized"))?;
        sip.redirect_incoming_call(call_id, target).await
    }

    fn sip_registered(&self) -> Option<bool> {
        self.session
            .as_ref()
            .and_then(|s| s.sip_client.as_ref())
            .map(|sip| sip.is_registered())
    }

    async fn shutdown(&self) -> Result<()> {
        if let Some(sip) = self.session.as_ref().and_then(|s| s.sip_client.as_ref()) {
            let _ = sip.subscribe_swyx_events(0).await;
            sip.register(0).await?;
        }
        Ok(())
    }

    async fn answer(&self, call_id: &str) -> Result<ActiveCall> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;
        let sip = sess
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("SIP client not initialized"))?;
        sip.answer_incoming_call(call_id).await
    }

    async fn reject(&self, call_id: &str, code: u16) -> Result<()> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;
        let sip = sess
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("SIP client not initialized"))?;
        sip.reject_incoming_call(call_id, code).await
    }

    fn take_event_receiver(&mut self) -> Option<tokio::sync::mpsc::Receiver<SipEvent>> {
        self.session.as_mut().and_then(|s| s.sip_events.take())
    }

    async fn hangup(&self, call: &ActiveCall) -> Result<()> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;
        let sip = sess
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("SIP client not initialized"))?;
        sip.hangup(call).await
    }

    async fn hold(&self, call: &mut ActiveCall) -> Result<()> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;
        let sip = sess
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("SIP client not initialized"))?;
        sip.hold(call).await
    }

    async fn unhold(&self, call: &mut ActiveCall) -> Result<()> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;
        let sip = sess
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("SIP client not initialized"))?;
        sip.unhold(call).await
    }

    async fn transfer(&self, call: &mut ActiveCall, target: &str) -> Result<()> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;
        let sip = sess
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("SIP client not initialized"))?;
        sip.transfer(call, target).await
    }

    async fn attended_transfer(&self, call1: &mut ActiveCall, call2: &ActiveCall) -> Result<()> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;
        let sip = sess
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("SIP client not initialized"))?;
        sip.attended_transfer(call1, call2).await
    }

    async fn get_call_journal(&self) -> Result<Vec<CallJournalEntry>> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;
        match &sess.mode {
            SwyxBackendMode::Soap(cds) => {
                let token = Self::current_soap_token(sess).await;
                cds.get_call_journal(
                    sess.user_id,
                    &sess.sip_creds.sip_user_name,
                    &token,
                )
                .await
            }
            SwyxBackendMode::Rest(rest) => {
                let token = self.valid_rest_token(sess, rest).await?;
                match rest.get_calls(sess.user_id, &token).await {
                    Ok(calls) => Ok(calls),
                    Err(e) if is_unauthorized_err(&e) => {
                        let fresh = self.force_refresh_rest_token(sess, rest, true).await?;
                        rest.get_calls(sess.user_id, &fresh).await
                    }
                    Err(e) => Err(e),
                }
            }
        }
    }

    async fn get_presence(&self) -> Result<Option<RichPresence>> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;
        match &sess.mode {
            SwyxBackendMode::Soap(cds) => {
                let token = Self::current_soap_token(sess).await;
                let presence = cds
                    .get_presence(
                        sess.user_id,
                        &sess.sip_creds.sip_user_name,
                        &token,
                    )
                    .await?;
                Ok(Some(presence))
            }
            SwyxBackendMode::Rest(_) => Ok(Some(RichPresence {
                user_id: sess.user_id,
                status: crate::cds::PresenceStatus::Available,
                status_text: "Available".into(),
            })),
        }
    }

    async fn get_phonebook(&self) -> Result<Vec<crate::events::PhonebookEntry>> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;

        if let SwyxBackendMode::Rest(rest) = &sess.mode {
            let mut token = self.valid_rest_token(sess, rest).await?;

            // 1. Live employees & presence (fast ~12ms call, ALWAYS fetched fresh!)
            let emps = match rest.get_employees(&token).await {
                Ok(list) => list,
                Err(e) if is_unauthorized_err(&e) => {
                    token = self.force_refresh_rest_token(sess, rest, true).await?;
                    rest.get_employees(&token).await?
                }
                Err(e) => {
                    warn!("Swyx REST get_employees failed: {}", e);
                    return Err(e);
                }
            };

            // 2. Static global phonebook (cached for 5 minutes so 2s presence polling stays lightweight)
            let pb_entries = {
                let cached = sess.global_phonebook_cache.read().await;
                cached
                    .as_ref()
                    .filter(|(_, ts)| ts.elapsed() < Duration::from_secs(GLOBAL_PHONEBOOK_CACHE_SECS))
                    .map(|(list, _)| list.clone())
            };
            let pb = match pb_entries {
                Some(list) => list,
                None => match rest.get_phonebook(&token).await {
                    Ok(list) => {
                        *sess.global_phonebook_cache.write().await = Some((list.clone(), Instant::now()));
                        list
                    }
                    Err(e) if is_unauthorized_err(&e) => {
                        let fresh = self.force_refresh_rest_token(sess, rest, true).await?;
                        let list = rest.get_phonebook(&fresh).await.unwrap_or_default();
                        *sess.global_phonebook_cache.write().await = Some((list.clone(), Instant::now()));
                        list
                    }
                    Err(e) => {
                        warn!("Swyx REST get_phonebook failed: {}", e);
                        Vec::new()
                    }
                },
            };

            let mut all_entries = Vec::with_capacity(emps.len() + pb.len());
            // Employees FIRST so real Swyx user IDs and live presence always take priority
            all_entries.extend(emps);
            all_entries.extend(pb);
            if !all_entries.is_empty() {
                return Ok(all_entries);
            }
        }

        // Return empty vector when no remote phonebook entries exist,
        // allowing the frontend to preserve its cached contact list.
        Ok(Vec::new())
    }

    async fn send_message(&self, target: &str, message: &str) -> Result<()> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;
        let sip_client = sess
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("SIP client not available"))?;

        // Swyx-UserID (EmployeeId) des Empfängers auflösen
        // Format prüfen: z. B. "451;id=swyx-147", "swyx-147" oder einfache Durchwahl "451"
        let (target_clean, dest_id_hint) = if let Some((num, id_part)) = target.split_once(';') {
            let num_clean: String = num.chars().filter(|c| !c.is_whitespace()).collect();
            let parsed = id_part
                .strip_prefix("id=")
                .or_else(|| id_part.strip_prefix("swyx_id="))
                .unwrap_or(id_part)
                .strip_prefix("swyx-")
                .and_then(|s| s.parse::<i32>().ok())
                .or_else(|| id_part.parse::<i32>().ok());
            (num_clean, parsed)
        } else {
            let clean: String = target.chars().filter(|c| !c.is_whitespace()).collect();
            let direct_id = clean.strip_prefix("swyx-").and_then(|s| s.parse::<i32>().ok());
            (clean, direct_id)
        };

        let mut dest_user_id = dest_id_hint;

        // 1. Wenn noch nicht bekannt: REST-API versuchen (sofern verfügbar)
        if dest_user_id.is_none() {
            if let SwyxBackendMode::Rest(rest) = &sess.mode {
                if let Ok(token) = self.valid_rest_token(sess, rest).await {
                    let emps_res = match rest.get_employees(&token).await {
                        Ok(list) => Ok(list),
                        Err(e) if is_unauthorized_err(&e) => {
                            match self.force_refresh_rest_token(sess, rest, true).await {
                                Ok(fresh) => rest.get_employees(&fresh).await,
                                Err(re) => Err(re),
                            }
                        }
                        Err(e) => Err(e),
                    };
                    if let Ok(emps) = emps_res {
                        dest_user_id = emps
                            .iter()
                            .find(|e| {
                                e.extension.as_deref() == Some(target_clean.as_str())
                                    || e.number.replace(' ', "") == target_clean
                            })
                            .and_then(|e| e.id.as_deref())
                            .and_then(|id| id.strip_prefix("swyx-"))
                            .and_then(|id| id.parse::<i32>().ok());
                    }
                }
            }
        }

        // 2. Fallback: Lokale Kontaktdaten (~/.config/deskphone/data.json)
        if dest_user_id.is_none() {
            dest_user_id = resolve_dest_user_id_from_local_data(&target_clean);
        }

        info!(
            "Sending Swyx IM to {} (dest user id: {:?}, own user id: {})",
            target_clean, dest_user_id, sess.user_id
        );

        sip_client
            .send_swyx_instant_message(&target_clean, message, Some(sess.user_id), dest_user_id)
            .await
    }

    async fn set_presence(
        &self,
        status: crate::cds::PresenceStatus,
        status_text: &str,
    ) -> Result<()> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;

        // 1. Update via SIP PUBLISH (standard SIP compatibility)
        if let Some(ref sip) = sess.sip_client {
            let is_open = matches!(status, crate::cds::PresenceStatus::Available);
            let _ = sip.publish_presence(is_open, status_text).await;
        }

        // 2. Update via Swyx CDS REST or SOAP
        match &sess.mode {
            SwyxBackendMode::Rest(rest) => {
                let token = self.valid_rest_token(sess, rest).await?;
                match rest.set_presence(sess.user_id, &token, status, status_text).await {
                    Ok(()) => Ok(()),
                    Err(e) if is_unauthorized_err(&e) => {
                        let fresh = self.force_refresh_rest_token(sess, rest, true).await?;
                        rest.set_presence(sess.user_id, &fresh, status, status_text).await
                    }
                    Err(e) => Err(e),
                }
            }
            SwyxBackendMode::Soap(cds) => {
                let token = Self::current_soap_token(sess).await;
                cds.set_presence(
                    sess.user_id,
                    status,
                    status_text,
                    &sess.sip_creds.sip_user_name,
                    &token,
                )
                .await
            }
        }
    }

    async fn set_call_forwarding(&self, enabled: bool, target: &str) -> Result<()> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;

        match &sess.mode {
            SwyxBackendMode::Rest(rest) => {
                let token = self.valid_rest_token(sess, rest).await?;
                match rest.set_call_forwarding(sess.user_id, &token, enabled, target).await {
                    Ok(()) => Ok(()),
                    Err(e) if is_unauthorized_err(&e) => {
                        let fresh = self.force_refresh_rest_token(sess, rest, true).await?;
                        rest.set_call_forwarding(sess.user_id, &fresh, enabled, target).await
                    }
                    Err(e) => Err(e),
                }
            }
            SwyxBackendMode::Soap(cds) => {
                let token = Self::current_soap_token(sess).await;
                cds.set_call_forwarding(
                    sess.user_id,
                    enabled,
                    target,
                    &sess.sip_creds.sip_user_name,
                    &token,
                )
                .await
            }
        }
    }

    async fn get_voicemails(&self) -> Result<Vec<crate::cds::VoiceMessageEntry>> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;

        match &sess.mode {
            SwyxBackendMode::Rest(rest) => {
                let token = self.valid_rest_token(sess, rest).await?;
                match rest.get_voicemails(sess.user_id, &token).await {
                    Ok(vms) => Ok(vms),
                    Err(e) if is_unauthorized_err(&e) => {
                        let fresh = self.force_refresh_rest_token(sess, rest, true).await?;
                        rest.get_voicemails(sess.user_id, &fresh).await
                    }
                    Err(e) => Err(e),
                }
            }
            SwyxBackendMode::Soap(cds) => {
                let token = Self::current_soap_token(sess).await;
                cds.get_voice_messages(
                    sess.user_id,
                    &sess.sip_creds.sip_user_name,
                    &token,
                )
                .await
            }
        }
    }

    async fn download_avatar(&self, file_id: &str) -> Result<Option<Vec<u8>>> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;

        match &sess.mode {
            SwyxBackendMode::Rest(rest) => {
                let token = self.valid_rest_token(sess, rest).await?;
                match rest.get_file(file_id, &token).await {
                    Ok(bytes) => Ok(bytes),
                    Err(e) if is_unauthorized_err(&e) => {
                        let fresh = self.force_refresh_rest_token(sess, rest, true).await?;
                        rest.get_file(file_id, &fresh).await
                    }
                    Err(e) => Err(e),
                }
            }
            SwyxBackendMode::Soap(_) => Ok(None),
        }
    }

    async fn get_own_avatar(&self) -> Result<Option<Vec<u8>>> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;

        match &sess.mode {
            SwyxBackendMode::Rest(rest) => {
                let token = self.valid_rest_token(sess, rest).await?;
                match rest.get_user_avatar(sess.user_id, &token).await {
                    Ok(bytes) => Ok(bytes),
                    Err(e) if is_unauthorized_err(&e) => {
                        let fresh = self.force_refresh_rest_token(sess, rest, true).await?;
                        rest.get_user_avatar(sess.user_id, &fresh).await
                    }
                    Err(e) => Err(e),
                }
            }
            SwyxBackendMode::Soap(_) => Ok(None),
        }
    }

    async fn upload_own_avatar(&self, image_bytes: Vec<u8>) -> Result<String> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;

        match &sess.mode {
            SwyxBackendMode::Rest(rest) => {
                let token = self.valid_rest_token(sess, rest).await?;
                match rest
                    .upload_user_avatar(sess.user_id, &token, image_bytes.clone())
                    .await
                {
                    Ok(fid) => Ok(fid),
                    Err(e) if is_unauthorized_err(&e) => {
                        let fresh = self.force_refresh_rest_token(sess, rest, true).await?;
                        rest.upload_user_avatar(sess.user_id, &fresh, image_bytes)
                            .await
                    }
                    Err(e) => Err(e),
                }
            }
            SwyxBackendMode::Soap(_) => Err(anyhow!("Avatar-Upload erfordert die Swyx REST API")),
        }
    }

    async fn delete_own_avatar(&self) -> Result<()> {
        let sess = self
            .session
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected"))?;

        match &sess.mode {
            SwyxBackendMode::Rest(rest) => {
                let mut token = self.valid_rest_token(sess, rest).await?;
                let emps = match rest.get_employees(&token).await {
                    Ok(list) => list,
                    Err(e) if is_unauthorized_err(&e) => {
                        token = self.force_refresh_rest_token(sess, rest, true).await?;
                        rest.get_employees(&token).await?
                    }
                    Err(e) => return Err(e),
                };

                let own_id = format!("swyx-{}", sess.user_id);
                let file_id = emps
                    .into_iter()
                    .find(|e| e.id.as_deref() == Some(own_id.as_str()))
                    .and_then(|e| e.avatar_file_id);

                let Some(fid) = file_id else {
                    return Ok(());
                };

                match rest.delete_file(&fid, &token).await {
                    Ok(_) => Ok(()),
                    Err(e) if is_unauthorized_err(&e) => {
                        let fresh = self.force_refresh_rest_token(sess, rest, true).await?;
                        rest.delete_file(&fid, &fresh).await?;
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            SwyxBackendMode::Soap(_) => Err(anyhow!("Avatar-Entfernung erfordert die Swyx REST API")),
        }
    }
}

/// Fallback-Helfer: Versucht die Swyx-UserID anhand von Durchwahl oder Rufnummer
/// aus der lokalen Konfigurationsdatei (~/.config/deskphone/data.json) aufzulösen.
fn resolve_dest_user_id_from_local_data(target_clean: &str) -> Option<i32> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::Path::new(&home).join(".config/deskphone/data.json");
    let content = std::fs::read_to_string(path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    let contacts = val.get("contacts")?.as_array()?;
    for c in contacts {
        let ext = c.get("extension").and_then(|v| v.as_str()).unwrap_or("");
        let phone = c.get("phone").and_then(|v| v.as_str()).unwrap_or("").replace(' ', "");
        let id_str = c.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if (!ext.is_empty() && ext == target_clean)
            || (!phone.is_empty() && phone == target_clean)
            || (!id_str.is_empty() && id_str == target_clean)
        {
            if let Some(num) = id_str.strip_prefix("swyx-").and_then(|s| s.parse::<i32>().ok()) {
                return Some(num);
            }
            if let Ok(num) = id_str.parse::<i32>() {
                return Some(num);
            }
        }
    }
    None
}

