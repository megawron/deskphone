# Provider Development Guide

This guide explains how to add support for a new phone system (such as STARFACE, innovaphone, 3CX, or Cisco CUCM) to `deskphone-core`.

## Overview

`deskphone-core` separates phone-system specifics from standard telephony:

- **Provider implementation:** Handles login, account discovery, and optional features like call history, colleague presence, or a company directory.
- **Shared SIP and audio engine:** Call setup, RTP audio, hold, transfers, and DTMF keypad tones are already handled by `SipClient` in `crates/deskphone-core/src/sip/client.rs`.

If your PBX speaks standard SIP for voice calls, you do not need to write any SIP or audio code yourself - just implement `PbxProvider` and delegate call actions to `SipClient`.

## The `PbxProvider` Trait

Defined in [crates/deskphone-core/src/provider/mod.rs](../crates/deskphone-core/src/provider/mod.rs):

```rust
use async_trait::async_trait;
use anyhow::Result;
use crate::provider::{AccountInfo, ProbeReport};
use crate::sip::{ActiveCall, SipEvent};
use crate::cds::{CallJournalEntry, RichPresence};
use crate::events::PhonebookEntry;

#[async_trait]
pub trait PbxProvider: Send + Sync {
    /// Human-readable provider name (e.g. "3CX Phone System").
    fn name(&self) -> &'static str;

    /// Non-invasive reachability check without logging in.
    async fn probe(&self, local_ip: &str) -> Result<ProbeReport>;

    /// Authenticate and register with the PBX.
    async fn connect(&mut self, local_ip: &str, local_sip_port: u16) -> Result<()>;

    /// Return user account details (display name, extension, SIP realm).
    async fn get_account_info(&self) -> Result<AccountInfo>;

    /// Place an outgoing call.
    async fn dial(&self, target_number: &str) -> Result<ActiveCall>;

    /// Answer an incoming call.
    async fn answer(&self, call_id: &str) -> Result<ActiveCall> {
        Err(anyhow::anyhow!("Incoming calls not supported"))
    }

    /// Reject an incoming call with a SIP status code (e.g. 486 or 603).
    async fn reject(&self, call_id: &str, code: u16) -> Result<()> {
        Err(anyhow::anyhow!("Incoming call rejection not supported"))
    }

    /// Hand over the incoming SIP event stream to the engine.
    fn take_event_receiver(&mut self) -> Option<tokio::sync::mpsc::Receiver<SipEvent>> {
        None
    }

    /// End an active call.
    async fn hangup(&self, call: &ActiveCall) -> Result<()>;

    /// Send a DTMF keypad tone.
    async fn send_dtmf(&self, call: &ActiveCall, digit: char) -> Result<()> {
        call.send_dtmf(digit).await
    }

    async fn hold(&self, call: &mut ActiveCall) -> Result<()>;
    async fn unhold(&self, call: &mut ActiveCall) -> Result<()>;
    async fn transfer(&self, call: &mut ActiveCall, target: &str) -> Result<()>;
    async fn attended_transfer(&self, call1: &mut ActiveCall, call2: &ActiveCall) -> Result<()>;

    // Optional directory and status methods (default to empty/none)

    async fn get_call_journal(&self) -> Result<Vec<CallJournalEntry>> {
        Ok(Vec::new())
    }

    async fn get_presence(&self) -> Result<Option<RichPresence>> {
        Ok(None)
    }

    async fn get_phonebook(&self) -> Result<Vec<PhonebookEntry>> {
        Ok(Vec::new())
    }
}
```

## Step-by-Step

1. Copy [crates/deskphone-core/src/provider/template.rs](../crates/deskphone-core/src/provider/template.rs) to `crates/deskphone-core/src/provider/<your_provider>.rs`.
2. Export the module in [crates/deskphone-core/src/provider/mod.rs](../crates/deskphone-core/src/provider/mod.rs):
   ```rust
   pub mod your_provider;
   ```
3. Wire up `SipClient` inside `connect()` and delegate `dial`, `answer`, `reject`, `hangup`, `hold`, `unhold`, `transfer`, and `attended_transfer` to it (see `generic_sip.rs` or `asterisk.rs` for concise examples).
4. Add the provider option to the CLI in [crates/deskphone-cli/src/main.rs](../crates/deskphone-cli/src/main.rs) and, if appropriate, to the GTK desktop settings dialog.
5. Add unit or integration tests under `crates/deskphone-core/tests/`.

## Minimal Example

```rust
use std::sync::Arc;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use crate::provider::{AccountInfo, PbxProvider, ProbeCheck, ProbeReport};
use crate::provider::network::check_udp_sip;
use crate::sip::{ActiveCall, SipClient, SipConfig, SipEvent};

pub struct CustomProvider {
    pub server_host: String,
    pub sip_port: u16,
    pub username: String,
    pub password: String,
    sip_client: Option<Arc<SipClient>>,
    event_rx: Option<tokio::sync::mpsc::Receiver<SipEvent>>,
}

impl CustomProvider {
    pub fn new(server_host: &str, username: &str, password: &str) -> Self {
        Self {
            server_host: server_host.to_string(),
            sip_port: 5060,
            username: username.to_string(),
            password: password.to_string(),
            sip_client: None,
            event_rx: None,
        }
    }
}

#[async_trait]
impl PbxProvider for CustomProvider {
    fn name(&self) -> &'static str {
        "Custom PBX"
    }

    async fn probe(&self, local_ip: &str) -> Result<ProbeReport> {
        let sip_ok = check_udp_sip(&self.server_host, self.sip_port, 1500).await;
        Ok(ProbeReport {
            provider: self.name(),
            server: self.server_host.clone(),
            local_ip: local_ip.to_string(),
            checks: vec![ProbeCheck {
                port: self.sip_port,
                protocol: "UDP",
                service: "SIP Signaling",
                open: sip_ok,
                note: None,
            }],
        })
    }

    async fn connect(&mut self, local_ip: &str, local_sip_port: u16) -> Result<()> {
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
        client_arc.register(3600).await?;

        self.sip_client = Some(client_arc);
        self.event_rx = Some(rx);
        Ok(())
    }

    async fn get_account_info(&self) -> Result<AccountInfo> {
        Ok(AccountInfo {
            user_id: self.username.clone(),
            display_name: Some(self.username.clone()),
            extensions: vec![self.username.clone()],
            sip_user: self.username.clone(),
            sip_realm: self.server_host.clone(),
        })
    }

    async fn dial(&self, target_number: &str) -> Result<ActiveCall> {
        let sip = self.sip_client.as_ref().ok_or_else(|| anyhow!("Not connected"))?;
        sip.dial(target_number).await
    }

    async fn answer(&self, call_id: &str) -> Result<ActiveCall> {
        let sip = self.sip_client.as_ref().ok_or_else(|| anyhow!("Not connected"))?;
        sip.answer_incoming_call(call_id).await
    }

    async fn reject(&self, call_id: &str, code: u16) -> Result<()> {
        let sip = self.sip_client.as_ref().ok_or_else(|| anyhow!("Not connected"))?;
        sip.reject_incoming_call(call_id, code).await
    }

    fn take_event_receiver(&mut self) -> Option<tokio::sync::mpsc::Receiver<SipEvent>> {
        self.event_rx.take()
    }

    async fn hangup(&self, call: &ActiveCall) -> Result<()> {
        let sip = self.sip_client.as_ref().ok_or_else(|| anyhow!("Not connected"))?;
        sip.hangup(call).await
    }

    async fn hold(&self, call: &mut ActiveCall) -> Result<()> {
        let sip = self.sip_client.as_ref().ok_or_else(|| anyhow!("Not connected"))?;
        sip.hold(call).await
    }

    async fn unhold(&self, call: &mut ActiveCall) -> Result<()> {
        let sip = self.sip_client.as_ref().ok_or_else(|| anyhow!("Not connected"))?;
        sip.unhold(call).await
    }

    async fn transfer(&self, call: &mut ActiveCall, target: &str) -> Result<()> {
        let sip = self.sip_client.as_ref().ok_or_else(|| anyhow!("Not connected"))?;
        sip.transfer(call, target).await
    }

    async fn attended_transfer(&self, call1: &mut ActiveCall, call2: &ActiveCall) -> Result<()> {
        let sip = self.sip_client.as_ref().ok_or_else(|| anyhow!("Not connected"))?;
        sip.attended_transfer(call1, call2).await
    }
}
```

## Testing with the CLI

```bash
# Check server reachability
cargo run --bin deskphone-cli -- -P your_provider probe -s 192.168.1.100

# Verify login and account info
cargo run --bin deskphone-cli -- -P your_provider login -s 192.168.1.100 -u 200 -p secret

# Run the offline unit test suite
cargo test -p deskphone-core --lib
```
