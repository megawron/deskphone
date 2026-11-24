use std::net::SocketAddr;
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

use crate::engine::SoftphoneEngine;
use crate::events::SoftphoneCommand;

pub async fn run_websocket_server(engine: SoftphoneEngine, addr: SocketAddr) -> Result<()> {
    let listener = TcpListener::bind(addr).await?;
    info!("🚀 Softphone WebSocket Daemon listening on ws://{}", addr);

    while let Ok((stream, peer_addr)) = listener.accept().await {
        info!("WebSocket client connected from {}", peer_addr);
        let engine_clone = engine.clone();

        tokio::spawn(async move {
            let ws_stream = match tokio_tungstenite::accept_async(stream).await {
                Ok(ws) => ws,
                Err(e) => {
                    warn!("WebSocket handshake failed with {}: {}", peer_addr, e);
                    return;
                }
            };

            let (mut ws_tx, mut ws_rx) = ws_stream.split();
            let mut event_rx = engine_clone.subscribe();

            // Task 1: Push events from engine to WebSocket client
            let push_task = tokio::spawn(async move {
                while let Ok(event) = event_rx.recv().await {
                    if let Ok(json) = serde_json::to_string(&event) {
                        if ws_tx.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                }
            });

            // Task 2: Receive commands from WebSocket client and execute on engine
            while let Some(Ok(msg)) = ws_rx.next().await {
                if let Message::Text(text) = msg {
                    match serde_json::from_str::<SoftphoneCommand>(&text) {
                        Ok(cmd) => match cmd {
                            SoftphoneCommand::Dial { number } => {
                                info!("IPC Dial request to {}", number);
                                let _ = engine_clone.dial(&number).await;
                            }
                            SoftphoneCommand::Answer { call_id } => {
                                info!("IPC Answer request for {}", call_id);
                                let _ = engine_clone.answer(&call_id).await;
                            }
                            SoftphoneCommand::Hangup { call_id } => {
                                info!("IPC Hangup request for {}", call_id);
                                let _ = engine_clone.hangup(&call_id).await;
                            }
                            SoftphoneCommand::Hold { call_id } => {
                                info!("IPC Hold request for {}", call_id);
                                let _ = engine_clone.hold(&call_id).await;
                            }
                            SoftphoneCommand::Unhold { call_id } => {
                                info!("IPC Unhold request for {}", call_id);
                                let _ = engine_clone.unhold(&call_id).await;
                            }
                            SoftphoneCommand::Transfer { call_id, target } => {
                                info!("IPC Transfer request for {} to {}", call_id, target);
                                let _ = engine_clone.transfer(&call_id, &target).await;
                            }
                            SoftphoneCommand::AttendedTransfer { call_id, target_call_id } => {
                                info!("IPC AttendedTransfer request connecting {} to {}", call_id, target_call_id);
                                let _ = engine_clone.attended_transfer(&call_id, &target_call_id).await;
                            }
                            SoftphoneCommand::SetDnd { enabled } => {
                                info!("IPC SetDnd request: {}", enabled);
                                engine_clone.set_dnd(enabled);
                            }
                            SoftphoneCommand::SetPresence { status, status_text } => {
                                info!("IPC SetPresence request: {} ('{}')", status, status_text);
                                let st = crate::cds::PresenceStatus::from_i32(status);
                                let _ = engine_clone.set_user_presence(st, &status_text).await;
                            }
                            SoftphoneCommand::SetCallForward { enabled, target } => {
                                info!("IPC SetCallForward request: enabled={}, target='{}'", enabled, target);
                                let _ = engine_clone.set_call_forwarding(enabled, &target).await;
                            }
                            SoftphoneCommand::SendDtmf { call_id, digit } => {
                                info!("IPC DTMF request: '{}' on {}", digit, call_id);
                                let _ = engine_clone.send_dtmf(&call_id, digit).await;
                            }
                            SoftphoneCommand::SetMute { muted } => {
                                info!("IPC SetMute request: {}", muted);
                                engine_clone.set_mute(muted).await;
                            }
                            SoftphoneCommand::SendChatMessage { target, message } => {
                                info!("IPC SendChatMessage request to {}: {}", target, message);
                                let _ = engine_clone.send_chat_message(&target, &message).await;
                            }
                            _ => {}
                        },
                        Err(e) => {
                            warn!("Invalid JSON command from client: {}", e);
                        }
                    }
                }
            }

            push_task.abort();
            info!("WebSocket client {} disconnected", peer_addr);
        });
    }

    Ok(())
}
