//! Passive smoke test of the real engine code path against the configured SwyxServer:
//! CDS login + SIP REGISTER via `SoftphoneEngine::start`, then immediate de-registration.
//!
//! NO call is ever placed. Incoming INVITEs during the few seconds of registration are
//! NOT answered (the engine only signals them; this tool never calls `answer`).
//!
//! Usage: DESKPHONE_NO_AUDIO=1 cargo run -p deskphone-core --example engine_smoke

use std::time::Duration;

use anyhow::Result;
use deskphone_core::{SoftphoneEngine, SoftphoneEvent, SwyxProvider};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
        .init();

    let home = std::env::var("HOME")?;
    let data: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{home}/.config/deskphone/data.json"))?)?;
    let server = data["server_ip"].as_str().unwrap_or_default().to_string();
    let user = data["user_name"].as_str().unwrap_or_default().to_string();
    let password = data["password"].as_str().unwrap_or_default().to_string();

    let engine = SoftphoneEngine::new(Box::new(SwyxProvider::new(&server, &user, &password)));
    let mut events = engine.subscribe();

    let local_ip = deskphone_core::provider::network::get_local_ip_for(&server, 5060);
    let port = (50660..50700)
        .find(|p| std::net::UdpSocket::bind((local_ip.as_str(), *p)).is_ok())
        .unwrap_or(50660);

    engine.start(&local_ip, port).await?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while let Ok(Ok(ev)) = tokio::time::timeout_at(deadline, events.recv()).await {
        match ev {
            SoftphoneEvent::RegistrationState { registered, account, message, .. } => {
                println!(">>> RegistrationState registered={registered} account={account} msg={message}")
            }
            SoftphoneEvent::IncomingCall { caller_number, .. } => {
                println!(">>> (eingehender Ruf von {caller_number} wird ignoriert)")
            }
            other => println!(">>> {other:?}"),
        }
    }

    engine.shutdown().await?;
    println!(">>> SIP-Registrierung wieder entfernt. Fertig.");
    Ok(())
}
