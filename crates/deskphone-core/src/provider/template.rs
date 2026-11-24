//! # PBX Provider Contribution Template
//!
//! This module provides a clean blueprint for adding new PBX systems
//! (e.g. STARFACE, innovaphone, 3CX, NFON Cloudya, Cisco CUCM, Mitel, etc.).
//!
//! ## How to add a new provider:
//! 1. Copy this template to `src/provider/my_provider.rs`.
//! 2. Implement the `PbxProvider` trait methods:
//!    - `probe`: Test ports (e.g., REST API port, SIP port).
//!    - `connect`: Perform authentication (OAuth2, REST API key, or SOAP) and register SIP.
//!    - `dial`: Initiate calls (via CTI REST API or direct SIP `SipClient::dial`).
//!    - `hangup`: Hang up calls.
//!    - `get_call_journal`: Retrieve call history via the PBX's REST/CTI API.
//!    - `get_presence`: Retrieve user presence via the PBX's REST/WebSocket API.
//! 3. Register your provider in `src/provider/mod.rs` and the CLI in `src/main.rs`.
//! 4. Submit a Pull Request! Community contributions are warmly welcome!

use anyhow::{anyhow, Result};
use async_trait::async_trait;

use crate::cds::{CallJournalEntry, RichPresence};
use crate::provider::{AccountInfo, PbxProvider, ProbeReport};
use crate::sip::ActiveCall;

#[allow(dead_code)]
pub struct CustomPbxProvider {
    pub server_host: String,
    pub api_port: u16,
    pub sip_port: u16,
    pub username: String,
    pub password: String,
}

impl CustomPbxProvider {
    pub fn new(server_host: &str, username: &str, password: &str) -> Self {
        Self {
            server_host: server_host.to_string(),
            api_port: 443,
            sip_port: 5060,
            username: username.to_string(),
            password: password.to_string(),
        }
    }
}

#[async_trait]
impl PbxProvider for CustomPbxProvider {
    fn name(&self) -> &'static str {
        "Custom PBX Template"
    }

    async fn probe(&self, _local_ip: &str) -> Result<ProbeReport> {
        // Tip: Use crate::provider::network::check_tcp_port or check_udp_sip
        Err(anyhow!("Not yet implemented for Custom PBX"))
    }

    async fn connect(&mut self, _local_ip: &str, _local_sip_port: u16) -> Result<()> {
        // 1. Authenticate with PBX API (REST / WebSocket / SOAP)
        // 2. Fetch SIP credentials or use provided credentials
        // 3. Register with SIP server using crate::sip::SipClient
        Err(anyhow!("Not yet implemented for Custom PBX"))
    }

    async fn get_account_info(&self) -> Result<AccountInfo> {
        Err(anyhow!("Not yet implemented for Custom PBX"))
    }

    async fn dial(&self, _target_number: &str) -> Result<ActiveCall> {
        Err(anyhow!("Not yet implemented for Custom PBX"))
    }

    async fn hangup(&self, _call: &ActiveCall) -> Result<()> {
        Err(anyhow!("Not yet implemented for Custom PBX"))
    }

    async fn get_call_journal(&self) -> Result<Vec<CallJournalEntry>> {
        Ok(Vec::new())
    }

    async fn get_presence(&self) -> Result<Option<RichPresence>> {
        Ok(None)
    }
}
