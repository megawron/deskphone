use deskphone_core::{PbxProvider, SwyxProvider};

#[tokio::test]
async fn test_swyx_mock_probe() {
    let provider = SwyxProvider::new("127.0.0.1", "alice", "secret");
    let report = match provider.probe("127.0.0.1").await {
        Ok(r) => r,
        Err(_) => return, // Skip if Docker environment not running
    };

    assert_eq!(report.server, "127.0.0.1");
    let soap_check = report.checks.iter().find(|c| c.port == 8094).unwrap();
    if !soap_check.open {
        eprintln!("Skipping Swyx mock probe test (mock container on 127.0.0.1:8094 not running)");
        return;
    }

    let sip_check = report.checks.iter().find(|c| c.port == 5060);
    assert!(sip_check.is_some());
    assert!(sip_check.unwrap().open, "SIP port 5060 should be responsive");
}

#[tokio::test]
async fn test_swyx_mock_full_flow() {
    let mut provider = SwyxProvider::new("127.0.0.1", "alice", "secret");

    // 1. Connect (SOAP CDS Login + PSK Query + SIP Register on ephemeral port 50606)
    if let Err(e) = provider.connect("127.0.0.1", 50606).await {
        eprintln!("Skipping Swyx test (environment not reachable): {}", e);
        return;
    }

    // 2. Account Info
    let info = provider.get_account_info().await.unwrap();
    assert_eq!(info.user_id, "42");
    assert_eq!(info.sip_user, "1001");

    // 3. Presence
    let presence = provider.get_presence().await.unwrap();
    assert!(presence.is_some());

    // 4. Journal
    let journal = provider.get_call_journal().await.unwrap();
    assert!(!journal.is_empty());
    assert_eq!(journal[0].caller_number, "+491701234567");

    // 5. Dial Echo (extension 600)
    let call = provider.dial("600").await.expect("Failed to dial Echo application");
    assert!(!call.call_id.is_empty());

    // 6. Let audio stream for 500ms
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 7. Hangup
    provider.hangup(&call).await.expect("Failed to hang up call");
}
