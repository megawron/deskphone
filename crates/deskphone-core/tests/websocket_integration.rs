use std::net::SocketAddr;
use std::time::Duration;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use deskphone_core::events::{SoftphoneCommand, SoftphoneEvent};
use deskphone_core::{AsteriskProvider, SoftphoneEngine};

#[tokio::test]
async fn test_websocket_ipc_flow() {
    let ws_port = 8798;
    let addr: SocketAddr = format!("127.0.0.1:{}", ws_port).parse().unwrap();

    let provider = AsteriskProvider::new("127.0.0.1", "1001", "secret1001");
    let engine = SoftphoneEngine::new(Box::new(provider));

    // Connect & start engine on test port 50610 for isolation
    if let Err(e) = engine.start("127.0.0.1", 50610).await {
        eprintln!("Skipping WebSocket test (PBX container not reachable): {}", e);
        return;
    }

    // Spawn WebSocket server
    let engine_clone = engine.clone();
    tokio::spawn(async move {
        let _ = deskphone_core::run_websocket_server(engine_clone, addr).await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect as WebSocket client (as a GTK4 or web UI would)
    let ws_url = format!("ws://127.0.0.1:{}", ws_port);
    let (ws_stream, _) = match connect_async(&ws_url).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to connect to test WS: {}", e);
            return;
        }
    };

    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    // 1. Send Dial command over WebSocket
    let dial_cmd = SoftphoneCommand::Dial {
        number: "600".into(),
    };
    let json = serde_json::to_string(&dial_cmd).unwrap();
    ws_tx.send(Message::Text(json)).await.unwrap();

    // 2. Receive CallConnected event over WebSocket
    let mut call_id = String::new();
    while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(3), ws_rx.next()).await {
        if let Message::Text(text) = msg {
            if let Ok(event) = serde_json::from_str::<SoftphoneEvent>(&text) {
                if let SoftphoneEvent::CallConnected { call_id: cid, .. } = event {
                    call_id = cid;
                    break;
                }
            }
        }
    }

    assert!(!call_id.is_empty(), "Should have received CallConnected event");

    // 3. Send Hold command over WebSocket
    let hold_cmd = SoftphoneCommand::Hold {
        call_id: call_id.clone(),
    };
    let json = serde_json::to_string(&hold_cmd).unwrap();
    ws_tx.send(Message::Text(json)).await.unwrap();

    // 4. Expect CallHoldState { on_hold: true }
    let mut hold_received = false;
    while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(3), ws_rx.next()).await {
        if let Message::Text(text) = msg {
            if let Ok(event) = serde_json::from_str::<SoftphoneEvent>(&text) {
                if let SoftphoneEvent::CallHoldState { on_hold: true, .. } = event {
                    hold_received = true;
                    break;
                }
            }
        }
    }
    assert!(hold_received, "Should have received CallHoldState (on_hold=true)");

    // 5. Send Unhold command over WebSocket
    let unhold_cmd = SoftphoneCommand::Unhold {
        call_id: call_id.clone(),
    };
    let json = serde_json::to_string(&unhold_cmd).unwrap();
    ws_tx.send(Message::Text(json)).await.unwrap();

    // 6. Expect CallHoldState { on_hold: false }
    let mut unhold_received = false;
    while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(3), ws_rx.next()).await {
        if let Message::Text(text) = msg {
            if let Ok(event) = serde_json::from_str::<SoftphoneEvent>(&text) {
                if let SoftphoneEvent::CallHoldState { on_hold: false, .. } = event {
                    unhold_received = true;
                    break;
                }
            }
        }
    }
    assert!(unhold_received, "Should have received CallHoldState (on_hold=false)");

    // 7. Send Hangup command over WebSocket
    let hangup_cmd = SoftphoneCommand::Hangup {
        call_id: call_id.clone(),
    };
    let json = serde_json::to_string(&hangup_cmd).unwrap();
    ws_tx.send(Message::Text(json)).await.unwrap();

    // 4. Receive CallEnded event
    let mut ended = false;
    while let Ok(Some(Ok(msg))) = tokio::time::timeout(Duration::from_secs(3), ws_rx.next()).await {
        if let Message::Text(text) = msg {
            if let Ok(event) = serde_json::from_str::<SoftphoneEvent>(&text) {
                if let SoftphoneEvent::CallEnded { .. } = event {
                    ended = true;
                    break;
                }
            }
        }
    }

    assert!(ended, "Should have received CallEnded event");

    // 8. Test SendChatMessage over WebSocket
    let chat_cmd = SoftphoneCommand::SendChatMessage {
        target: "1002".into(),
        message: "Hallo via WebSocket!".into(),
    };
    let json = serde_json::to_string(&chat_cmd).unwrap();
    assert!(json.contains("SendChatMessage"));
    assert!(json.contains("Hallo via WebSocket!"));
    ws_tx.send(Message::Text(json)).await.unwrap();
}

