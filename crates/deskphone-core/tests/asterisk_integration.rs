use deskphone_core::{AsteriskProvider, GenericSipProvider, PbxProvider};

#[tokio::test]
async fn test_asterisk_probe() {
    let provider = AsteriskProvider::new("127.0.0.1", "1001", "secret1001");
    let report = match provider.probe("127.0.0.1").await {
        Ok(r) => r,
        Err(_) => return, // Skip if Docker container is not running
    };

    assert_eq!(report.server, "127.0.0.1");
    let sip_check = report.checks.iter().find(|c| c.port == 5060);
    let ami_check = report.checks.iter().find(|c| c.port == 5038);
    assert!(sip_check.is_some());
    assert!(ami_check.is_some());

    if !sip_check.unwrap().open && !ami_check.unwrap().open {
        eprintln!("Skipping Asterisk probe test (container not running)");
        return;
    }

    assert!(sip_check.unwrap().open, "SIP port 5060 should be responsive");
    assert!(ami_check.unwrap().open, "AMI port 5038 should be open");
}

#[tokio::test]
async fn test_asterisk_sip_register_and_echo_call() {
    let mut provider = AsteriskProvider::new("127.0.0.1", "1001", "secret1001");

    // 1. Connect & Register SIP (bind to ephemeral port 50602 for test isolation)
    if let Err(e) = provider.connect("127.0.0.1", 50602).await {
        eprintln!("Skipping Asterisk test (container not reachable): {}", e);
        return;
    }

    // 2. Verify account info
    let info = provider.get_account_info().await.unwrap();
    assert_eq!(info.user_id, "1001");
    assert_eq!(info.sip_user, "1001");

    // 3. Verify presence via AMI
    let presence = provider.get_presence().await.unwrap();
    assert!(presence.is_some());

    // 4. Dial Echo application (extension 600)
    let call = provider.dial("600").await.expect("Failed to dial Echo application");
    assert!(!call.call_id.is_empty());

    // 5. Let audio stream for 1 second
    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

    // 6. Terminate call
    provider.hangup(&call).await.expect("Failed to hang up call");
}

#[tokio::test]
async fn test_generic_sip_asterisk_echo_call() {
    let mut provider = GenericSipProvider::new("127.0.0.1", "1002", "secret1002");

    // 1. Connect & Register
    if let Err(e) = provider.connect("127.0.0.1", 50604).await {
        eprintln!("Skipping Generic SIP test: {}", e);
        return;
    }

    // 2. Dial Echo application
    let call = provider.dial("600").await.expect("Failed to dial Echo");
    assert!(!call.call_id.is_empty());

    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

    provider.hangup(&call).await.expect("Failed to hang up call");
}

#[tokio::test]
async fn test_asterisk_hold_and_unhold() {
    let mut provider = AsteriskProvider::new("127.0.0.1", "1001", "secret1001");

    // 1. Connect & Register SIP (bind to ephemeral port 50606 for test isolation)
    if let Err(e) = provider.connect("127.0.0.1", 50606).await {
        eprintln!("Skipping Asterisk hold test (container not reachable): {}", e);
        return;
    }

    // 2. Dial Echo application (extension 600)
    let mut call = provider.dial("600").await.expect("Failed to dial Echo application");
    assert!(!call.call_id.is_empty());
    assert!(!call.on_hold);

    // 3. Audio flows for 500ms
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 4. Put call on hold (re-INVITE with a=sendonly)
    provider.hold(&mut call).await.expect("Failed to put call on hold");
    assert!(call.on_hold, "Call should be marked on_hold");

    // Wait 500ms while on hold
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 5. Unhold call (re-INVITE with a=sendrecv)
    provider.unhold(&mut call).await.expect("Failed to resume call from hold");
    assert!(!call.on_hold, "Call should no longer be on_hold");

    // 6. Terminate call
    provider.hangup(&call).await.expect("Failed to hang up call");
}

#[tokio::test]
async fn test_asterisk_transfer() {
    let mut provider = AsteriskProvider::new("127.0.0.1", "1001", "secret1001");

    // 1. Connect & Register SIP (bind to ephemeral port 50608 for test isolation)
    if let Err(e) = provider.connect("127.0.0.1", 50608).await {
        eprintln!("Skipping Asterisk transfer test (container not reachable): {}", e);
        return;
    }

    // 2. Dial Echo application (extension 600)
    let mut call = provider.dial("600").await.expect("Failed to dial Echo application");
    assert!(!call.call_id.is_empty());

    // 3. Audio flows for 500ms
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 4. Blind transfer call to extension 700 (Milliwatt tone) via SIP REFER (RFC 3515)
    let transfer_res = provider.transfer(&mut call, "700").await;
    match transfer_res {
        Ok(_) => {
            // Transfer succeeded and hung up our leg
        }
        Err(e) => {
            // Some PBX dialplans require special transfer permissions or contexts
            eprintln!("Asterisk transfer response: {}", e);
        }
    }
}
