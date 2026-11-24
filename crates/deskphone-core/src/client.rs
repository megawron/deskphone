use std::sync::Arc;
use anyhow::{anyhow, Result};
use tokio::sync::Mutex;
use tracing::info;

use crate::cds::{
    AuthenticationResult, CallJournalEntry, CdsClient, RichPresence, UserNumberInfo,
    UserSipCredentials,
};
use crate::sip::{ActiveCall, SipClient, SipConfig, SipEvent};

pub struct SwyxSession {
    pub cds: CdsClient,
    pub auth: AuthenticationResult,
    pub sip_creds: UserSipCredentials,
    pub numbers: Vec<UserNumberInfo>,
    pub sip_client: Option<Arc<SipClient>>,
    pub sip_events: Option<tokio::sync::mpsc::Receiver<SipEvent>>,
}

pub struct SwyxClient {
    pub server_host: String,
    pub cds_port: u16,
    pub sip_port: u16,
    pub use_tls: bool,
    session: Mutex<Option<SwyxSession>>,
}

impl SwyxClient {
    pub fn new(server_host: &str) -> Self {
        Self {
            server_host: server_host.to_string(),
            cds_port: 8094, // Default HTTP port for Swyx CDS SOAP
            sip_port: 5060, // Default SIP port
            use_tls: false,
            session: Mutex::new(None),
        }
    }

    pub fn with_ports(mut self, cds_port: u16, sip_port: u16, use_tls: bool) -> Self {
        self.cds_port = cds_port;
        self.sip_port = sip_port;
        self.use_tls = use_tls;
        self
    }

    /// Full login flow: Authenticates with CDS, fetches SIP credentials, and registers SIP
    pub async fn login_and_connect(
        &self,
        username: &str,
        password: &str,
        local_ip: &str,
        local_sip_port: u16,
    ) -> Result<()> {
        info!("Connecting to SwyxServer at {}", self.server_host);

        // 1. CDS Login
        let cds = CdsClient::new(self.server_host.clone(), self.cds_port, self.use_tls);
        let auth = cds.login(username, password).await?;

        // 2. Fetch User SIP Credentials & PSK
        let sip_creds = cds
            .get_sip_credentials(auth.user_id, username, &auth.access_token)
            .await?;
        info!(
            "Fetched SIP credentials: user='{}', realm='{}', has_psk={}",
            sip_creds.sip_user_name,
            sip_creds.sip_realm,
            sip_creds.psk.is_some()
        );

        // 3. Fetch User Numbers / Extensions
        let numbers = cds
            .get_numbers(auth.user_id, username, &auth.access_token)
            .await
            .unwrap_or_default();

        for n in &numbers {
            info!(
                "Assigned Extension: {:?}, Public: {:?}",
                n.internal_number, n.public_number
            );
        }

        // 4. Initialize and Register SIP Client
        let sip_pass = sip_creds
            .psk
            .clone()
            .unwrap_or_else(|| password.to_string());

        let sip_cfg = SipConfig {
            server_ip: self.server_host.clone(),
            server_port: self.sip_port,
            username: if !sip_creds.sip_user_name.is_empty() {
                sip_creds.sip_user_name.clone()
            } else {
                username.to_string()
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
        let sip_client_arc = Arc::new(sip_client);

        // Register to PBX
        if let Err(e) = sip_client_arc.register(3600).await {
            info!("SIP registration note: {} (will proceed for CTI/CDS operations)", e);
        }

        let mut lock = self.session.lock().await;
        *lock = Some(SwyxSession {
            cds,
            auth,
            sip_creds,
            numbers,
            sip_client: Some(sip_client_arc),
            sip_events: Some(event_rx),
        });

        info!("SwyxClient initialization complete!");
        Ok(())
    }

    /// Makes an outbound call
    pub async fn dial(&self, target_number: &str) -> Result<ActiveCall> {
        let lock = self.session.lock().await;
        let sess = lock
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call login_and_connect first."))?;

        let sip = sess
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("SIP client not initialized"))?;

        sip.dial(target_number).await
    }

    /// Terminates an active call
    pub async fn hangup(&self, call: &ActiveCall) -> Result<()> {
        let lock = self.session.lock().await;
        let sess = lock
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call login_and_connect first."))?;

        let sip = sess
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("SIP client not initialized"))?;

        sip.hangup(call).await
    }

    /// Retrieves the call journal
    pub async fn get_call_journal(&self) -> Result<Vec<CallJournalEntry>> {
        let lock = self.session.lock().await;
        let sess = lock
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call login_and_connect first."))?;

        sess.cds
            .get_call_journal(
                sess.auth.user_id,
                &sess.sip_creds.sip_user_name,
                &sess.auth.access_token,
            )
            .await
    }

    /// Retrieves user presence
    pub async fn get_presence(&self) -> Result<RichPresence> {
        let lock = self.session.lock().await;
        let sess = lock
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call login_and_connect first."))?;

        sess.cds
            .get_presence(
                sess.auth.user_id,
                &sess.sip_creds.sip_user_name,
                &sess.auth.access_token,
            )
            .await
    }
}
