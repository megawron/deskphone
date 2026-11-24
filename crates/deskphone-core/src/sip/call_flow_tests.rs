//! Loopback call-flow tests against a minimal PBX mock (behaves like SwyxServer:
//! `100 Trying` before anything else, REGISTER/INVITE without digest challenge).
//! These tests never touch a real network or PBX.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::net::UdpSocket;

use super::client::{SipClient, SipConfig, SipEvent};
use super::message::SipMessage;
use crate::rtp::RtpPacket;

const TO: Duration = Duration::from_secs(3);

fn config(server_port: u16, local_port: u16) -> SipConfig {
    SipConfig {
        server_ip: "127.0.0.1".into(),
        server_port,
        username: "{11111111-2222-4333-8444-555555555555}".into(),
        password: "psk-secret".into(),
        realm: "127.0.0.1".into(),
        local_ip: "127.0.0.1".into(),
        local_port,
    }
}

fn free_udp_port() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT_PORT: AtomicU16 = AtomicU16::new(41200);
    loop {
        let candidate = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
        if std::net::UdpSocket::bind(("127.0.0.1", candidate)).is_ok() {
            return candidate;
        }
    }
}

async fn recv_msg(sock: &UdpSocket) -> (SipMessage, std::net::SocketAddr) {
    let mut buf = vec![0u8; 65535];
    let (len, src) = tokio::time::timeout(TO, sock.recv_from(&mut buf))
        .await
        .expect("timeout waiting for SIP message")
        .unwrap();
    (SipMessage::parse(&buf[..len]).unwrap(), src)
}

/// Receives the next request with the given method (skips e.g. INVITE retransmissions)
async fn recv_request(sock: &UdpSocket, method: &str) -> (SipMessage, std::net::SocketAddr) {
    loop {
        let (m, src) = recv_msg(sock).await;
        if m.method() == Some(method) {
            return (m, src);
        }
    }
}

fn reply(req: &SipMessage, code: u16, reason: &str, to_tag: Option<&str>) -> SipMessage {
    let mut r = SipMessage::new_response(code, reason);
    for h in ["Via", "From", "Call-ID", "CSeq"] {
        if let Some(v) = req.get_header(h) {
            r.set_header(h, v);
        }
    }
    let to = req.get_header("To").unwrap_or("").to_string();
    match to_tag {
        Some(t) if !to.contains("tag=") => r.set_header("To", &format!("{};tag={}", to, t)),
        _ => r.set_header("To", &to),
    }
    r
}

fn offered_port(sdp: &[u8]) -> u16 {
    let s = String::from_utf8_lossy(sdp);
    let line = s.lines().find(|l| l.starts_with("m=audio")).expect("no m=audio");
    line.split_whitespace().nth(1).unwrap().parse().unwrap()
}

fn pbx_sdp(rtp_port: u16) -> Vec<u8> {
    format!(
        "v=0\r\no=SwyxServer 1 1 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n\
         m=audio {rtp_port} RTP/AVP 8 101\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:101 telephone-event/8000\r\na=sendrecv\r\n"
    )
    .into_bytes()
}

#[tokio::test]
async fn outbound_call_with_100_trying_and_no_challenge() {
    let pbx = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let pbx_port = pbx.local_addr().unwrap().port();
    let pbx_rtp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let pbx_rtp_port = pbx_rtp.local_addr().unwrap().port();

    let (client, mut events) = SipClient::new(config(pbx_port, free_udp_port())).await.unwrap();
    let client = Arc::new(client);

    let dial_client = client.clone();
    let dial = tokio::spawn(async move { dial_client.dial_with_call_id("cid-out-1", "102").await });

    // INVITE
    let (inv, src) = recv_request(&pbx, "INVITE").await;
    assert!(super::client::ALLOW_METHODS.contains("INVITE"));
    let SipMessage { ref msg_type, .. } = inv;
    if let super::message::SipMessageType::Request { uri, .. } = msg_type {
        assert_eq!(uri, "sip:102@127.0.0.1");
    }
    let from = inv.get_header("From").unwrap();
    assert!(from.contains("%7b11111111"), "From must use the registered (encoded) binding: {from}");
    assert!(!from.contains('{'));
    assert_eq!(inv.get_header("Call-ID"), Some("cid-out-1"));
    assert!(!String::from_utf8_lossy(&inv.body).contains("opus"));
    let sdp_port = offered_port(&inv.body);
    let (inv_cseq, _) = inv.cseq().unwrap();

    // Swyx-like: 100 Trying, 180 Ringing, 200 OK
    pbx.send_to(&reply(&inv, 100, "Trying", None).serialize(), src).await.unwrap();
    pbx.send_to(&reply(&inv, 180, "Ringing", Some("pbxtag1")).serialize(), src).await.unwrap();
    let mut ok = reply(&inv, 200, "OK", Some("pbxtag1"));
    ok.set_header("Contact", &format!("<sip:102@127.0.0.1:{}>", pbx_port));
    ok.set_header("Content-Type", "application/sdp");
    ok.body = pbx_sdp(pbx_rtp_port);
    pbx.send_to(&ok.serialize(), src).await.unwrap();

    // ACK for 2xx with same CSeq number
    let (ack, _) = recv_request(&pbx, "ACK").await;
    assert_eq!(ack.cseq().unwrap(), (inv_cseq, "ACK".to_string()));
    assert!(ack.get_header("To").unwrap().contains("tag=pbxtag1"));

    let call = tokio::time::timeout(TO, dial).await.unwrap().unwrap().expect("dial failed");
    assert_eq!(call.call_id, "cid-out-1");
    assert_eq!(call.remote_tag.as_deref(), Some("pbxtag1"));

    // RTP socket advertised in the SDP offer must be the one used for the call
    let rtp = call.rtp_session.clone().unwrap();
    assert_eq!(rtp.local_port, sdp_port);

    // Ringing event was emitted for the same call id
    let ev = tokio::time::timeout(TO, events.recv()).await.unwrap().unwrap();
    assert!(matches!(ev, SipEvent::CallRinging { ref call_id } if call_id == "cid-out-1"), "{ev:?}");

    // Client sends RTP (PCMA) to the PBX media address from the SDP answer
    let mut buf = vec![0u8; 2048];
    let (len, rtp_src) = tokio::time::timeout(TO, pbx_rtp.recv_from(&mut buf)).await.unwrap().unwrap();
    let pkt = RtpPacket::parse(Bytes::copy_from_slice(&buf[..len])).unwrap();
    assert_eq!(pkt.payload_type, 8);
    assert_eq!(pkt.payload.len(), 160);
    assert_eq!(rtp_src.port(), sdp_port, "RTP must be sent from the advertised port");

    // Hangup -> BYE inside the dialog
    let hc = client.clone();
    let call2 = call.clone();
    let hang = tokio::spawn(async move { hc.hangup(&call2).await });
    let (bye, bsrc) = recv_request(&pbx, "BYE").await;
    assert!(bye.get_header("To").unwrap().contains("tag=pbxtag1"));
    assert!(bye.get_header("From").unwrap().contains(&format!("tag={}", call.local_tag)));
    assert_eq!(bye.get_header("Call-ID"), Some("cid-out-1"));
    pbx.send_to(&reply(&bye, 200, "OK", None).serialize(), bsrc).await.unwrap();
    tokio::time::timeout(TO, hang).await.unwrap().unwrap().unwrap();
}

#[tokio::test]
async fn outbound_call_cancelled_while_ringing() {
    let pbx = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let pbx_port = pbx.local_addr().unwrap().port();
    let (client, mut events) = SipClient::new(config(pbx_port, free_udp_port())).await.unwrap();
    let client = Arc::new(client);

    let dc = client.clone();
    let dial = tokio::spawn(async move { dc.dial_with_call_id("cid-cancel", "102").await });

    let (inv, src) = recv_request(&pbx, "INVITE").await;
    pbx.send_to(&reply(&inv, 100, "Trying", None).serialize(), src).await.unwrap();
    pbx.send_to(&reply(&inv, 180, "Ringing", Some("t2")).serialize(), src).await.unwrap();

    let ev = tokio::time::timeout(TO, events.recv()).await.unwrap().unwrap();
    assert!(matches!(ev, SipEvent::CallRinging { .. }));

    assert!(client.cancel_outgoing("cid-cancel").await);

    let (cancel, csrc) = recv_request(&pbx, "CANCEL").await;
    assert_eq!(cancel.get_header("Via"), inv.get_header("Via"), "CANCEL must reuse the INVITE branch");
    let (n, _) = inv.cseq().unwrap();
    assert_eq!(cancel.cseq().unwrap(), (n, "CANCEL".to_string()));
    pbx.send_to(&reply(&cancel, 200, "OK", Some("t2")).serialize(), csrc).await.unwrap();
    pbx.send_to(&reply(&inv, 487, "Request Terminated", Some("t2")).serialize(), src).await.unwrap();

    let (ack, _) = recv_request(&pbx, "ACK").await;
    assert_eq!(ack.get_header("Via"), inv.get_header("Via"), "ACK for 487 must reuse the INVITE branch");

    let res = tokio::time::timeout(TO, dial).await.unwrap().unwrap();
    let err = res.expect_err("cancelled call must fail");
    assert!(err.to_string().contains("abgebrochen"), "{err}");
}

#[tokio::test]
async fn outbound_call_busy_reports_reason() {
    let pbx = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let pbx_port = pbx.local_addr().unwrap().port();
    let (client, _events) = SipClient::new(config(pbx_port, free_udp_port())).await.unwrap();
    let client = Arc::new(client);

    let dc = client.clone();
    let dial = tokio::spawn(async move { dc.dial_with_call_id("cid-busy", "102").await });
    let (inv, src) = recv_request(&pbx, "INVITE").await;
    pbx.send_to(&reply(&inv, 100, "Trying", None).serialize(), src).await.unwrap();
    pbx.send_to(&reply(&inv, 486, "Busy Here", Some("b1")).serialize(), src).await.unwrap();
    let _ = recv_request(&pbx, "ACK").await;
    let err = tokio::time::timeout(TO, dial).await.unwrap().unwrap().expect_err("busy");
    assert!(err.to_string().contains("besetzt"), "{err}");
}

#[tokio::test]
async fn outbound_call_with_proxy_auth_challenge_after_trying() {
    let pbx = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let pbx_port = pbx.local_addr().unwrap().port();
    let pbx_rtp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let (client, _events) = SipClient::new(config(pbx_port, free_udp_port())).await.unwrap();
    let client = Arc::new(client);

    let dc = client.clone();
    let dial = tokio::spawn(async move { dc.dial_with_call_id("cid-auth", "102").await });

    let (inv1, src) = recv_request(&pbx, "INVITE").await;
    pbx.send_to(&reply(&inv1, 100, "Trying", None).serialize(), src).await.unwrap();
    let mut chal = reply(&inv1, 407, "Proxy Authentication Required", Some("a1"));
    chal.set_header(
        "Proxy-Authenticate",
        "Digest realm=\"127.0.0.1\", nonce=\"n0nce\", opaque=\"op4que\", qop=\"auth\", algorithm=MD5",
    );
    pbx.send_to(&chal.serialize(), src).await.unwrap();
    let (ack1, _) = recv_request(&pbx, "ACK").await;
    assert_eq!(ack1.get_header("Via"), inv1.get_header("Via"));

    let (inv2, src2) = recv_request(&pbx, "INVITE").await;
    let auth = inv2.get_header("Proxy-Authorization").expect("missing Proxy-Authorization");
    assert!(auth.contains("opaque=\"op4que\""));
    assert!(auth.contains("uri=\"sip:102@127.0.0.1\""));
    assert!(inv2.cseq().unwrap().0 > inv1.cseq().unwrap().0);

    let mut ok = reply(&inv2, 200, "OK", Some("a2"));
    ok.set_header("Content-Type", "application/sdp");
    ok.body = pbx_sdp(pbx_rtp.local_addr().unwrap().port());
    pbx.send_to(&ok.serialize(), src2).await.unwrap();
    let _ = recv_request(&pbx, "ACK").await;

    let call = tokio::time::timeout(TO, dial).await.unwrap().unwrap().unwrap();
    assert_eq!(call.remote_tag.as_deref(), Some("a2"));
    if let Some(r) = call.rtp_session {
        r.stop();
    }
}

fn incoming_invite(call_id: &str, client_port: u16, rtp_port: u16) -> SipMessage {
    let mut inv = SipMessage::new_request("INVITE", &format!("sip:%7bguid%7d@127.0.0.1:{client_port}"));
    inv.set_header("Via", "SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bKin1");
    inv.set_header("From", "\"Musterfrau E. 102\" <sip:102@127.0.0.1>;tag=caller102");
    inv.set_header("To", "<sip:101@127.0.0.1>");
    inv.set_header("Call-ID", call_id);
    inv.set_header("CSeq", "7 INVITE");
    inv.set_header("Contact", "<sip:102@127.0.0.1:5060>");
    inv.set_header("Content-Type", "application/sdp");
    inv.body = format!(
        "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\n\
         m=audio {rtp_port} RTP/AVP 111 0 8 101\r\na=rtpmap:111 opus/48000/2\r\na=rtpmap:101 telephone-event/8000\r\n"
    )
    .into_bytes();
    inv
}

#[tokio::test]
async fn incoming_call_answer_reinvite_and_retransmissions() {
    let client_port = free_udp_port();
    let (client, mut events) = SipClient::new(config(5060, client_port)).await.unwrap();
    let pbx = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let pbx_rtp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target: std::net::SocketAddr = format!("127.0.0.1:{client_port}").parse().unwrap();

    let inv = incoming_invite("cid-in-1", client_port, pbx_rtp.local_addr().unwrap().port());
    pbx.send_to(&inv.serialize(), target).await.unwrap();
    let (ringing, _) = recv_msg(&pbx).await;
    assert_eq!(ringing.status_code(), Some(180));
    let tag180 = ringing.to_tag().expect("180 must carry a To-tag");

    // INVITE retransmission -> 180 again, but NO second IncomingCall event
    pbx.send_to(&inv.serialize(), target).await.unwrap();
    let (ringing2, _) = recv_msg(&pbx).await;
    assert_eq!(ringing2.status_code(), Some(180));
    assert_eq!(ringing2.to_tag().as_deref(), Some(tag180.as_str()));

    let ev = tokio::time::timeout(TO, events.recv()).await.unwrap().unwrap();
    assert!(matches!(ev, SipEvent::IncomingCall { ref call_id, .. } if call_id == "cid-in-1"));
    assert!(
        tokio::time::timeout(Duration::from_millis(300), events.recv()).await.is_err(),
        "retransmission must not raise a second IncomingCall"
    );

    let call = client.answer_incoming_call("cid-in-1").await.unwrap();
    assert_eq!(call.target_number, "102");
    let (ok, _) = recv_msg(&pbx).await;
    assert_eq!(ok.status_code(), Some(200));
    assert_eq!(ok.to_tag().as_deref(), Some(tag180.as_str()), "200 OK must reuse the 180 To-tag");
    let body = String::from_utf8_lossy(&ok.body).to_string();
    // Answer contains exactly one audio codec we can encode (PCMU was preferred over opus)
    assert!(body.contains("RTP/AVP 0 101"), "{body}");
    assert!(!body.contains("opus"));

    // ACK stops 200 retransmissions
    let mut ack = SipMessage::new_request("ACK", &format!("sip:x@127.0.0.1:{client_port}"));
    ack.set_header("Via", "SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bKack1");
    ack.set_header("From", inv.get_header("From").unwrap());
    ack.set_header("To", ok.get_header("To").unwrap());
    ack.set_header("Call-ID", "cid-in-1");
    ack.set_header("CSeq", "7 ACK");
    pbx.send_to(&ack.serialize(), target).await.unwrap();

    // re-INVITE from the PBX moving media to a new port
    let new_rtp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut reinv = incoming_invite("cid-in-1", client_port, new_rtp.local_addr().unwrap().port());
    reinv.set_header("To", ok.get_header("To").unwrap());
    reinv.set_header("CSeq", "8 INVITE");
    reinv.set_header("Via", "SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bKreinv");
    pbx.send_to(&reinv.serialize(), target).await.unwrap();
    let resp = loop {
        let (m, _) = recv_msg(&pbx).await;
        if m.cseq().map(|(n, _)| n) == Some(8) {
            break m;
        }
    };
    assert_eq!(resp.status_code(), Some(200));
    assert!(String::from_utf8_lossy(&resp.body).contains("m=audio"));

    // RTP now flows to the new address
    let mut buf = vec![0u8; 2048];
    let got = tokio::time::timeout(TO, new_rtp.recv_from(&mut buf)).await;
    assert!(got.is_ok(), "RTP must follow the re-INVITE");

    // Local hangup sends BYE with swapped From/To
    let hang_call = call.clone();
    let client = Arc::new(client);
    let hc = client.clone();
    let hang = tokio::spawn(async move { hc.hangup(&hang_call).await });
    // BYE goes to the configured server (port 5060) - not observable here, just ensure no panic
    let _ = tokio::time::timeout(Duration::from_secs(10), hang).await;
    assert!(call.local_header.contains(&format!("tag={tag180}")));
    assert!(call.remote_header.contains("tag=caller102"));
    assert_eq!(call.remote_target, "sip:102@127.0.0.1:5060");
}

#[tokio::test]
async fn incoming_call_cancelled_by_caller() {
    let client_port = free_udp_port();
    let (_client, mut events) = SipClient::new(config(5060, client_port)).await.unwrap();
    let pbx = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target: std::net::SocketAddr = format!("127.0.0.1:{client_port}").parse().unwrap();

    let inv = incoming_invite("cid-in-cancel", client_port, 40000);
    pbx.send_to(&inv.serialize(), target).await.unwrap();
    let (ringing, _) = recv_msg(&pbx).await;
    let tag = ringing.to_tag().unwrap();
    let _ = events.recv().await;

    let mut cancel = SipMessage::new_request("CANCEL", "sip:x@127.0.0.1");
    cancel.set_header("Via", inv.get_header("Via").unwrap());
    cancel.set_header("From", inv.get_header("From").unwrap());
    cancel.set_header("To", inv.get_header("To").unwrap());
    cancel.set_header("Call-ID", "cid-in-cancel");
    cancel.set_header("CSeq", "7 CANCEL");
    cancel.set_header("Reason", "SIP;cause=200;text=\"Call completed elsewhere\"");
    pbx.send_to(&cancel.serialize(), target).await.unwrap();

    let mut got_200 = false;
    let mut got_487 = false;
    for _ in 0..2 {
        let (m, _) = recv_msg(&pbx).await;
        match m.status_code() {
            Some(200) => got_200 = true,
            Some(487) => {
                got_487 = true;
                assert_eq!(m.to_tag().as_deref(), Some(tag.as_str()));
                assert_eq!(m.cseq().unwrap(), (7, "INVITE".to_string()));
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(got_200 && got_487);
    let ev = tokio::time::timeout(TO, events.recv()).await.unwrap().unwrap();
    assert!(matches!(ev, SipEvent::CallEnded { ref reason, .. } if reason == "Answered elsewhere"), "{ev:?}");
}

#[test]
fn parser_keeps_all_via_headers_and_expands_compact_form() {
    let raw = b"INVITE sip:a@b SIP/2.0\r\nv: SIP/2.0/UDP 1.1.1.1;branch=z9hG4bK1\r\nVia: SIP/2.0/UDP 2.2.2.2;branch=z9hG4bK2\r\ni: abc\r\nl: 4\r\n\r\nbodyEXTRA";
    let m = SipMessage::parse(raw).unwrap();
    assert_eq!(
        m.get_header("Via"),
        Some("SIP/2.0/UDP 1.1.1.1;branch=z9hG4bK1, SIP/2.0/UDP 2.2.2.2;branch=z9hG4bK2")
    );
    assert_eq!(m.call_id(), Some("abc"));
    assert_eq!(m.body, b"body");
}

#[tokio::test]
async fn swyx_outgoing_invite_uses_realm_ppi_sendingcomplete_and_x_ippbx_headers() {
    use super::client::SwyxLineContext;
    use super::message::SipMessageType;

    let pbx = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let pbx_rtp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let pbx_port = pbx.local_addr().unwrap().port();
    let client_port = free_udp_port();

    let mut cfg = config(pbx_port, client_port);
    cfg.realm = "SWYXPBX01".into();
    let (client, _events) = SipClient::new(cfg).await.unwrap();
    client.set_swyx_context(SwyxLineContext {
        own_extension: "101".into(),
        display_name: Some("Mustermann M. 101".into()),
        user_id: 42,
        site_id: 1,
    });
    let client = Arc::new(client);

    // 1. REGISTER uses sip:<server_ip>:<server_port> as Request-URI and @SWYXPBX01 in To/From
    // Note: register_with_expiry returns granted seconds; register() wraps it
    let c_reg = client.clone();
    let reg_task = tokio::spawn(async move { c_reg.register_with_expiry(3600).await });
    let (reg, src) = recv_request(&pbx, "REGISTER").await;
    assert_eq!(
        reg.msg_type,
        SipMessageType::Request {
            method: "REGISTER".into(),
            uri: format!("sip:127.0.0.1:{pbx_port}"),
        }
    );
    assert!(reg.get_header("To").unwrap().ends_with("@SWYXPBX01>"));
    let mut reg_ok = reply(&reg, 200, "OK", Some("rtag"));
    // Simulate SwyxServer returning multiple Contact headers (an older TCP CSTA binding first, our UDP binding second)
    let raw_reg_ok = format!(
        "SIP/2.0 200 OK\r\nVia: {}\r\nFrom: {}\r\nTo: {}\r\nCall-ID: {}\r\nCSeq: {}\r\n\
         Contact: <sip:%7bother%7d@127.0.0.1:65007;transport=tcp>;expires=83\r\n\
         Contact: {};expires=120\r\nContent-Length: 0\r\n\r\n",
        reg_ok.get_header("Via").unwrap(),
        reg_ok.get_header("From").unwrap(),
        reg_ok.get_header("To").unwrap(),
        reg_ok.get_header("Call-ID").unwrap(),
        reg_ok.get_header("CSeq").unwrap(),
        reg.get_header("Contact").unwrap(),
    );
    let _ = &mut reg_ok;
    pbx.send_to(raw_reg_ok.as_bytes(), src).await.unwrap();
    let granted = tokio::time::timeout(TO, reg_task).await.unwrap().unwrap().unwrap();
    assert_eq!(granted, 120, "must extract expires=120 from the second Contact header");

    // 2. SUBSCRIBE Event: x-ippbx + NOTIFY Event: x-ippbx
    let c_sub = client.clone();
    let sub_task = tokio::spawn(async move { c_sub.subscribe_swyx_events(3600).await });
    let (sub, src_sub) = recv_request(&pbx, "SUBSCRIBE").await;
    assert_eq!(sub.get_header("Event"), Some("x-ippbx"));
    assert!(sub.get_header("To").unwrap().ends_with("@SWYXPBX01>"));
    let mut sub_ok = reply(&sub, 200, "OK", Some("subtag"));
    sub_ok.set_header("Event", "x-ippbx");
    pbx.send_to(&sub_ok.serialize(), src_sub).await.unwrap();
    tokio::time::timeout(TO, sub_task).await.unwrap().unwrap().unwrap();

    // 3. Outgoing INVITE to 102 carries exact CLMgr.exe headers & handles 407 challenge
    let c_dial = client.clone();
    let dial_task = tokio::spawn(async move { c_dial.dial_with_call_id("swyx-out-102", "102").await });
    let (inv1, src_inv1) = recv_request(&pbx, "INVITE").await;
    let expected_ruri = format!("sip:102@127.0.0.1:{pbx_port}");
    assert_eq!(
        inv1.msg_type,
        SipMessageType::Request {
            method: "INVITE".into(),
            uri: expected_ruri.clone(),
        }
    );
    assert_eq!(
        inv1.get_header("To"),
        Some("<sip:102@SWYXPBX01;user=phone;x-sendingcomplete>")
    );
    assert!(inv1.get_header("From").unwrap().contains("@SWYXPBX01>;tag="));
    assert_eq!(
        inv1.get_header("P-Preferred-Identity"),
        Some("\"Mustermann M. 101\" <sip:101@SWYXPBX01;user=phone>")
    );
    assert_eq!(inv1.get_header("P-Early-Media"), Some("supported"));
    assert_eq!(inv1.get_header("X-IPPBXCallType"), Some("0"));
    assert_eq!(inv1.get_header("X-IPPBXOrigUserID"), Some("42"));
    assert_eq!(inv1.get_header("X-IPPBXOrigSiteID"), Some("1"));
    let wire_inv1 = String::from_utf8(inv1.serialize()).unwrap();
    for exact_hdr in [
        "P-Preferred-Identity: \"Mustermann M. 101\" <sip:101@SWYXPBX01;user=phone>\r\n",
        "P-Early-Media: supported\r\n",
        "X-IPPBXCallType: 0\r\n",
        "X-IPPBXOrigUserID: 42\r\n",
        "X-IPPBXOrigSiteID: 1\r\n",
    ] {
        assert!(wire_inv1.contains(exact_hdr), "missing exact wire header {exact_hdr:?} in:\n{wire_inv1}");
    }

    let mut ch = reply(&inv1, 407, "Proxy Authentication Required", Some("ch102"));
    ch.set_header(
        "Proxy-Authenticate",
        "Digest realm=\"SWYXPBX01\", nonce=\"n102\", qop=\"auth\"",
    );
    pbx.send_to(&ch.serialize(), src_inv1).await.unwrap();
    let _ = recv_request(&pbx, "ACK").await;

    let (inv2, src_inv2) = recv_request(&pbx, "INVITE").await;
    let pauth = inv2.get_header("Proxy-Authorization").unwrap();
    assert!(
        pauth.contains(&format!("uri=\"{expected_ruri}\"")),
        "Digest URI must match INVITE Request-URI: {pauth}"
    );

    let mut ok = reply(&inv2, 200, "OK", Some("swyx102"));
    ok.set_header("Contact", &format!("<sip:102@127.0.0.1:{pbx_port}>"));
    ok.set_header("Content-Type", "application/sdp");
    ok.body = pbx_sdp(pbx_rtp.local_addr().unwrap().port());
    pbx.send_to(&ok.serialize(), src_inv2).await.unwrap();
    let _ = recv_request(&pbx, "ACK").await;

    let call = tokio::time::timeout(TO, dial_task).await.unwrap().unwrap().unwrap();
    if let Some(r) = call.rtp_session {
        r.stop();
    }
}

#[tokio::test]
async fn swyx_incoming_invite_resolves_caller_ext_and_called_group_while_preserving_wire_dialog() {
    use super::client::SwyxLineContext;

    let client_port = free_udp_port();
    let mut cfg = config(5060, client_port);
    cfg.realm = "SWYXPBX01".into();
    let (client, mut events) = SipClient::new(cfg).await.unwrap();
    client.set_swyx_context(SwyxLineContext {
        own_extension: "101".into(),
        display_name: Some("Mustermann M. 101".into()),
        user_id: 42,
        site_id: 1,
    });

    let pbx = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let pbx_rtp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target: std::net::SocketAddr = format!("127.0.0.1:{client_port}").parse().unwrap();

    let wire_from = "\"Musterfrau E. 102\" <sip:%7bpeer-guid%7d@SWYXPBX01;x-ippbx-ext=102>;tag=swyxin1";
    let wire_to = "<sip:%7b11111111-0000-0000-0000-000000000001%7d@SWYXPBX01>";
    let mut inv = incoming_invite("cid-swyx-in", client_port, pbx_rtp.local_addr().unwrap().port());
    inv.set_header("From", wire_from);
    inv.set_header("To", wire_to);
    inv.set_header("X-IPPBXCalledExtension", "200");
    inv.set_header("X-IPPBXCalledName", "Support");

    pbx.send_to(&inv.serialize(), target).await.unwrap();
    let (ringing, _) = recv_msg(&pbx).await;
    assert_eq!(ringing.status_code(), Some(180));
    assert_eq!(ringing.get_header("From"), Some(wire_from));
    assert!(ringing.get_header("To").unwrap().starts_with(wire_to));

    let ev = tokio::time::timeout(TO, events.recv()).await.unwrap().unwrap();
    match ev {
        SipEvent::IncomingCall { from, to, .. } => {
            assert_eq!(from, "\"Musterfrau E. 102\" <sip:102@SWYXPBX01>");
            assert_eq!(to, "\"Support\" <sip:200@SWYXPBX01>");
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let call = client.answer_incoming_call("cid-swyx-in").await.unwrap();
    assert_eq!(call.target_number, "102");
    let (ok, _) = recv_msg(&pbx).await;
    assert_eq!(ok.status_code(), Some(200));
    assert_eq!(ok.get_header("From"), Some(wire_from));
    assert!(ok.get_header("To").unwrap().starts_with(wire_to));
    if let Some(ref r) = call.rtp_session {
        r.stop();
    }
}

#[tokio::test]
async fn swyx_cti_xml_control_message_is_acknowledged_but_not_treated_as_chat() {
    let client_port = free_udp_port();
    let mut cfg = config(5060, client_port);
    cfg.realm = "SWYXPBX01".into();
    let (_client, mut events) = SipClient::new(cfg).await.unwrap();

    let pbx = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target: std::net::SocketAddr = format!("127.0.0.1:{client_port}").parse().unwrap();

    // 1. Send exact Swyx CTI pairing release XML MESSAGE -> must get 200 OK, NO chat event!
    let mut cti_msg = SipMessage::new_request(
        "MESSAGE",
        &format!("sip:%7B22222222-3333-4444-8555-666666666666%7D@127.0.0.1:{client_port};transport=udp"),
    );
    cti_msg.set_header("Via", "SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bKcti1");
    cti_msg.set_header(
        "From",
        "<sip:%7B22222222-3333-4444-8555-666666666666%7D@SWYXPBX01>;tag=0677956b",
    );
    cti_msg.set_header(
        "To",
        "<sip:%7B22222222-3333-4444-8555-666666666666%7D@SWYXPBX01>",
    );
    cti_msg.set_header("Call-ID", "cti-pairing-release-1");
    cti_msg.set_header("CSeq", "1 MESSAGE");
    cti_msg.set_header("Content-Type", "application/x-ippbxcti+xml;charset=utf-8");
    cti_msg.body = br#"<?xml version="1.0" encoding="UTF-16" standalone="no" ?><messages><CtiPairingRelease/></messages>"#.to_vec();

    pbx.send_to(&cti_msg.serialize(), target).await.unwrap();
    let (ok, _) = recv_msg(&pbx).await;
    assert_eq!(ok.status_code(), Some(200));

    // 2. Follow with a real plain-text chat MESSAGE -> must arrive as the VERY NEXT event!
    let mut txt_msg = SipMessage::new_request("MESSAGE", &format!("sip:101@127.0.0.1:{client_port}"));
    txt_msg.set_header("Via", "SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bKtxt2");
    txt_msg.set_header("From", "\"Musterfrau E. 102\" <sip:102@SWYXPBX01>;tag=txt2");
    txt_msg.set_header("To", "<sip:101@SWYXPBX01>");
    txt_msg.set_header("Call-ID", "chat-msg-2");
    txt_msg.set_header("CSeq", "2 MESSAGE");
    txt_msg.set_header("Content-Type", "text/plain;charset=utf-8");
    txt_msg.body = b"Hallo Max!".to_vec();

    pbx.send_to(&txt_msg.serialize(), target).await.unwrap();
    let (ok2, _) = recv_msg(&pbx).await;
    assert_eq!(ok2.status_code(), Some(200));

    let ev = tokio::time::timeout(TO, events.recv()).await.unwrap().unwrap();
    match ev {
        SipEvent::IncomingMessage { from, content } => {
            assert!(from.contains("102"));
            assert_eq!(content, "Hallo Max!");
        }
        other => panic!("unexpected event: {other:?}"),
    }
}
