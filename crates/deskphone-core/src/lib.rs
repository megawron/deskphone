pub mod cds;
pub mod client;
pub mod engine;
pub mod events;
pub mod provider;
pub mod rtp;
pub mod server;
pub mod sip;

pub use client::SwyxClient;
pub use engine::SoftphoneEngine;
pub use events::{PhonebookEntry, SoftphoneCommand, SoftphoneEvent};
pub use provider::asterisk::AsteriskProvider;
pub use provider::generic_sip::GenericSipProvider;
pub use provider::swyx::SwyxProvider;
pub use provider::{AccountInfo, PbxProvider, ProbeCheck, ProbeReport};
pub use server::run_websocket_server;

#[cfg(test)]
mod tests {
    use super::*;
    use cds::soap::{SoapBuilder, SoapParser};
    use rtp::codec::G711;
    use rtp::packet::RtpPacket;
    use sip::digest::DigestAuth;
    use sip::message::SipMessage;
    use sip::sdp::SdpSession;
    use sip::{SipClient, SipConfig, SipEvent};
    use bytes::Bytes;

    #[test]
    fn test_soap_login_builder_and_parser() {
        let req = SoapBuilder::build_login_request("alice", "secret123");
        assert!(req.contains("<d4p1:UserName>alice</d4p1:UserName>"));
        assert!(req.contains("<d4p1:Password>secret123</d4p1:Password>"));
        assert!(req.contains("http://tempuri.org/"));

        let sample_resp = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
            <s:Body>
                <LoginResponse xmlns="http://tempuri.org/">
                    <LoginResult xmlns:a="http://schemas.datacontract.org/2004/07/IpPbx.Web.Model">
                        <a:AccessToken>eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.sample</a:AccessToken>
                        <a:RefreshToken>refresh_abc</a:RefreshToken>
                        <a:UserId>42</a:UserId>
                    </LoginResult>
                </LoginResponse>
            </s:Body>
        </s:Envelope>"#;

        let res = SoapParser::parse_login_response(sample_resp).unwrap();
        assert_eq!(res.user_id, 42);
        assert_eq!(res.access_token, "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.sample");
        assert_eq!(res.refresh_token, "refresh_abc");
    }

    #[test]
    fn test_soap_sip_credentials_parser() {
        let sample_resp = r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
            <s:Body>
                <GetSipCredentialsResponse xmlns="http://tempuri.org/">
                    <GetSipCredentialsResult xmlns:a="http://schemas.datacontract.org/2004/07/SWConfigDataWSLib.Transport">
                        <a:SipRealm>swyx.company.local</a:SipRealm>
                        <a:SipUserID>101</a:SipUserID>
                        <a:SipUserName>alice_sip</a:SipUserName>
                        <a:UserID>42</a:UserID>
                    </GetSipCredentialsResult>
                </GetSipCredentialsResponse>
            </s:Body>
        </s:Envelope>"#;

        let res = SoapParser::parse_sip_credentials(sample_resp, 42).unwrap();
        assert_eq!(res.user_id, 42);
        assert_eq!(res.sip_realm, "swyx.company.local");
        assert_eq!(res.sip_user_name, "alice_sip");
        assert_eq!(res.sip_user_id, "101");
    }

    #[test]
    fn test_sip_digest_auth_rfc2617() {
        // RFC 2617 test vector check
        let resp = DigestAuth::compute_response(
            "MGRR",
            "circle",
            "testrealm@host.com",
            "dcd98b7102dd2f0e8b11d0f600bfb0c093",
            "GET",
            "/dir/index.html",
            Some("auth"),
            1,
            Some("0a4f113b"),
        );
        assert_eq!(resp, "5cf724946fd17e9075378637b3afac06");
    }

    #[test]
    fn test_sip_message_serialize_and_parse() {
        let mut msg = SipMessage::new_request("REGISTER", "sip:swyx.local");
        msg.set_header("Via", "SIP/2.0/UDP 192.168.1.50:5060;branch=z9hG4bK12345");
        msg.set_header("From", "<sip:alice@swyx.local>;tag=9876");
        msg.set_header("To", "<sip:alice@swyx.local>");
        msg.set_header("Call-ID", "abcd-1234");
        msg.set_header("CSeq", "1 REGISTER");

        let serialized = msg.serialize();
        let parsed = SipMessage::parse(&serialized).unwrap();

        assert_eq!(parsed.method(), Some("REGISTER"));
        assert_eq!(parsed.get_header("Call-ID"), Some("abcd-1234"));
        assert_eq!(parsed.cseq(), Some((1, "REGISTER".to_string())));
        assert_eq!(parsed.from_tag(), Some("9876".to_string()));
    }

    #[test]
    fn test_sdp_builder_and_parser() {
        let offer = SdpSession::build_offer("192.168.1.100", 12345);
        assert!(offer.contains("c=IN IP4 192.168.1.100"));
        assert!(offer.contains("m=audio 12345 RTP/AVP 8 0 101"));
        assert!(offer.contains("a=rtpmap:8 PCMA/8000"));
        // Opus wird nicht angeboten, da kein Opus-Codec implementiert ist
        assert!(!offer.contains("opus"));
        assert!(offer.contains("a=sendrecv"));

        let hold_offer = SdpSession::build_offer_with_direction(
            "192.168.1.100",
            12345,
            crate::sip::sdp::MediaDirection::SendOnly,
        );
        assert!(hold_offer.contains("a=sendonly"));

        let answer = "v=0\r\n\
            o=- 2000 2000 IN IP4 10.0.0.1\r\n\
            s=SwyxServer\r\n\
            c=IN IP4 10.0.0.1\r\n\
            t=0 0\r\n\
            m=audio 50004 RTP/AVP 8 101\r\n\
            a=rtpmap:8 PCMA/8000\r\n\
            a=sendrecv\r\n";

        let parsed = SdpSession::parse_answer(answer).unwrap();
        assert_eq!(parsed.remote_ip.unwrap().to_string(), "10.0.0.1");
        assert_eq!(parsed.remote_port.unwrap(), 50004);
        assert_eq!(parsed.selected_payload_type, 8);
        assert_eq!(parsed.direction, crate::sip::sdp::MediaDirection::SendRecv);
    }

    #[test]
    fn test_rtp_packet_roundtrip() {
        let payload = Bytes::from_static(b"0123456789ABCDEF");
        let pkt = RtpPacket::new(8, 42, 1600, 0x12345678, payload.clone());
        let serialized = pkt.serialize();

        let parsed = RtpPacket::parse(serialized).unwrap();
        assert_eq!(parsed.payload_type, 8);
        assert_eq!(parsed.sequence_number, 42);
        assert_eq!(parsed.timestamp, 1600);
        assert_eq!(parsed.ssrc, 0x12345678);
        assert_eq!(parsed.payload, payload);
    }

    #[test]
    fn test_g711_alaw_codec() {
        let pcm_in: i16 = 1234;
        let alaw = G711::linear_to_alaw(pcm_in);
        let pcm_out = G711::alaw_to_linear(alaw);
        // A-law is lossy companding, should be close
        assert!((pcm_in - pcm_out).abs() < 64);
    }

    #[test]
    fn test_providers_trait_dispatch() {
        let swyx: Box<dyn PbxProvider> = Box::new(SwyxProvider::new("127.0.0.1", "testuser", "pass"));
        assert!(swyx.name().contains("Swyx"));

        let sip: Box<dyn PbxProvider> = Box::new(GenericSipProvider::new("192.168.178.1", "620", "secret"));
        assert!(sip.name().contains("Generic SIP"));
    }

    #[test]
    fn test_dnd_state_toggle() {
        let provider = Box::new(GenericSipProvider::new("127.0.0.1", "test", "pass"));
        let engine = SoftphoneEngine::new(provider);
        let mut sub = engine.subscribe();

        assert!(!engine.is_dnd());

        engine.set_dnd(true);
        assert!(engine.is_dnd());

        let event = sub.try_recv().expect("Should have received DndState event");
        if let SoftphoneEvent::DndState { enabled } = event {
            assert!(enabled);
        } else {
            panic!("Expected DndState event");
        }

        engine.set_dnd(false);
        assert!(!engine.is_dnd());
    }

    #[test]
    fn test_attended_transfer_replaces_format() {
        let call_id = "test-call-456";
        let remote_tag = "remotetag123";
        let local_tag = "localtag789";

        let replaces_param = format!(
            "Replaces={}%3Bto-tag%3D{}%3Bfrom-tag%3D{}",
            call_id, remote_tag, local_tag
        );

        assert_eq!(
            replaces_param,
            "Replaces=test-call-456%3Bto-tag%3Dremotetag123%3Bfrom-tag%3Dlocaltag789"
        );

        let target = "1002";
        let realm = "pbx.company.local";
        let refer_to_header = format!("<sip:{}@{}?{}>", target, realm, replaces_param);
        assert_eq!(
            refer_to_header,
            "<sip:1002@pbx.company.local?Replaces=test-call-456%3Bto-tag%3Dremotetag123%3Bfrom-tag%3Dlocaltag789>"
        );
    }

    #[test]
    fn test_g711_mulaw_codec() {
        let pcm_in: i16 = 2500;
        let mulaw = G711::linear_to_mulaw(pcm_in);
        let pcm_out = G711::mulaw_to_linear(mulaw);
        assert!((pcm_in - pcm_out).abs() < 128);
    }

    #[tokio::test]
    async fn test_inbound_sip_options_ping() {
        let config = SipConfig {
            server_ip: "127.0.0.1".into(),
            server_port: 5060,
            username: "tester".into(),
            password: "password".into(),
            realm: "127.0.0.1".into(),
            local_ip: "127.0.0.1".into(),
            local_port: 50630,
        };
        let (_client, _rx) = SipClient::new(config).await.expect("Failed to create SipClient");

        // Act as remote entity sending OPTIONS
        let remote_sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target_addr: std::net::SocketAddr = "127.0.0.1:50630".parse().unwrap();

        let mut opt = SipMessage::new_request("OPTIONS", "sip:tester@127.0.0.1:50630");
        opt.set_header("Via", "SIP/2.0/UDP 127.0.0.1:50631;branch=z9hG4bKtestopt");
        opt.set_header("From", "<sip:caller@test.com>;tag=111");
        opt.set_header("To", "<sip:tester@127.0.0.1>");
        opt.set_header("Call-ID", "opt-ping-1234");
        opt.set_header("CSeq", "1 OPTIONS");

        remote_sock.send_to(&opt.serialize(), target_addr).await.unwrap();

        let mut buf = vec![0u8; 1500];
        let (len, _) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            remote_sock.recv_from(&mut buf),
        )
        .await
        .expect("Timeout waiting for OPTIONS response")
        .expect("Recv error");

        let resp = SipMessage::parse(&buf[..len]).expect("Failed to parse SIP response");
        assert_eq!(resp.status_code(), Some(200));
        assert_eq!(resp.get_header("Call-ID"), Some("opt-ping-1234"));
    }

    #[tokio::test]
    async fn test_inbound_sip_invite_ringing_and_reject() {
        let config = SipConfig {
            server_ip: "127.0.0.1".into(),
            server_port: 5060,
            username: "callee".into(),
            password: "password".into(),
            realm: "127.0.0.1".into(),
            local_ip: "127.0.0.1".into(),
            local_port: 50632,
        };
        let (client, mut event_rx) = SipClient::new(config).await.expect("Failed to create SipClient");

        let remote_sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target_addr: std::net::SocketAddr = "127.0.0.1:50632".parse().unwrap();

        let call_id = "test-inbound-invite-call-id";
        let mut inv = SipMessage::new_request("INVITE", "sip:callee@127.0.0.1:50632");
        inv.set_header("Via", "SIP/2.0/UDP 127.0.0.1:50633;branch=z9hG4bKcallbranch");
        inv.set_header("From", "\"Alice Caller\" <sip:1001@company.com>;tag=caller123");
        inv.set_header("To", "<sip:callee@127.0.0.1:50632>");
        inv.set_header("Call-ID", call_id);
        inv.set_header("CSeq", "100 INVITE");
        inv.set_header("Content-Type", "application/sdp");
        inv.body = "v=0\r\no=alice 1 1 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 10040 RTP/AVP 8\r\n".into();

        remote_sock.send_to(&inv.serialize(), target_addr).await.unwrap();

        // 1. Verify remote caller receives 180 Ringing automatically
        let mut buf = vec![0u8; 1500];
        let (len, _) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            remote_sock.recv_from(&mut buf),
        )
        .await
        .expect("Timeout waiting for 180 Ringing")
        .expect("Recv error");

        let ringing = SipMessage::parse(&buf[..len]).unwrap();
        assert_eq!(ringing.status_code(), Some(180));
        assert_eq!(ringing.get_header("Call-ID"), Some(call_id));

        // 2. Verify SipEvent::IncomingCall is emitted
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), event_rx.recv())
            .await
            .expect("Timeout waiting for IncomingCall event")
            .expect("Channel closed");

        match event {
            SipEvent::IncomingCall { call_id: cid, from, .. } => {
                assert_eq!(cid, call_id);
                assert!(from.contains("1001"));
            }
            _ => panic!("Expected IncomingCall event"),
        }

        // 3. Reject call with 486 Busy Here
        client.reject_incoming_call(call_id, 486).await.unwrap();

        // 4. Verify remote caller receives 486 Busy Here
        let (len2, _) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            remote_sock.recv_from(&mut buf),
        )
        .await
        .expect("Timeout waiting for 486 response")
        .expect("Recv error");

        let reject_resp = SipMessage::parse(&buf[..len2]).unwrap();
        assert_eq!(reject_resp.status_code(), Some(486));
        assert_eq!(reject_resp.get_header("Call-ID"), Some(call_id));
    }

    #[tokio::test]
    async fn test_inbound_sip_invite_answer_and_bye() {
        let config = SipConfig {
            server_ip: "127.0.0.1".into(),
            server_port: 5060,
            username: "callee2".into(),
            password: "password".into(),
            realm: "127.0.0.1".into(),
            local_ip: "127.0.0.1".into(),
            local_port: 50634,
        };
        let (client, mut event_rx) = SipClient::new(config).await.expect("Failed to create SipClient");

        let remote_sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target_addr: std::net::SocketAddr = "127.0.0.1:50634".parse().unwrap();

        let call_id = "test-inbound-answer-call-id";
        let mut inv = SipMessage::new_request("INVITE", "sip:callee2@127.0.0.1:50634");
        inv.set_header("Via", "SIP/2.0/UDP 127.0.0.1:50635;branch=z9hG4bKansbranch");
        inv.set_header("From", "\"Bob Caller\" <sip:2002@company.com>;tag=bobtag");
        inv.set_header("To", "<sip:callee2@127.0.0.1:50634>");
        inv.set_header("Call-ID", call_id);
        inv.set_header("CSeq", "1 INVITE");
        inv.set_header("Content-Type", "application/sdp");
        inv.body = "v=0\r\no=bob 1 1 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 10060 RTP/AVP 8\r\n".into();

        remote_sock.send_to(&inv.serialize(), target_addr).await.unwrap();

        // 1. Drain 180 Ringing
        let mut buf = vec![0u8; 1500];
        let (len, _) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            remote_sock.recv_from(&mut buf),
        )
        .await
        .expect("Timeout waiting for 180 Ringing")
        .expect("Recv error");

        let ringing = SipMessage::parse(&buf[..len]).unwrap();
        assert_eq!(ringing.status_code(), Some(180));

        // 2. Consume IncomingCall event
        let event = event_rx.recv().await.unwrap();
        if let SipEvent::IncomingCall { call_id: cid, .. } = event {
            assert_eq!(cid, call_id);
        } else {
            panic!("Expected IncomingCall event");
        }

        // 3. Answer incoming call
        let active_call = client.answer_incoming_call(call_id).await.expect("Failed to answer incoming call");
        assert_eq!(active_call.call_id, call_id);
        assert!(!active_call.on_hold);
        assert!(active_call.rtp_session.is_some());

        // 4. Remote caller receives 200 OK with SDP
        let (len2, _) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            remote_sock.recv_from(&mut buf),
        )
        .await
        .expect("Timeout waiting for 200 OK")
        .expect("Recv error");

        let ok_resp = SipMessage::parse(&buf[..len2]).unwrap();
        assert_eq!(ok_resp.status_code(), Some(200));
        assert!(String::from_utf8_lossy(&ok_resp.body).contains("m=audio"));

        // 5. Remote caller hangs up via BYE
        let mut bye = SipMessage::new_request("BYE", "sip:callee2@127.0.0.1:50634");
        bye.set_header("Via", "SIP/2.0/UDP 127.0.0.1:50635;branch=z9hG4bKbyebranch");
        bye.set_header("From", "\"Bob Caller\" <sip:2002@company.com>;tag=bobtag");
        bye.set_header("To", ok_resp.get_header("To").unwrap_or("<sip:callee2@127.0.0.1:50634>"));
        bye.set_header("Call-ID", call_id);
        bye.set_header("CSeq", "2 BYE");

        remote_sock.send_to(&bye.serialize(), target_addr).await.unwrap();

        // 6. Verify 200 OK response to BYE
        let (len3, _) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            remote_sock.recv_from(&mut buf),
        )
        .await
        .expect("Timeout waiting for BYE 200 OK")
        .expect("Recv error");

        let bye_resp = SipMessage::parse(&buf[..len3]).unwrap();
        assert_eq!(bye_resp.status_code(), Some(200));

        // 7. Verify CallEnded event is received
        let end_event = event_rx.recv().await.unwrap();
        if let SipEvent::CallEnded { call_id: cid, reason } = end_event {
            assert_eq!(cid, call_id);
            assert!(reason.contains("Remote party hung up"));
        } else {
            panic!("Expected CallEnded event");
        }
    }

    #[tokio::test]
    async fn test_inbound_sip_instant_message() {
        let config = SipConfig {
            server_ip: "127.0.0.1".into(),
            server_port: 5060,
            username: "callee_msg".into(),
            password: "password".into(),
            realm: "127.0.0.1".into(),
            local_ip: "127.0.0.1".into(),
            local_port: 50638,
        };
        let (_client, mut event_rx) = SipClient::new(config).await.expect("Failed to create SipClient");

        let remote_sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target_addr: std::net::SocketAddr = "127.0.0.1:50638".parse().unwrap();

        let call_id = "msg-chat-test-123";
        let mut msg = SipMessage::new_request("MESSAGE", "sip:callee_msg@127.0.0.1:50638");
        msg.set_header("Via", "SIP/2.0/UDP 127.0.0.1:50639;branch=z9hG4bKchatbranch");
        msg.set_header("From", "\"Alice Swyx\" <sip:1001@swyx.local>;tag=alicetag123");
        msg.set_header("To", "<sip:callee_msg@127.0.0.1:50638>");
        msg.set_header("Call-ID", call_id);
        msg.set_header("CSeq", "1 MESSAGE");
        msg.set_header("Content-Type", "text/plain; charset=utf-8");
        msg.body = "Kaffee in der Teekueche? ☕".as_bytes().to_vec();

        remote_sock.send_to(&msg.serialize(), target_addr).await.unwrap();

        // 1. Verify remote sender receives 200 OK immediately
        let mut buf = vec![0u8; 1500];
        let (len, _) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            remote_sock.recv_from(&mut buf),
        )
        .await
        .expect("Timeout waiting for 200 OK")
        .expect("Recv error");

        let ok_resp = SipMessage::parse(&buf[..len]).unwrap();
        assert_eq!(ok_resp.status_code(), Some(200));
        assert_eq!(ok_resp.get_header("Call-ID"), Some(call_id));

        // 2. Verify IncomingMessage event is dispatched
        let ev = tokio::time::timeout(std::time::Duration::from_secs(2), event_rx.recv())
            .await
            .expect("Timeout waiting for IncomingMessage event")
            .expect("Event channel closed");

        if let SipEvent::IncomingMessage { from, content } = ev {
            assert!(from.contains("Alice Swyx") || from.contains("1001"));
            assert_eq!(content, "Kaffee in der Teekueche? ☕");
        } else {
            panic!("Expected IncomingMessage event, got {:?}", ev);
        }
    }

    #[tokio::test]
    async fn test_outbound_sip_instant_message() {
        let server_sock = tokio::net::UdpSocket::bind("127.0.0.1:50640").await.unwrap();

        let config = SipConfig {
            server_ip: "127.0.0.1".into(),
            server_port: 50640,
            username: "alice".into(),
            password: "psk_secret".into(),
            realm: "127.0.0.1".into(),
            local_ip: "127.0.0.1".into(),
            local_port: 50641,
        };
        let (client, _rx) = SipClient::new(config).await.expect("Failed to create SipClient");

        // Spawn mock server responding 200 OK to MESSAGE
        tokio::spawn(async move {
            let mut buf = vec![0u8; 1500];
            let (len, src) = server_sock.recv_from(&mut buf).await.unwrap();
            let req = SipMessage::parse(&buf[..len]).unwrap();
            assert_eq!(req.method(), Some("MESSAGE"));
            assert_eq!(String::from_utf8_lossy(&req.body), "Hallo Bob!");

            let mut ok = SipMessage::new_response(200, "OK");
            if let Some(via) = req.get_header("Via") {
                ok.set_header("Via", via);
            }
            if let Some(from) = req.get_header("From") {
                ok.set_header("From", from);
            }
            if let Some(to) = req.get_header("To") {
                ok.set_header("To", to);
            }
            if let Some(cid) = req.get_header("Call-ID") {
                ok.set_header("Call-ID", cid);
            }
            if let Some(cseq) = req.get_header("CSeq") {
                ok.set_header("CSeq", cseq);
            }
            server_sock.send_to(&ok.serialize(), src).await.unwrap();
        });

        // Send message from client
        let res = client.send_instant_message("1002", "Hallo Bob!").await;
        assert!(res.is_ok(), "Expected send_instant_message to succeed, got {:?}", res);
    }

    #[test]
    fn test_mwi_summary_parser_and_soap_builders() {
        let body = "Messages-Waiting: yes\r\nMessage-Account: sip:100@swyx.local\r\nVoice-Message: 2/5 (1/0)\r\n";
        let (waiting, new_m, old_m, acc) = crate::sip::client::parse_mwi_summary(body);
        assert!(waiting);
        assert_eq!(new_m, 2);
        assert_eq!(old_m, 5);
        assert_eq!(acc.as_deref(), Some("sip:100@swyx.local"));

        let pres_xml = SoapBuilder::build_set_presence(42, 1, "Abwesend", "alice", "tok123");
        assert!(pres_xml.contains("<SetRichPresenceInfo"));
        assert!(pres_xml.contains("<status>1</status>"));
        assert!(pres_xml.contains("<statusText>Abwesend</statusText>"));

        let fwd_xml = SoapBuilder::build_set_call_forwarding(42, true, "01701234567", "alice", "tok123");
        assert!(fwd_xml.contains("<SetCallForwarding"));
        assert!(fwd_xml.contains("<unconditionalForwarding>true</unconditionalForwarding>"));
        assert!(fwd_xml.contains("<unconditionalNumber>01701234567</unconditionalNumber>"));

        let out_devs = crate::rtp::AudioDeviceManager::list_output_devices();
        assert!(!out_devs.is_empty());
        assert_eq!(out_devs[0].id, "default");

        let in_devs = crate::rtp::AudioDeviceManager::list_input_devices();
        assert!(!in_devs.is_empty());
        assert_eq!(in_devs[0].id, "default");
    }

    #[test]
    fn test_swyx_constants_and_register_headers() {
        use crate::sip::client::{SWYX_USER_AGENT, ALLOW_METHODS, SUPPORTED_FEATURES};
        assert!(SWYX_USER_AGENT.contains("SwyxIt!/14.20.0"));
        assert!(SWYX_USER_AGENT.contains("LineMgr=CLMgr.exe"));
        assert!(ALLOW_METHODS.contains("MESSAGE"));
        assert!(ALLOW_METHODS.contains("INVITE"));
        assert!(SUPPORTED_FEATURES.contains("replaces"));
        // Session-Timer und 100rel (PRACK) sind nicht implementiert und dürfen daher
        // nicht beworben werden, sonst bricht die PBX Rufe mangels PRACK/Refresh ab.
        assert!(!SUPPORTED_FEATURES.contains("timer"));
        assert!(!SUPPORTED_FEATURES.contains("100rel"));

        let cfg = SipConfig {
            server_ip: "10.0.0.10".into(),
            server_port: 5060,
            username: "{00000000-0000-0000-0000-000000000001}".into(),
            password: "secret".into(),
            realm: "SWYXPBX01".into(),
            local_ip: "10.0.0.50".into(),
            local_port: 50600,
        };
        let ua = cfg.user_agent();
        assert!(ua.contains("(CtiMaster=No)"), "User-Agent must set CtiMaster=No so SwyxServer delivers incoming INVITEs: {ua}");
        assert!(!ua.contains("CtiMaster=Yes"));
    }

    #[test]
    fn test_ippbx_im_hallo_and_msg_responses() {
        use crate::sip::ippbx_im::{IpPbxIm, TYPE_HALLO, TYPE_MSG, DIR_REQUEST, DIR_RESPONSE_OK};

        // 1. Hallo handshake request
        let hallo = IpPbxIm {
            msg_type: TYPE_HALLO,
            dir: DIR_REQUEST,
            id: 1,
            timestamp: None,
            text: None,
        };
        let hallo_xml = hallo.to_xml();
        assert!(hallo_xml.contains("<Type>1</Type><Dir>1</Dir><ID>1</ID>"));
        assert!(!hallo.is_incoming_text());

        let hallo_ack = hallo.response_ok();
        assert_eq!(hallo_ack.msg_type, TYPE_HALLO);
        assert_eq!(hallo_ack.dir, DIR_RESPONSE_OK);
        assert_eq!(hallo_ack.id, 1);
        assert!(!hallo_ack.is_incoming_text());

        // 2. Chat message request
        let msg = IpPbxIm::new_message(7, "Hallo Kollege!");
        assert_eq!(msg.msg_type, TYPE_MSG);
        assert_eq!(msg.dir, DIR_REQUEST);
        assert!(msg.is_incoming_text());
        assert_eq!(msg.text.as_deref(), Some("Hallo Kollege!"));

        let msg_ack = msg.response_ok();
        assert_eq!(msg_ack.msg_type, TYPE_MSG);
        assert_eq!(msg_ack.dir, DIR_RESPONSE_OK);
        assert_eq!(msg_ack.id, 7);
        assert!(!msg_ack.is_incoming_text());
    }

    #[tokio::test]
    async fn test_outbound_swyx_instant_message_digest_auth() {
        let server_sock = tokio::net::UdpSocket::bind("127.0.0.1:50645").await.unwrap();

        let config = SipConfig {
            server_ip: "127.0.0.1".into(),
            server_port: 50645,
            username: "100".into(),
            password: "test_password_123".into(),
            realm: "127.0.0.1".into(),
            local_ip: "127.0.0.1".into(),
            local_port: 50646,
        };
        let (client, _rx) = SipClient::new(config).await.expect("Failed to create SipClient");

        // Mock Swyx PBX responding with 407 challenge then 200 OK
        tokio::spawn(async move {
            let mut buf = vec![0u8; 2048];

            // Step 1: Receive unauthenticated MESSAGE
            let (len, src) = server_sock.recv_from(&mut buf).await.unwrap();
            let req1 = SipMessage::parse(&buf[..len]).unwrap();
            assert_eq!(req1.method(), Some("MESSAGE"));
            assert_eq!(req1.get_header("X-IPPBXOrigUserID"), Some("42"));
            assert_eq!(req1.get_header("X-IPPBXDestUserID"), Some("43"));
            let user_agent = req1.get_header("User-Agent").unwrap_or("");
            assert!(user_agent.contains("SwyxIt!/14.20.0"));

            // Respond with 407 Proxy Authentication Required
            let mut resp407 = SipMessage::new_response(407, "Proxy Authentication Required");
            if let Some(via) = req1.get_header("Via") { resp407.set_header("Via", via); }
            if let Some(from) = req1.get_header("From") { resp407.set_header("From", from); }
            if let Some(to) = req1.get_header("To") { resp407.set_header("To", to); }
            if let Some(cid) = req1.get_header("Call-ID") { resp407.set_header("Call-ID", cid); }
            if let Some(cseq) = req1.get_header("CSeq") { resp407.set_header("CSeq", cseq); }
            resp407.set_header(
                "Proxy-Authenticate",
                "Digest realm=\"127.0.0.1\", nonce=\"mocknonce12345\", qop=\"auth\", algorithm=MD5",
            );
            server_sock.send_to(&resp407.serialize(), src).await.unwrap();

            // Step 2: Receive authenticated MESSAGE
            let (len2, src2) = server_sock.recv_from(&mut buf).await.unwrap();
            let req2 = SipMessage::parse(&buf[..len2]).unwrap();
            assert_eq!(req2.method(), Some("MESSAGE"));
            let auth = req2.get_header("Proxy-Authorization").expect("Missing Proxy-Authorization");
            assert!(auth.contains("username=\"100\""));
            assert!(auth.contains("nonce=\"mocknonce12345\""));
            assert_eq!(req2.get_header("X-IPPBXOrigUserID"), Some("42"));
            assert_eq!(req2.get_header("X-IPPBXDestUserID"), Some("43"));

            // Respond with 200 OK
            let mut ok200 = SipMessage::new_response(200, "OK");
            if let Some(via) = req2.get_header("Via") { ok200.set_header("Via", via); }
            if let Some(from) = req2.get_header("From") { ok200.set_header("From", from); }
            if let Some(to) = req2.get_header("To") { ok200.set_header("To", to); }
            if let Some(cid) = req2.get_header("Call-ID") { ok200.set_header("Call-ID", cid); }
            if let Some(cseq) = req2.get_header("CSeq") { ok200.set_header("CSeq", cseq); }
            server_sock.send_to(&ok200.serialize(), src2).await.unwrap();
        });

        let res = client
            .send_swyx_instant_message("101", "Test offline message", Some(42), Some(43))
            .await;
        assert!(res.is_ok(), "Expected authenticated Swyx IM to succeed, got {:?}", res);
    }
}

