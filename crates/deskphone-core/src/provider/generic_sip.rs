use std::sync::Arc;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tracing::info;

use crate::provider::network::{check_tcp_port, check_udp_sip};
use crate::provider::{AccountInfo, PbxProvider, ProbeCheck, ProbeReport};
use crate::sip::{ActiveCall, SipClient, SipConfig, SipEvent};

pub struct GenericSipProvider {
    pub server_host: String,
    pub sip_port: u16,
    pub username: String,
    pub password: String,
    pub realm: Option<String>,
    sip_client: Option<Arc<SipClient>>,
    #[allow(dead_code)]
    event_rx: Option<tokio::sync::mpsc::Receiver<SipEvent>>,
}

impl GenericSipProvider {
    pub fn new(server_host: &str, username: &str, password: &str) -> Self {
        Self {
            server_host: server_host.to_string(),
            sip_port: 5060,
            username: username.to_string(),
            password: password.to_string(),
            realm: None,
            sip_client: None,
            event_rx: None,
        }
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.sip_port = port;
        self
    }

    pub fn with_realm(mut self, realm: &str) -> Self {
        self.realm = Some(realm.to_string());
        self
    }
}

#[async_trait]
impl PbxProvider for GenericSipProvider {
    fn name(&self) -> &'static str {
        "Generic SIP (FRITZ!Box / Asterisk / FreePBX / FreeSWITCH / sipgate)"
    }

    async fn probe(&self, local_ip: &str) -> Result<ProbeReport> {
        let mut checks = Vec::new();

        // 1. SIP UDP (Standard RFC 3261)
        let udp_ok = check_udp_sip(&self.server_host, self.sip_port, 1500).await;
        checks.push(ProbeCheck {
            port: self.sip_port,
            protocol: "UDP",
            service: "SIP VoIP Signaling",
            open: udp_ok,
            note: if udp_ok {
                Some("SIP server responded to OPTIONS ping".into())
            } else {
                Some("No direct OPTIONS reply (common for NAT/firewalled servers)".into())
            },
        });

        // 2. SIP TCP (Port 5060)
        let tcp_ok = check_tcp_port(&self.server_host, self.sip_port, 1500).await;
        checks.push(ProbeCheck {
            port: self.sip_port,
            protocol: "TCP",
            service: "SIP over TCP",
            open: tcp_ok,
            note: None,
        });

        // 3. SIPS TLS (Port 5061)
        let tls_ok = check_tcp_port(&self.server_host, 5061, 1500).await;
        checks.push(ProbeCheck {
            port: 5061,
            protocol: "TCP",
            service: "SIPS (Encrypted SIP TLS)",
            open: tls_ok,
            note: None,
        });

        Ok(ProbeReport {
            provider: self.name(),
            server: self.server_host.clone(),
            local_ip: local_ip.to_string(),
            checks,
        })
    }

    async fn connect(&mut self, local_ip: &str, local_sip_port: u16) -> Result<()> {
        let realm = self
            .realm
            .clone()
            .unwrap_or_else(|| self.server_host.clone());

        info!(
            "Connecting to Generic SIP Server {}:{} as user '{}' (realm '{}')",
            self.server_host, self.sip_port, self.username, realm
        );

        let cfg = SipConfig {
            server_ip: self.server_host.clone(),
            server_port: self.sip_port,
            username: self.username.clone(),
            password: self.password.clone(),
            realm,
            local_ip: local_ip.to_string(),
            local_port: local_sip_port,
        };

        let (client, rx) = SipClient::new(cfg).await?;
        let client_arc = Arc::new(client);

        // Perform SIP digest authentication registration
        client_arc.register(3600).await?;

        self.sip_client = Some(client_arc);
        self.event_rx = Some(rx);

        info!("Generic SIP registration successful!");
        Ok(())
    }

    async fn get_account_info(&self) -> Result<AccountInfo> {
        Ok(AccountInfo {
            user_id: self.username.clone(),
            display_name: Some(self.username.clone()),
            extensions: vec![self.username.clone()],
            sip_user: self.username.clone(),
            sip_realm: self
                .realm
                .clone()
                .unwrap_or_else(|| self.server_host.clone()),
        })
    }

    async fn dial(&self, target_number: &str) -> Result<ActiveCall> {
        let sip = self
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call connect() first."))?;
        sip.dial(target_number).await
    }

    async fn dial_with_id(&self, call_id: &str, target_number: &str) -> Result<ActiveCall> {
        let sip = self
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call connect() first."))?;
        sip.dial_with_call_id(call_id, target_number).await
    }

    async fn cancel_dial(&self, call_id: &str) -> Result<bool> {
        match self.sip_client.as_ref() {
            Some(sip) => Ok(sip.cancel_outgoing(call_id).await),
            None => Ok(false),
        }
    }

    async fn deflect(&self, call_id: &str, target: &str) -> Result<()> {
        let sip = self
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call connect() first."))?;
        sip.redirect_incoming_call(call_id, target).await
    }

    fn sip_registered(&self) -> Option<bool> {
        self.sip_client.as_ref().map(|s| s.is_registered())
    }

    async fn answer(&self, call_id: &str) -> Result<ActiveCall> {
        let sip = self
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call connect() first."))?;
        sip.answer_incoming_call(call_id).await
    }

    async fn reject(&self, call_id: &str, code: u16) -> Result<()> {
        let sip = self
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call connect() first."))?;
        sip.reject_incoming_call(call_id, code).await
    }

    fn take_event_receiver(&mut self) -> Option<tokio::sync::mpsc::Receiver<SipEvent>> {
        self.event_rx.take()
    }

    async fn hangup(&self, call: &ActiveCall) -> Result<()> {
        let sip = self
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call connect() first."))?;
        sip.hangup(call).await
    }

    async fn hold(&self, call: &mut ActiveCall) -> Result<()> {
        let sip = self
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call connect() first."))?;
        sip.hold(call).await
    }

    async fn unhold(&self, call: &mut ActiveCall) -> Result<()> {
        let sip = self
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call connect() first."))?;
        sip.unhold(call).await
    }

    async fn transfer(&self, call: &mut ActiveCall, target: &str) -> Result<()> {
        let sip = self
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call connect() first."))?;
        sip.transfer(call, target).await
    }

    async fn attended_transfer(&self, call1: &mut ActiveCall, call2: &ActiveCall) -> Result<()> {
        let sip = self
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call connect() first."))?;
        sip.attended_transfer(call1, call2).await
    }

    async fn send_message(&self, target: &str, message: &str) -> Result<()> {
        let sip = self
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call connect() first."))?;
        sip.send_instant_message(target, message).await
    }

    async fn set_presence(
        &self,
        status: crate::cds::PresenceStatus,
        status_text: &str,
    ) -> Result<()> {
        let sip = self
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call connect() first."))?;
        let is_open = matches!(status, crate::cds::PresenceStatus::Available);
        sip.publish_presence(is_open, status_text).await
    }
}

