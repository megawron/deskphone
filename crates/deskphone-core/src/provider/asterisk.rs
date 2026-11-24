use std::sync::Arc;
use std::time::Duration;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tracing::info;

use crate::cds::{CallJournalEntry, RichPresence};
use crate::provider::network::{check_tcp_port, check_udp_sip};
use crate::provider::{AccountInfo, PbxProvider, ProbeCheck, ProbeReport};
use crate::sip::{ActiveCall, SipClient, SipConfig, SipEvent};

pub struct AsteriskProvider {
    pub server_host: String,
    pub sip_port: u16,
    pub ami_port: u16,
    pub username: String,
    pub password: String,
    pub ami_user: String,
    pub ami_secret: String,
    pub context: String,
    sip_client: Option<Arc<SipClient>>,
    #[allow(dead_code)]
    event_rx: Option<tokio::sync::mpsc::Receiver<SipEvent>>,
}

impl AsteriskProvider {
    pub fn new(server_host: &str, username: &str, password: &str) -> Self {
        Self {
            server_host: server_host.to_string(),
            sip_port: 5060,
            ami_port: 5038,
            username: username.to_string(),
            password: password.to_string(),
            ami_user: "admin".to_string(),
            ami_secret: "adminpass".to_string(),
            context: "deskphone-test".to_string(),
            sip_client: None,
            event_rx: None,
        }
    }

    pub fn with_ami(mut self, port: u16, user: &str, secret: &str) -> Self {
        self.ami_port = port;
        self.ami_user = user.to_string();
        self.ami_secret = secret.to_string();
        self
    }

    /// Query extension state from Asterisk via Asterisk Manager Interface (AMI)
    async fn query_ami_extension_state(&self) -> Result<(i32, String)> {
        let addr = format!("{}:{}", self.server_host, self.ami_port);
        let stream = timeout(Duration::from_secs(2), TcpStream::connect(&addr)).await??;
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        // Read banner
        let mut banner = String::new();
        reader.read_line(&mut banner).await?;

        // Send Login
        let login_cmd = format!(
            "Action: Login\r\nUsername: {}\r\nSecret: {}\r\nEvents: off\r\n\r\n",
            self.ami_user, self.ami_secret
        );
        writer.write_all(login_cmd.as_bytes()).await?;

        // Read login response
        let mut line = String::new();
        let mut login_ok = false;
        while reader.read_line(&mut line).await? > 0 {
            if line.contains("Response: Success") {
                login_ok = true;
            }
            if line.trim().is_empty() {
                break;
            }
            line.clear();
        }

        if !login_ok {
            return Err(anyhow!("Asterisk AMI login failed"));
        }

        // Send ExtensionState query
        let query_cmd = format!(
            "Action: ExtensionState\r\nExten: {}\r\nContext: {}\r\n\r\n",
            self.username, self.context
        );
        writer.write_all(query_cmd.as_bytes()).await?;

        let mut status_code = 0;
        let mut status_text = "Available".to_string();

        line.clear();
        while reader.read_line(&mut line).await? > 0 {
            if line.starts_with("Status: ") {
                let s = line.trim_start_matches("Status: ").trim();
                status_code = s.parse().unwrap_or(0);
            }
            if line.starts_with("StatusText: ") {
                status_text = line.trim_start_matches("StatusText: ").trim().to_string();
            }
            if line.trim().is_empty() {
                break;
            }
            line.clear();
        }

        // Send Logoff
        let _ = writer.write_all(b"Action: Logoff\r\n\r\n").await;

        Ok((status_code, status_text))
    }
}

#[async_trait]
impl PbxProvider for AsteriskProvider {
    fn name(&self) -> &'static str {
        "Asterisk PBX (PJSIP + AMI CTI)"
    }

    async fn probe(&self, local_ip: &str) -> Result<ProbeReport> {
        let mut checks = Vec::new();

        // 1. SIP UDP
        let udp_ok = check_udp_sip(&self.server_host, self.sip_port, 1500).await;
        checks.push(ProbeCheck {
            port: self.sip_port,
            protocol: "UDP",
            service: "SIP VoIP Signaling",
            open: udp_ok,
            note: if udp_ok {
                Some("Asterisk PJSIP responded to OPTIONS ping".into())
            } else {
                Some("No direct OPTIONS reply".into())
            },
        });

        // 2. SIP TCP
        let tcp_ok = check_tcp_port(&self.server_host, self.sip_port, 1500).await;
        checks.push(ProbeCheck {
            port: self.sip_port,
            protocol: "TCP",
            service: "SIP over TCP",
            open: tcp_ok,
            note: None,
        });

        // 3. Asterisk Manager Interface (AMI)
        let ami_ok = check_tcp_port(&self.server_host, self.ami_port, 1500).await;
        checks.push(ProbeCheck {
            port: self.ami_port,
            protocol: "TCP",
            service: "Asterisk Manager (AMI CTI)",
            open: ami_ok,
            note: if ami_ok {
                Some("CTI & Event control available".into())
            } else {
                Some("AMI port closed or not configured".into())
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
        info!(
            "Connecting to Asterisk PBX {}:{} as user '{}'",
            self.server_host, self.sip_port, self.username
        );

        let cfg = SipConfig {
            server_ip: self.server_host.clone(),
            server_port: self.sip_port,
            username: self.username.clone(),
            password: self.password.clone(),
            realm: self.server_host.clone(),
            local_ip: local_ip.to_string(),
            local_port: local_sip_port,
        };

        let (client, rx) = SipClient::new(cfg).await?;
        let client_arc = Arc::new(client);

        // Register to Asterisk PJSIP
        client_arc.register(3600).await?;

        self.sip_client = Some(client_arc);
        self.event_rx = Some(rx);

        info!("Asterisk PJSIP registration successful!");
        Ok(())
    }

    async fn get_account_info(&self) -> Result<AccountInfo> {
        Ok(AccountInfo {
            user_id: self.username.clone(),
            display_name: Some(format!("Asterisk Extension {}", self.username)),
            extensions: vec![self.username.clone()],
            sip_user: self.username.clone(),
            sip_realm: self.server_host.clone(),
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

    async fn get_call_journal(&self) -> Result<Vec<CallJournalEntry>> {
        // Asterisk CDR can be queried via AMI or database
        Ok(Vec::new())
    }

    async fn get_presence(&self) -> Result<Option<RichPresence>> {
        match self.query_ami_extension_state().await {
            Ok((code, text)) => Ok(Some(RichPresence {
                user_id: self.username.parse().unwrap_or(0),
                status: match code {
                    0 => crate::cds::PresenceStatus::Available,
                    1 => crate::cds::PresenceStatus::Busy,
                    4 => crate::cds::PresenceStatus::Offline,
                    _ => crate::cds::PresenceStatus::Away,
                },
                status_text: text,
            })),
            Err(e) => {
                info!("Could not query Asterisk AMI presence: {}", e);
                Ok(None)
            }
        }
    }

    async fn get_phonebook(&self) -> Result<Vec<crate::events::PhonebookEntry>> {
        Ok(vec![
            crate::events::PhonebookEntry {
                name: "Echo Application".into(),
                number: "600".into(),
                extension: Some("600".into()),
                department: Some("Testing".into()),
                ..Default::default()
            },
            crate::events::PhonebookEntry {
                name: "Milliwatt 1000Hz Tone".into(),
                number: "700".into(),
                extension: Some("700".into()),
                department: Some("Testing".into()),
                ..Default::default()
            },
            crate::events::PhonebookEntry {
                name: "User 1001".into(),
                number: "1001".into(),
                extension: Some("1001".into()),
                department: Some("Support".into()),
                ..Default::default()
            },
            crate::events::PhonebookEntry {
                name: "User 1002".into(),
                number: "1002".into(),
                extension: Some("1002".into()),
                department: Some("Sales".into()),
                ..Default::default()
            },
        ])
    }

    async fn send_message(&self, target: &str, message: &str) -> Result<()> {
        let sip = self
            .sip_client
            .as_ref()
            .ok_or_else(|| anyhow!("Not connected. Call connect() first."))?;
        sip.send_instant_message(target, message).await
    }
}

