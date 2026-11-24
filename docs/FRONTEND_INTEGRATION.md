# Frontend Integration Guide

`deskphone-core` is a headless telephony library that can be embedded directly in Rust applications or controlled from any language over a local WebSocket daemon (`deskphone-cli`).

## Choosing an Integration Path

| | In-Process Rust Library | WebSocket IPC Daemon |
| --- | --- | --- |
| **Best for** | Native Rust apps (GTK 4, Iced, Slint) | Web apps, scripts, Tauri, Qt, Python |
| **Connection** | Direct crate dependency | `ws://127.0.0.1:8765` |
| **Audio** | Handled in-process | Handled in background daemon |

## Path A: In-Process Rust Library

Add `deskphone-core` to your `Cargo.toml`:

```toml
[dependencies]
deskphone-core = { git = "https://github.com/megawron/deskphone.git", package = "deskphone-core" }
tokio = { version = "1", features = ["full"] }
```

### Initializing the Engine

Pick a provider (`GenericSipProvider`, `AsteriskProvider`, or `SwyxProvider`) and wrap it in `SoftphoneEngine`:

```rust
use deskphone_core::engine::SoftphoneEngine;
use deskphone_core::provider::generic_sip::GenericSipProvider;

let provider = Box::new(
    GenericSipProvider::new("fritz.box", "620", "secret")
);
let engine = SoftphoneEngine::new(provider);
```

### Subscribing to Events

Call `engine.subscribe()` to receive live call and registration events (`SoftphoneEvent`), and forward them to your UI thread (for example via `async-channel` into `glib::MainContext::default().spawn_local(...)` in GTK 4):

```rust
use deskphone_core::events::SoftphoneEvent;

let mut rx = engine.subscribe();
while let Ok(event) = rx.recv().await {
    match event {
        SoftphoneEvent::IncomingCall { call_id, caller_number, caller_name } => {
            println!("Incoming call {call_id} from {caller_name:?} ({caller_number})");
        }
        SoftphoneEvent::CallConnected { call_id, target_number } => {
            println!("Connected {call_id} with {target_number}");
        }
        SoftphoneEvent::CallHoldState { call_id, on_hold } => {
            println!("Call {call_id} on hold: {on_hold}");
        }
        SoftphoneEvent::CallEnded { call_id, reason } => {
            println!("Call {call_id} ended: {reason}");
        }
        SoftphoneEvent::RegistrationState { registered, account, message, .. } => {
            println!("Registered={registered} ({account}): {message}");
        }
        _ => {}
    }
}
```

### Common Engine Methods

```rust
let call_id = engine.dial("201").await?;
engine.answer(&call_id).await?;
engine.reject(&call_id, 486).await?;
engine.hangup(&call_id).await?;

engine.hold(&call_id).await?;
engine.unhold(&call_id).await?;
engine.transfer(&call_id, "202").await?;
engine.attended_transfer(&call_id_1, &call_id_2).await?;
engine.send_dtmf(&call_id, '1').await?;

engine.set_mute(true).await;
engine.set_dnd(true);

let journal = engine.get_call_journal().await?;
let presence = engine.get_presence().await?;
let contacts = engine.get_phonebook().await?;
```

## Path B: WebSocket IPC Daemon

Start the daemon with `deskphone-cli`:

```bash
cargo run --release --bin deskphone-cli -- -P generic serve \
  -s fritz.box -u 620 -p secret --ws-port 8765
```

### Commands (Client -> Daemon)

Send JSON messages over `ws://127.0.0.1:8765`:

```json
{ "action": "Dial", "params": { "number": "201" } }
{ "action": "Answer", "params": { "call_id": "call-id-1" } }
{ "action": "Hangup", "params": { "call_id": "call-id-1" } }
{ "action": "Hold", "params": { "call_id": "call-id-1" } }
{ "action": "Unhold", "params": { "call_id": "call-id-1" } }
{ "action": "Transfer", "params": { "call_id": "call-id-1", "target": "202" } }
{ "action": "AttendedTransfer", "params": { "call_id": "call-id-1", "target_call_id": "call-id-2" } }
{ "action": "SendDtmf", "params": { "call_id": "call-id-1", "digit": "5" } }
{ "action": "SetMute", "params": { "muted": true } }
{ "action": "SetDnd", "params": { "enabled": true } }
{ "action": "SendChatMessage", "params": { "target": "201", "message": "Hello!" } }
```

### Events (Daemon -> Client)

The daemon broadcasts JSON events to all connected clients:

```json
{
  "event": "IncomingCall",
  "data": {
    "call_id": "9a7fbc28@192.168.1.50",
    "caller_number": "201",
    "caller_name": "Clara Weber"
  }
}
```

Other emitted events include `CallConnected`, `CallHoldState`, `CallTransferred`, `CallEnded`, `DndState`, `RegistrationState`, and `ChatMessageReceived`.

## Call Lifecycle

```text
Idle
 ├── dial()            -> Dialing -> (200 OK)  -> CallConnected
 └── incoming INVITE   -> IncomingCall
                            ├── answer()       -> CallConnected
                            └── reject()       -> CallEnded

CallConnected
 ├── hold() / unhold() -> CallHoldState
 ├── transfer()        -> CallTransferred      -> CallEnded
 └── hangup() / BYE    -> CallEnded
```
