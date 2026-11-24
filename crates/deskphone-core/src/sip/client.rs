use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use anyhow::{anyhow, Result};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Mutex};
use tokio::time::Instant;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::rtp::RtpSession;
use super::digest::DigestAuth;
use super::message::{extract_uri, extract_user, SipMessage, SipMessageType};
use super::sdp::{MediaDirection, SdpSession};

pub const SWYX_USER_AGENT: &str = concat!(
    "SwyxIt!/14.20.0 (Language=Deutsch) (LineMgr=CLMgr.exe/14.21) (OS=Linux) (Deskphone/",
    env!("CARGO_PKG_VERSION"),
    ")"
);
pub const ALLOW_METHODS: &str =
    "INVITE, ACK, CANCEL, BYE, NOTIFY, REFER, OPTIONS, INFO, SUBSCRIBE, MESSAGE, UPDATE";
/// Only advertise extensions that are actually implemented. Session timers (`timer`)
/// and reliable provisional responses (`100rel` / PRACK) are NOT implemented.
pub const SUPPORTED_FEATURES: &str = "replaces";

/// Maximum ringing time for outgoing calls before we CANCEL ourselves
const MAX_RING_TIME: Duration = Duration::from_secs(180);
/// RFC 3261 Timer T1 (RTT estimate)
const T1: Duration = Duration::from_millis(500);
/// RFC 3261 Timer T2
const T2: Duration = Duration::from_secs(4);

/// Generates a globally unique SIP Call-ID (usable before the call is placed,
/// so UIs can correlate all events of an outgoing call).
pub fn generate_call_id() -> String {
    Uuid::new_v4().simple().to_string()
}

fn new_branch() -> String {
    format!("z9hG4bK{}", Uuid::new_v4().simple())
}

fn new_tag() -> String {
    format!("{:x}", rand::random::<u64>())
}

fn header_has_tag(header: &str) -> bool {
    // Tag-Parameter stehen hinter der URI (nach '>' bzw. ohne spitze Klammern nach ';')
    let params = match header.rfind('>') {
        Some(idx) => &header[idx + 1..],
        None => header,
    };
    params.split(';').any(|p| p.trim().starts_with("tag="))
}

fn tag_of(header: &str) -> Option<String> {
    let params = match header.rfind('>') {
        Some(idx) => &header[idx + 1..],
        None => header,
    };
    params
        .split(';')
        .find_map(|p| p.trim().strip_prefix("tag=").map(|t| t.to_string()))
}

fn add_tag(header: &str, tag: &str) -> String {
    if header_has_tag(header) {
        header.to_string()
    } else {
        format!("{};tag={}", header, tag)
    }
}

fn request_uri(m: &SipMessage) -> String {
    match &m.msg_type {
        SipMessageType::Request { uri, .. } => uri.clone(),
        _ => String::new(),
    }
}

fn reason_phrase(m: &SipMessage) -> String {
    match &m.msg_type {
        SipMessageType::Response { reason, .. } => reason.clone(),
        _ => String::new(),
    }
}

fn matches_cseq(m: &SipMessage, cseq: u32, method: &str) -> bool {
    m.cseq()
        .map(|(n, me)| n == cseq && me.eq_ignore_ascii_case(method))
        .unwrap_or(false)
}

/// Escapes characters that are not allowed in the user part of a SIP URI
fn escape_user(user: &str) -> String {
    user.replace('#', "%23")
}

/// Reads the first non-loopback hardware MAC address (`XX-XX-XX-XX-XX-XX`) for the Swyx `User-Agent`.
fn detect_local_mac() -> String {
    if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str == "lo" || name_str.starts_with("docker") || name_str.starts_with("veth") {
                continue;
            }
            if let Ok(raw) = std::fs::read_to_string(entry.path().join("address")) {
                let trimmed = raw.trim();
                if trimmed.len() == 17 && trimmed != "00:00:00:00:00:00" {
                    return trimmed.replace(':', "-").to_ascii_uppercase();
                }
            }
        }
    }
    "02-00-11-22-33-44".to_string()
}

/// Human readable (German) reason for a failed outgoing call
pub fn sip_failure_reason(code: u16, phrase: &str) -> String {
    match code {
        404 | 604 => "Rufnummer nicht gefunden (404)".into(),
        480 => "Teilnehmer nicht erreichbar (480)".into(),
        486 | 600 => "Teilnehmer besetzt (486)".into(),
        487 => "Anruf abgebrochen".into(),
        408 => "Keine Antwort (408)".into(),
        403 => "Anruf vom Server verweigert (403)".into(),
        415 | 488 | 606 => "Kein gemeinsamer Audio-Codec (488)".into(),
        603 => "Anruf abgelehnt (603)".into(),
        401 | 407 => "Authentifizierung am SIP-Server fehlgeschlagen".into(),
        484 => "Rufnummer unvollständig (484)".into(),
        503 => "Telefonanlage nicht verfügbar (503)".into(),
        _ => format!("Rufaufbau fehlgeschlagen: SIP {} {}", code, phrase),
    }
}

/// Builds a response mirroring Via/From/To/Call-ID/CSeq of the request
fn response_for(req: &SipMessage, code: u16, reason: &str) -> SipMessage {
    let mut r = SipMessage::new_response(code, reason);
    for h in ["Via", "From", "To", "Call-ID", "CSeq"] {
        if let Some(v) = req.get_header(h) {
            r.set_header(h, v);
        }
    }
    r
}

/// ACK for a non-2xx final response to an INVITE (RFC 3261 §17.1.1.3):
/// same Request-URI and Via branch as the INVITE, To taken from the response.
fn ack_for_non2xx(invite: &SipMessage, resp: &SipMessage) -> SipMessage {
    let mut ack = SipMessage::new_request("ACK", &request_uri(invite));
    for h in ["Via", "From", "Call-ID", "Route"] {
        if let Some(v) = invite.get_header(h) {
            ack.set_header(h, v);
        }
    }
    ack.set_header("Max-Forwards", "70");
    if let Some(to) = resp.get_header("To").or_else(|| invite.get_header("To")) {
        ack.set_header("To", to);
    }
    let n = invite.cseq().map(|(n, _)| n).unwrap_or(1);
    ack.set_header("CSeq", &format!("{} ACK", n));
    ack
}

/// CANCEL for a pending INVITE (RFC 3261 §9.1)
fn cancel_for(invite: &SipMessage) -> SipMessage {
    let mut cancel = SipMessage::new_request("CANCEL", &request_uri(invite));
    for h in ["Via", "From", "To", "Call-ID", "Route"] {
        if let Some(v) = invite.get_header(h) {
            cancel.set_header(h, v);
        }
    }
    cancel.set_header("Max-Forwards", "70");
    let n = invite.cseq().map(|(n, _)| n).unwrap_or(1);
    cancel.set_header("CSeq", &format!("{} CANCEL", n));
    cancel
}

/// Applies a remote SDP (answer or offer) to an RTP session. Returns the parsed SDP.
fn apply_remote_sdp(rtp: &RtpSession, body: &str) -> Option<SdpSession> {
    let sdp = SdpSession::parse_answer(body).ok()?;
    if let (Some(ip), Some(port)) = (sdp.remote_ip, sdp.remote_port) {
        if !ip.is_unspecified() && port != 0 {
            rtp.set_remote(SocketAddr::new(ip, port), sdp.selected_payload_type);
        }
    }
    if let Some(pt) = sdp.dtmf_payload_type {
        rtp.set_dtmf_payload_type(pt);
    }
    if !sdp.has_supported_codec {
        warn!("Gegenstelle bietet weder G.711 A-law noch µ-law an - Audio wird vermutlich fehlen");
    }
    Some(sdp)
}

/// Determines the registration lifetime granted by the registrar
fn granted_expires(resp: &SipMessage, requested: u32, cfg: &SipConfig) -> u32 {
    let needle = format!("{}:{}", cfg.local_ip, cfg.local_port);
    if let Some(contacts) = resp.get_header("Contact") {
        for c in contacts.split(',') {
            if c.contains(&needle) {
                if let Some(v) = c
                    .split(';')
                    .find_map(|p| p.trim().strip_prefix("expires="))
                    .and_then(|v| v.trim().parse::<u32>().ok())
                {
                    return v;
                }
            }
        }
    }
    resp.get_header("Expires")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(requested)
}

#[derive(Debug, Clone)]
pub struct IncomingDialogInfo {
    pub call_id: String,
    pub from: String,
    pub to: String,
    /// Caller header resolved for UI/display (e.g. via `P-Asserted-Identity` or `;x-ippbx-ext=`)
    pub caller_display: String,
    pub via: String,
    pub cseq: String,
    pub sdp: String,
    pub source_addr: SocketAddr,
    /// Our To-tag (identical in 180 Ringing and the final response)
    pub local_tag: String,
    pub record_route: Option<String>,
    /// Remote target (Contact URI of the INVITE)
    pub contact: Option<String>,
    /// Serialized 180 Ringing (re-sent on INVITE retransmissions)
    pub ringing: Vec<u8>,
}

/// SwyxWare / IpPbxSrv line metadata required for `CLMgr.exe`-compatible call signaling
/// (`To` with `;user=phone;x-sendingcomplete`, `P-Preferred-Identity`, `X-IPPBX*`, `Event: x-ippbx`).
#[derive(Debug, Clone)]
pub struct SwyxLineContext {
    /// Own primary internal extension (e.g. `"100"`), sent in `P-Preferred-Identity`
    pub own_extension: String,
    /// Display name of the logged-in user (e.g. `"Mustermann M. 100"`)
    pub display_name: Option<String>,
    /// Swyx `UserId` (`X-IPPBXOrigUserID`, e.g. `42`)
    pub user_id: i32,
    /// Swyx `SiteId` (`X-IPPBXOrigSiteID`, e.g. `1`, must be > 0)
    pub site_id: i32,
}

#[derive(Debug, Clone)]
pub struct SipConfig {
    pub server_ip: String,
    pub server_port: u16,
    pub username: String,
    pub password: String, // PSK
    pub realm: String,
    pub local_ip: String,
    pub local_port: u16,
}

impl SipConfig {
    /// Returns the SIP URI user part with `{` and `}` percent-encoded as `%7b` and `%7d`,
    /// matching `CLMgr.exe` (`0x9b876c`) for Swyx `SipRegisterId` GUIDs.
    pub fn uri_user(&self) -> String {
        self.username.replace('{', "%7b").replace('}', "%7d")
    }

    /// Returns the SwyxIt! `User-Agent` header expected by `SDeviceInfo::ParseSIPUserAgentString` (`0xaff8b0`).
    ///
    /// **Important (`CtiMaster=No`)**: In `Swyx.Core_14.21` (`0xaffe57..0xaffe9d`), `(CtiMaster=Yes)`
    /// classifies the endpoint as `DeviceType = 14` (Terminal-Server CTI-Master without audio),
    /// causing `IpPbxSrv` to suppress incoming SIP `INVITE`s and ring only a paired desk phone.
    /// `(CtiMaster=No)` classifies the endpoint as `DeviceType = 13` (full SwyxIt! SIP softphone),
    /// enabling parallel incoming call signaling alongside a physical SwyxPhone desk phone.
    pub fn user_agent(&self) -> String {
        format!(
            "SwyxIt!/14.20.0 (Language=Deutsch) (LineMgr=CLMgr.exe/14.21) (OS=Windows 7) (IP={}) (MAC={}) (CtiMaster=No)",
            self.local_ip,
            detect_local_mac()
        )
    }

    /// Our address-of-record (`sip:<user>@<realm>`), identical to the registered binding
    pub fn local_aor(&self) -> String {
        format!("sip:{}@{}", self.uri_user(), self.realm)
    }

    /// Our Contact header (`<sip:<user>@<local_ip>:<local_port>>`)
    pub fn contact_header(&self) -> String {
        format!("<sip:{}@{}:{}>", self.uri_user(), self.local_ip, self.local_port)
    }

    fn via(&self, branch: &str) -> String {
        format!(
            "SIP/2.0/UDP {}:{};branch={};rport",
            self.local_ip, self.local_port, branch
        )
    }
}

#[derive(Debug, Clone)]
pub struct ActiveCall {
    pub call_id: String,
    pub remote_tag: Option<String>,
    pub local_tag: String,
    /// Dialed number (outgoing) or caller number (incoming)
    pub target_number: String,
    pub cseq: u32,
    pub rtp_session: Option<Arc<RtpSession>>,
    pub on_hold: bool,
    /// `From` header for requests we send inside this dialog (incl. our tag)
    pub local_header: String,
    /// `To` header for requests we send inside this dialog (incl. remote tag)
    pub remote_header: String,
    /// Request-URI for in-dialog requests (remote Contact)
    pub remote_target: String,
    pub is_incoming: bool,
}

/// Established dialog state used by the inbound dispatcher (re-INVITE, ACK, BYE)
#[derive(Clone)]
struct DialogState {
    rtp: Option<Arc<RtpSession>>,
    /// Our 2xx to an incoming INVITE (re-sent on INVITE retransmissions)
    ok_response: Option<Vec<u8>>,
    ack_received: Arc<AtomicBool>,
    /// ACK we sent for an outgoing call's 2xx (re-sent on 2xx retransmissions)
    ack_request: Option<Vec<u8>>,
}

/// State of an outgoing INVITE that has not been answered yet
struct OutgoingInvite {
    cancel_requested: bool,
}

pub struct SipClient {
    pub config: SipConfig,
    socket: Arc<UdpSocket>,
    cseq: Arc<AtomicU32>,
    is_registered: Arc<AtomicBool>,
    /// Set after an explicit de-registration (Expires: 0) - stops the registration keeper
    deregistered: Arc<AtomicBool>,
    last_expires: Arc<AtomicU32>,
    register_call_id: String,
    register_tag: String,
    swyx_sub_call_id: String,
    swyx_sub_tag: String,
    swyx_sub_remote_tag: Arc<std::sync::RwLock<Option<String>>>,
    swyx_context: Arc<std::sync::RwLock<Option<SwyxLineContext>>>,
    event_tx: mpsc::Sender<SipEvent>,
    pending_transactions: Arc<Mutex<HashMap<String, mpsc::Sender<SipMessage>>>>,
    active_dialogs: Arc<Mutex<HashMap<String, IncomingDialogInfo>>>,
    established: Arc<Mutex<HashMap<String, DialogState>>>,
    outgoing: Arc<Mutex<HashMap<String, OutgoingInvite>>>,
}

#[derive(Debug)]
pub enum SipEvent {
    IncomingCall {
        call_id: String,
        from: String,
        to: String,
        sdp: String,
    },
    CallRinging {
        call_id: String,
    },
    CallAnswered {
        call_id: String,
        sdp: String,
    },
    CallEnded {
        call_id: String,
        reason: String,
    },
    CallHoldState {
        call_id: String,
        on_hold: bool,
    },
    CallTransferred {
        call_id: String,
        target: String,
    },
    IncomingMessage {
        from: String,
        content: String,
    },
    VoicemailSummary {
        waiting: bool,
        new_messages: u32,
        old_messages: u32,
        account: Option<String>,
    },
    /// SIP registration state changed (e.g. registration lost / restored)
    RegistrationChanged {
        registered: bool,
        message: String,
    },
}

/// Shared state for the inbound request handlers
#[derive(Clone)]
struct DispatchCtx {
    socket: Arc<UdpSocket>,
    config: Arc<SipConfig>,
    swyx_context: Arc<std::sync::RwLock<Option<SwyxLineContext>>>,
    events: mpsc::Sender<SipEvent>,
    dialogs: Arc<Mutex<HashMap<String, IncomingDialogInfo>>>,
    established: Arc<Mutex<HashMap<String, DialogState>>>,
}

fn is_guid_user(user: &str) -> bool {
    let u = user.trim();
    u.starts_with('{') || u.starts_with("%7b") || u.starts_with("%7B")
}

fn extract_uri_param(header: &str, param_name: &str) -> Option<String> {
    let uri_part = header
        .split('<')
        .nth(1)
        .and_then(|s| s.split('>').next())
        .unwrap_or(header);
    let prefix = format!("{}=", param_name.to_ascii_lowercase());
    for part in uri_part.split(';').skip(1) {
        let trimmed = part.trim();
        if trimmed.to_ascii_lowercase().starts_with(&prefix) {
            let val = trimmed[prefix.len()..].trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

fn extract_display_name(header: &str) -> Option<String> {
    let idx = header.find('<')?;
    let name = header[..idx].trim().trim_matches('"').trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Resolves the human-facing caller SIP header from an incoming `INVITE`
/// (checking `P-Asserted-Identity` and Swyx's `;x-ippbx-ext=` URI parameter when `From` carries a GUID).
fn resolve_incoming_caller_header(msg: &SipMessage, raw_from: &str, realm: &str) -> String {
    let from_user = extract_user(raw_from).unwrap_or_default();
    let from_name = extract_display_name(raw_from);

    if let Some(pai) = msg
        .get_header("P-Asserted-Identity")
        .or_else(|| msg.get_header("P-Preferred-Identity"))
    {
        let pai_user = extract_user(pai).unwrap_or_default();
        if !pai_user.is_empty() && !is_guid_user(&pai_user) && (from_user.is_empty() || is_guid_user(&from_user)) {
            let disp = extract_display_name(pai).or(from_name);
            return match disp {
                Some(n) => format!("\"{}\" <sip:{}@{}>", n.replace('"', ""), pai_user, realm),
                None => format!("<sip:{}@{}>", pai_user, realm),
            };
        }
    }

    if from_user.is_empty() || is_guid_user(&from_user) {
        if let Some(ext) = extract_uri_param(raw_from, "x-ippbx-ext") {
            return match from_name {
                Some(n) => format!("\"{}\" <sip:{}@{}>", n.replace('"', ""), ext, realm),
                None => format!("<sip:{}@{}>", ext, realm),
            };
        }
    }

    raw_from.to_string()
}

/// Resolves the human-facing called target SIP header from an incoming `INVITE`
/// (using `X-IPPBXCalledExtension` / `X-IPPBXCalledNumber` / `X-IPPBXCalledName` when `To` is our Swyx device GUID).
fn resolve_incoming_called_header(
    msg: &SipMessage,
    raw_to: &str,
    realm: &str,
    swyx: Option<&SwyxLineContext>,
) -> String {
    let called_ext = msg
        .get_header("X-IPPBXCalledExtension")
        .or_else(|| msg.get_header("X-IPPBXCalledNumber"))
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let called_name = msg
        .get_header("X-IPPBXCalledName")
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if let Some(ext) = called_ext {
        return match called_name {
            Some(n) => format!("\"{}\" <sip:{}@{}>", n.replace('"', ""), ext, realm),
            None => format!("<sip:{}@{}>", ext, realm),
        };
    }

    let to_user = extract_user(raw_to).unwrap_or_default();
    if is_guid_user(&to_user) {
        if let Some(ctx) = swyx {
            if !ctx.own_extension.is_empty() {
                return format!("<sip:{}@{}>", ctx.own_extension, realm);
            }
        }
    }

    raw_to.to_string()
}

/// Builds the 200 OK (incl. SDP answer/offer) for a re-INVITE or UPDATE in an established dialog
fn in_dialog_offer_response(ctx: &DispatchCtx, msg: &SipMessage, dlg: &DialogState) -> SipMessage {
    let mut resp = response_for(msg, 200, "OK");
    resp.set_header("Contact", &ctx.config.contact_header());
    resp.set_header("Allow", ALLOW_METHODS);
    resp.set_header("Supported", SUPPORTED_FEATURES);
    if let Some(rr) = msg.get_header("Record-Route") {
        resp.set_header("Record-Route", rr);
    }
    let local_port = dlg.rtp.as_ref().map(|r| r.local_port).unwrap_or(0);
    let body = String::from_utf8_lossy(&msg.body).to_string();
    if body.contains("m=audio") {
        let offer = match dlg.rtp.as_ref() {
            Some(rtp) => apply_remote_sdp(rtp, &body),
            None => SdpSession::parse_answer(&body).ok(),
        };
        if let Some(offer) = offer {
            let dir = MediaDirection::answer_for(offer.direction);
            resp.body = SdpSession::build_answer(
                &ctx.config.local_ip,
                local_port,
                offer.selected_payload_type,
                offer.dtmf_payload_type,
                dir,
            )
            .into_bytes();
            resp.set_header("Content-Type", "application/sdp");
        }
    } else if msg.method() == Some("INVITE") {
        // re-INVITE ohne SDP: wir machen das Angebot, die Antwort kommt im ACK
        resp.body = SdpSession::build_offer(&ctx.config.local_ip, local_port).into_bytes();
        resp.set_header("Content-Type", "application/sdp");
    }
    resp
}

async fn handle_incoming_invite(ctx: &DispatchCtx, msg: &SipMessage, src: SocketAddr) {
    let call_id = msg.get_header("Call-ID").unwrap_or("").to_string();
    if call_id.is_empty() {
        return;
    }

    // 1. re-INVITE innerhalb eines bestehenden Dialogs (z. B. Medienumleitung, Halten durch Gegenseite)
    if msg.to_tag().is_some() {
        let dlg = ctx.established.lock().await.get(&call_id).cloned();
        let resp = match dlg {
            Some(d) => {
                info!("re-INVITE für Gespräch {} empfangen", call_id);
                in_dialog_offer_response(ctx, msg, &d)
            }
            None => response_for(msg, 481, "Call/Transaction Does Not Exist"),
        };
        let _ = ctx.socket.send_to(&resp.serialize(), src).await;
        return;
    }

    // 2. Retransmission eines noch klingelnden INVITE -> 180 erneut senden, KEIN neues Event
    if let Some(ringing) = ctx.dialogs.lock().await.get(&call_id).map(|d| d.ringing.clone()) {
        debug!("INVITE retransmission for ringing call {}", call_id);
        let _ = ctx.socket.send_to(&ringing, src).await;
        return;
    }

    // 3. Retransmission nach Annahme (unser 200 OK ging verloren)
    if let Some(dlg) = ctx.established.lock().await.get(&call_id).cloned() {
        if let Some(ok) = dlg.ok_response {
            let _ = ctx.socket.send_to(&ok, src).await;
        }
        return;
    }

    // 4. Neuer eingehender Anruf
    let from = msg.get_header("From").unwrap_or("").to_string();
    let to = msg.get_header("To").unwrap_or("").to_string();
    let via = msg.get_header("Via").unwrap_or("").to_string();
    let cseq = msg.get_header("CSeq").unwrap_or("1 INVITE").to_string();
    let sdp = String::from_utf8_lossy(&msg.body).to_string();
    let record_route = msg.get_header("Record-Route").map(|s| s.to_string());
    let contact = msg.contact_uri();

    let swyx_snapshot = ctx.swyx_context.read().ok().and_then(|g| g.clone());
    let event_from = resolve_incoming_caller_header(msg, &from, &ctx.config.realm);
    let event_to = resolve_incoming_called_header(msg, &to, &ctx.config.realm, swyx_snapshot.as_ref());

    info!(
        "Incoming SIP INVITE for call {} from {} (resolved caller: {}, target: {})",
        call_id, from, event_from, event_to
    );

    let local_tag = new_tag();
    let mut ringing = response_for(msg, 180, "Ringing");
    ringing.set_header("To", &add_tag(&to, &local_tag));
    ringing.set_header("Contact", &ctx.config.contact_header());
    ringing.set_header("Allow", ALLOW_METHODS);
    ringing.set_header("User-Agent", &ctx.config.user_agent());
    if let Some(ref rr) = record_route {
        ringing.set_header("Record-Route", rr);
    }
    let ringing_raw = ringing.serialize();
    let _ = ctx.socket.send_to(&ringing_raw, src).await;

    ctx.dialogs.lock().await.insert(
        call_id.clone(),
        IncomingDialogInfo {
            call_id: call_id.clone(),
            from,
            to,
            caller_display: event_from.clone(),
            via,
            cseq,
            sdp: sdp.clone(),
            source_addr: src,
            local_tag,
            record_route,
            contact,
            ringing: ringing_raw,
        },
    );

    let _ = ctx
        .events
        .send(SipEvent::IncomingCall {
            call_id,
            from: event_from,
            to: event_to,
            sdp,
        })
        .await;
}

async fn handle_incoming_ack(ctx: &DispatchCtx, msg: &SipMessage) {
    let call_id = msg.get_header("Call-ID").unwrap_or("").to_string();
    let dlg = ctx.established.lock().await.get(&call_id).cloned();
    if let Some(d) = dlg {
        d.ack_received.store(true, Ordering::SeqCst);
        // Delayed-Offer: SDP-Antwort kommt im ACK
        let body = String::from_utf8_lossy(&msg.body).to_string();
        if body.contains("m=audio") {
            if let Some(ref rtp) = d.rtp {
                apply_remote_sdp(rtp, &body);
            }
        }
        debug!("ACK received for call {}", call_id);
    }
}

async fn handle_incoming_bye(ctx: &DispatchCtx, msg: &SipMessage, src: SocketAddr) {
    let call_id = msg.get_header("Call-ID").unwrap_or("").to_string();
    info!("Incoming SIP BYE for call {}", call_id);

    let ok = response_for(msg, 200, "OK");
    let _ = ctx.socket.send_to(&ok.serialize(), src).await;

    if let Some(d) = ctx.established.lock().await.remove(&call_id) {
        d.ack_received.store(true, Ordering::SeqCst);
        if let Some(rtp) = d.rtp {
            rtp.stop();
        }
    }
    ctx.dialogs.lock().await.remove(&call_id);

    let _ = ctx
        .events
        .send(SipEvent::CallEnded {
            call_id,
            reason: "Remote party hung up".to_string(),
        })
        .await;
}

async fn handle_incoming_cancel(ctx: &DispatchCtx, msg: &SipMessage, src: SocketAddr) {
    let call_id = msg.get_header("Call-ID").unwrap_or("").to_string();
    info!("Incoming SIP CANCEL for call {}", call_id);

    let ok = response_for(msg, 200, "OK");
    let _ = ctx.socket.send_to(&ok.serialize(), src).await;

    let Some(info) = ctx.dialogs.lock().await.remove(&call_id) else {
        return;
    };

    // 487 Request Terminated für die ursprüngliche INVITE-Transaktion
    let mut term = SipMessage::new_response(487, "Request Terminated");
    term.set_header("Via", &info.via);
    term.set_header("From", &info.from);
    term.set_header("To", &add_tag(&info.to, &info.local_tag));
    term.set_header("Call-ID", &call_id);
    term.set_header("CSeq", &info.cseq);
    let _ = ctx.socket.send_to(&term.serialize(), info.source_addr).await;

    let reason_hdr = msg.get_header("Reason").unwrap_or("").to_lowercase();
    let elsewhere = reason_hdr.contains("cause=200") || reason_hdr.contains("elsewhere");
    let _ = ctx
        .events
        .send(SipEvent::CallEnded {
            call_id,
            reason: if elsewhere {
                "Answered elsewhere".to_string()
            } else {
                "Caller cancelled call".to_string()
            },
        })
        .await;
}

async fn handle_incoming_update(ctx: &DispatchCtx, msg: &SipMessage, src: SocketAddr) {
    let call_id = msg.get_header("Call-ID").unwrap_or("").to_string();
    let dlg = ctx.established.lock().await.get(&call_id).cloned();
    let resp = match dlg {
        Some(d) => in_dialog_offer_response(ctx, msg, &d),
        None => response_for(msg, 200, "OK"),
    };
    let _ = ctx.socket.send_to(&resp.serialize(), src).await;
}

impl SipClient {
    pub async fn new(config: SipConfig) -> Result<(Self, mpsc::Receiver<SipEvent>)> {
        let bind_addr = format!("{}:{}", config.local_ip, config.local_port);
        let socket = UdpSocket::bind(&bind_addr).await?;
        info!("SIP socket bound to {}", bind_addr);

        let (event_tx, event_rx) = mpsc::channel(64);
        let pending_transactions: Arc<Mutex<HashMap<String, mpsc::Sender<SipMessage>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let active_dialogs: Arc<Mutex<HashMap<String, IncomingDialogInfo>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let established: Arc<Mutex<HashMap<String, DialogState>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let socket = Arc::new(socket);
        let cseq = Arc::new(AtomicU32::new(1));
        let server_addr: SocketAddr = format!("{}:{}", config.server_ip, config.server_port)
            .parse()
            .map_err(|e| anyhow!("Ungültige SIP-Serveradresse {}:{} ({})", config.server_ip, config.server_port, e))?;

        let swyx_context: Arc<std::sync::RwLock<Option<SwyxLineContext>>> =
            Arc::new(std::sync::RwLock::new(None));

        let ctx = DispatchCtx {
            socket: socket.clone(),
            config: Arc::new(config.clone()),
            swyx_context: swyx_context.clone(),
            events: event_tx.clone(),
            dialogs: active_dialogs.clone(),
            established: established.clone(),
        };

        // Spawn central SIP Inbound Dispatcher and Transaction Router
        let rx_socket = socket.clone();
        let rx_pending = pending_transactions.clone();
        let rx_events = event_tx.clone();
        let rx_config = ctx.config.clone();
        let rx_cseq = cseq.clone();
        let rx_server_addr = server_addr;
        let rx_established = established.clone();

        tokio::spawn(async move {
            let mut buf = vec![0u8; 65535];
            loop {
                let (len, src) = match rx_socket.recv_from(&mut buf).await {
                    Ok(res) => res,
                    Err(e) => {
                        // z. B. ICMP "port unreachable" - der Dispatcher darf deshalb NICHT sterben
                        debug!("SIP socket recv error: {}", e);
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        continue;
                    }
                };

                // CRLF keep-alive pings ignorieren
                if buf[..len].iter().all(|b| *b == b'\r' || *b == b'\n') {
                    continue;
                }

                let msg = match SipMessage::parse(&buf[..len]) {
                    Ok(m) => m,
                    Err(e) => {
                        debug!("Failed to parse incoming SIP message from {}: {}", src, e);
                        continue;
                    }
                };
                if msg.method() != Some("OPTIONS") {
                    debug!("Received SIP from {}:\n{}", src, String::from_utf8_lossy(&buf[..len]));
                }

                // Case 1: Incoming SIP Request
                if let Some(method) = msg.method().map(|m| m.to_string()) {
                    match method.as_str() {
                        "INVITE" => handle_incoming_invite(&ctx, &msg, src).await,
                        "ACK" => handle_incoming_ack(&ctx, &msg).await,
                        "BYE" => handle_incoming_bye(&ctx, &msg, src).await,
                        "CANCEL" => handle_incoming_cancel(&ctx, &msg, src).await,
                        "UPDATE" => handle_incoming_update(&ctx, &msg, src).await,
                        "INFO" | "PRACK" => {
                            let ok = response_for(&msg, 200, "OK");
                            let _ = rx_socket.send_to(&ok.serialize(), src).await;
                        }
                        "OPTIONS" => {
                            debug!("Replying 200 OK to OPTIONS ping from {}", src);
                            let mut ok = response_for(&msg, 200, "OK");
                            ok.set_header("Allow", ALLOW_METHODS);
                            ok.set_header("Accept", "application/sdp");
                            ok.set_header("Supported", SUPPORTED_FEATURES);
                            let _ = rx_socket.send_to(&ok.serialize(), src).await;
                        }
                        "MESSAGE" => {
                            let call_id = msg.get_header("Call-ID").unwrap_or("").to_string();
                            let from = msg.get_header("From").unwrap_or("").to_string();
                            let to = msg.get_header("To").unwrap_or("").to_string();
                            let via = msg.get_header("Via").unwrap_or("").to_string();
                            let cseq = msg.get_header("CSeq").unwrap_or("1 MESSAGE").to_string();
                            let body_text = String::from_utf8_lossy(&msg.body).to_string();

                            info!("Incoming SIP MESSAGE from {}: {}", from, body_text);

                            // Reply 200 OK immediately
                            let mut ok = SipMessage::new_response(200, "OK");
                            if !via.is_empty() {
                                ok.set_header("Via", &via);
                            }
                            if !from.is_empty() {
                                ok.set_header("From", &from);
                            }
                            if !to.is_empty() {
                                ok.set_header("To", &to);
                            }
                            ok.set_header("Call-ID", &call_id);
                            ok.set_header("CSeq", &cseq);
                            let _ = rx_socket.send_to(&ok.serialize(), src).await;

                            let content_type = msg
                                .get_header("Content-Type")
                                .or_else(|| msg.get_header("c"))
                                .unwrap_or("")
                                .to_string();

                            if super::ippbx_im::is_ippbx_im_content_type(&content_type) {
                                use super::ippbx_im::IpPbxIm;
                                let Some(im) = IpPbxIm::parse(&body_text) else {
                                    debug!("Unparsable IpPbxIM payload from {}", from);
                                    continue;
                                };

                                // Anfragen (Hallo/Msg/Bye/State) wie SwyxIt! mit ResponseOK quittieren
                                if im.dir == super::ippbx_im::DIR_REQUEST {
                                    let peer_uri = from
                                        .split('<')
                                        .nth(1)
                                        .and_then(|s| s.split('>').next())
                                        .map(|s| s.trim().to_string())
                                        .filter(|s| !s.is_empty())
                                        .or_else(|| {
                                            let part = from.split(';').next().unwrap_or("").trim();
                                            if part.starts_with("sip:") {
                                                Some(part.to_string())
                                            } else {
                                                None
                                            }
                                        })
                                        .unwrap_or_else(|| {
                                            format!("sip:{}@{}", rx_config.username, rx_config.realm)
                                        });
                                    if !peer_uri.is_empty() {
                                        let ack_body = im.response_ok().to_xml();
                                        let mut ack_headers = Vec::new();
                                        ack_headers.push(("X-IPPBXDestIP".into(), "0.0.0.0:0".into()));
                                        ack_headers.push(("X-IPPBXOrigIP".into(), "0.0.0.0:0".into()));

                                        // User-IDs und Site-IDs spiegeln (Orig <-> Dest).
                                        // CLMgr.exe (0x629219 / 0x62adba) verwirft jedes Paket mit SiteID == 0!
                                        if let Some(v) = msg
                                            .get_header("X-IPPBXDestUserID")
                                            .or_else(|| msg.get_header("X-IPPBXCTIDestUserID"))
                                        {
                                            ack_headers.push(("X-IPPBXOrigUserID".into(), v.to_string()));
                                        }
                                        if let Some(v) = msg
                                            .get_header("X-IPPBXOrigUserID")
                                            .or_else(|| msg.get_header("X-IPPBXCTIOrigUserID"))
                                        {
                                            ack_headers.push(("X-IPPBXDestUserID".into(), v.to_string()));
                                        }

                                        let orig_site_for_ack = msg
                                            .get_header("X-IPPBXDestSiteID")
                                            .or_else(|| msg.get_header("X-IPPBXCTIDestSiteID"))
                                            .map(|s| s.trim())
                                            .filter(|s| !s.is_empty() && *s != "0")
                                            .unwrap_or("1");
                                        let dest_site_for_ack = msg
                                            .get_header("X-IPPBXOrigSiteID")
                                            .or_else(|| msg.get_header("X-IPPBXCTIOrigSiteID"))
                                            .map(|s| s.trim())
                                            .filter(|s| !s.is_empty() && *s != "0")
                                            .unwrap_or("1");
                                        ack_headers.push(("X-IPPBXOrigSiteID".into(), orig_site_for_ack.to_string()));
                                        ack_headers.push(("X-IPPBXDestSiteID".into(), dest_site_for_ack.to_string()));

                                        if let Some(v) = msg.get_header("X-IPPBXDestSiteGUID") {
                                            ack_headers.push(("X-IPPBXOrigSiteGUID".into(), v.to_string()));
                                        }
                                        if let Some(v) = msg.get_header("X-IPPBXOrigSiteGUID") {
                                            ack_headers.push(("X-IPPBXDestSiteGUID".into(), v.to_string()));
                                        }
                                        if let Some(v) = msg.get_header("X-IPPBXDestSessionID") {
                                            ack_headers.push(("X-IPPBXOrigSessionID".into(), v.to_string()));
                                        }
                                        if let Some(v) = msg.get_header("X-IPPBXOrigSessionID") {
                                            ack_headers.push(("X-IPPBXDestSessionID".into(), v.to_string()));
                                        }

                                        let sock_ack = rx_socket.clone();
                                        let srv_ack = rx_server_addr;
                                        let cfg_ack = rx_config.clone();
                                        let cseq_ack = rx_cseq.clone();
                                        let pending_ack = rx_pending.clone();
                                        let peer_uri_log = peer_uri.clone();
                                        tokio::spawn(async move {
                                            if let Err(e) = do_send_message_with_auth(
                                                &sock_ack,
                                                srv_ack,
                                                &cfg_ack,
                                                &cseq_ack,
                                                &pending_ack,
                                                &peer_uri,
                                                super::ippbx_im::CONTENT_TYPE,
                                                ack_body.into_bytes(),
                                                &ack_headers,
                                            ).await {
                                                debug!("Failed to send authenticated IpPbxIM ResponseOK to {}: {}", peer_uri_log, e);
                                            } else {
                                                debug!("Sent authenticated IpPbxIM ResponseOK to {}", peer_uri_log);
                                            }
                                        });
                                    }
                                }

                                if !im.is_incoming_text() {
                                    debug!(
                                        "IpPbxIM control message from {} (type={}, dir={}, id={})",
                                        from, im.msg_type, im.dir, im.id
                                    );
                                    continue;
                                }

                                let effective_from = if let Some(orig_uid) = msg
                                    .get_header("X-IPPBXOrigUserID")
                                    .or_else(|| msg.get_header("X-IPPBXCTIOrigUserID"))
                                    .map(|s| s.trim())
                                    .filter(|s| !s.is_empty() && *s != "0")
                                {
                                    let has_display_name = from
                                        .find('<')
                                        .map(|idx| !from[..idx].trim().trim_matches('"').trim().is_empty())
                                        .unwrap_or(false);
                                    let uri_with_user = if !from.contains('@') && from.contains("sip:") {
                                        from.replacen("sip:", &format!("sip:swyx-{}@", orig_uid), 1)
                                    } else if let (Some(sip_idx), Some(at_idx)) = (from.find("sip:"), from.find('@')) {
                                        let user_part = &from[sip_idx + 4..at_idx];
                                        if user_part.is_empty()
                                            || user_part.starts_with("%7b")
                                            || user_part.starts_with("%7B")
                                            || user_part.starts_with('{')
                                        {
                                            format!(
                                                "{}sip:swyx-{}{}",
                                                &from[..sip_idx],
                                                orig_uid,
                                                &from[at_idx..]
                                            )
                                        } else {
                                            from.clone()
                                        }
                                    } else {
                                        from.clone()
                                    };
                                    if !has_display_name {
                                        if uri_with_user.contains('<') {
                                            format!("\"swyx-{}\" {}", orig_uid, uri_with_user)
                                        } else {
                                            format!("\"swyx-{}\" <{}>", orig_uid, uri_with_user)
                                        }
                                    } else {
                                        uri_with_user
                                    }
                                } else {
                                    from
                                };

                                let _ = rx_events
                                    .send(SipEvent::IncomingMessage {
                                        from: effective_from,
                                        content: im.text.unwrap_or_default(),
                                    })
                                    .await;
                                continue;
                            }

                            if is_non_chat_sip_message(&content_type, &body_text) {
                                debug!(
                                    "Ignoring non-chat SIP control MESSAGE from {} (content-type='{}')",
                                    from, content_type
                                );
                                continue;
                            }

                            let _ = rx_events
                                .send(SipEvent::IncomingMessage {
                                    from,
                                    content: body_text,
                                })
                                .await;
                        }
                        "NOTIFY" => {
                            let mut ok = SipMessage::new_response(200, "OK");
                            if let Some(via) = msg.get_header("Via") {
                                ok.set_header("Via", via);
                            }
                            if let Some(from) = msg.get_header("From") {
                                ok.set_header("From", from);
                            }
                            if let Some(to) = msg.get_header("To") {
                                ok.set_header("To", to);
                            }
                            if let Some(cid) = msg.get_header("Call-ID") {
                                ok.set_header("Call-ID", cid);
                            }
                            if let Some(cseq) = msg.get_header("CSeq") {
                                ok.set_header("CSeq", cseq);
                            }
                            ok.set_header(
                                "Contact",
                                &format!("<sip:{}:{}>", rx_config.local_ip, rx_config.local_port),
                            );
                            ok.set_header("Allow", ALLOW_METHODS);
                            ok.set_header("User-Agent", &rx_config.user_agent());
                            if let Some(ev) = msg.get_header("Event") {
                                ok.set_header("Event", ev);
                            }
                            let _ = rx_socket.send_to(&ok.serialize(), src).await;

                            let body_str = String::from_utf8_lossy(&msg.body);
                            let event_hdr = msg.get_header("Event").unwrap_or("");
                            let ctype = msg.get_header("Content-Type").unwrap_or("");

                            if event_hdr.contains("message-summary")
                                || ctype.contains("simple-message-summary")
                                || body_str.contains("Messages-Waiting:")
                            {
                                let (waiting, new_messages, old_messages, account) =
                                    parse_mwi_summary(&body_str);
                                info!(
                                    "Received SIP MWI Voicemail NOTIFY: waiting={}, new={}, old={}",
                                    waiting, new_messages, old_messages
                                );
                                let _ = rx_events
                                    .send(SipEvent::VoicemailSummary {
                                        waiting,
                                        new_messages,
                                        old_messages,
                                        account,
                                    })
                                    .await;
                            }
                        }
                        _ => {
                            debug!("Unhandled SIP request: {}", method);
                            let resp = response_for(&msg, 501, "Not Implemented");
                            let _ = rx_socket.send_to(&resp.serialize(), src).await;
                        }
                    }
                } else if let Some(code) = msg.status_code() {
                    // Case 2: Incoming SIP Response -> route to waiting transaction channel
                    let Some(cid) = msg.get_header("Call-ID").map(|s| s.to_string()) else {
                        continue;
                    };
                    let routed = {
                        let pending = rx_pending.lock().await;
                        match pending.get(&cid) {
                            Some(tx) => {
                                let _ = tx.try_send(msg.clone());
                                true
                            }
                            None => false,
                        }
                    };
                    if !routed {
                        let is_invite = msg.cseq().map(|(_, m)| m == "INVITE").unwrap_or(false);
                        if is_invite && (200..300).contains(&code) {
                            // 2xx-Retransmission: unser ACK ging verloren -> erneut senden
                            let ack = rx_established
                                .lock()
                                .await
                                .get(&cid)
                                .and_then(|d| d.ack_request.clone());
                            if let Some(ack) = ack {
                                debug!("Re-sending ACK for retransmitted 2xx ({})", cid);
                                let _ = rx_socket.send_to(&ack, rx_server_addr).await;
                            }
                        } else {
                            debug!("Unsolicited or expired SIP response for Call-ID: {}", cid);
                        }
                    }
                }
            }
        });

        let client = Self {
            config,
            socket,
            cseq,
            is_registered: Arc::new(AtomicBool::new(false)),
            deregistered: Arc::new(AtomicBool::new(false)),
            last_expires: Arc::new(AtomicU32::new(0)),
            register_call_id: format!("reg-{}", Uuid::new_v4().simple()),
            register_tag: new_tag(),
            swyx_sub_call_id: format!("swyxsub-{}", Uuid::new_v4().simple()),
            swyx_sub_tag: new_tag(),
            swyx_sub_remote_tag: Arc::new(std::sync::RwLock::new(None)),
            swyx_context,
            event_tx,
            pending_transactions,
            active_dialogs,
            established,
            outgoing: Arc::new(Mutex::new(HashMap::new())),
        };

        Ok((client, event_rx))
    }

    /// Configures SwyxWare line metadata (`own_extension`, `display_name`, `user_id`, `site_id`)
    /// so outgoing/incoming `INVITE`s and `REGISTER` follow `CLMgr.exe` signaling.
    pub fn set_swyx_context(&self, ctx: SwyxLineContext) {
        if let Ok(mut guard) = self.swyx_context.write() {
            *guard = Some(ctx);
        }
    }

    /// Returns the currently configured SwyxWare line metadata, if any.
    pub fn swyx_context(&self) -> Option<SwyxLineContext> {
        self.swyx_context.read().ok().and_then(|g| g.clone())
    }

    fn next_cseq(&self) -> u32 {
        self.cseq.fetch_add(1, Ordering::SeqCst)
    }

    fn server_addr(&self) -> SocketAddr {
        format!("{}:{}", self.config.server_ip, self.config.server_port)
            .parse()
            .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], self.config.server_port)))
    }

    /// Returns true if the last SIP REGISTER succeeded
    pub fn is_registered(&self) -> bool {
        self.is_registered.load(Ordering::SeqCst)
    }

    /// Returns true if an incoming call with this Call-ID is currently ringing
    pub async fn has_pending_incoming(&self, call_id: &str) -> bool {
        self.active_dialogs.lock().await.contains_key(call_id)
    }

    /// Registers a response channel for a pending transaction (matched by Call-ID)
    pub async fn register_transaction(&self, call_id: &str) -> mpsc::Receiver<SipMessage> {
        let (tx, rx) = mpsc::channel(32);
        let mut pending = self.pending_transactions.lock().await;
        pending.insert(call_id.to_string(), tx);
        rx
    }

    /// Unregisters a response channel for a completed transaction
    pub async fn unregister_transaction(&self, call_id: &str) {
        let mut pending = self.pending_transactions.lock().await;
        pending.remove(call_id);
    }

    /// Sends a SIP message to the server
    pub async fn send_message(&self, msg: &SipMessage) -> Result<()> {
        let raw = msg.serialize();
        debug!(
            "Sending SIP message to {}:\n{}",
            self.server_addr(),
            String::from_utf8_lossy(&raw)
        );
        self.socket.send_to(&raw, self.server_addr()).await?;
        Ok(())
    }

    /// Sends a request and waits for its final response (retransmitting over UDP until the
    /// first response arrives). Responses of other transactions on the same Call-ID are skipped.
    async fn send_and_wait_final(
        &self,
        req: &SipMessage,
        rx: &mut mpsc::Receiver<SipMessage>,
        method: &str,
        cseq: u32,
        total: Duration,
    ) -> Result<SipMessage> {
        let raw = req.serialize();
        debug!("Sending SIP {}:\n{}", method, String::from_utf8_lossy(&raw));
        self.socket.send_to(&raw, self.server_addr()).await?;

        let deadline = Instant::now() + total;
        let mut interval = T1;
        let mut next_retx = Instant::now() + interval;
        let mut got_provisional = false;

        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(anyhow!("Timeout: keine Antwort vom SIP-Server auf {}", method));
            }
            let wake = if got_provisional { deadline } else { next_retx.min(deadline) };
            match tokio::time::timeout_at(wake, rx.recv()).await {
                Ok(Some(m)) => {
                    if !matches_cseq(&m, cseq, method) {
                        continue;
                    }
                    if m.status_code().unwrap_or(0) >= 200 {
                        return Ok(m);
                    }
                    got_provisional = true;
                }
                Ok(None) => return Err(anyhow!("SIP-Transaktionskanal geschlossen")),
                Err(_) => {
                    if !got_provisional && Instant::now() < deadline {
                        self.socket.send_to(&raw, self.server_addr()).await?;
                        interval = (interval * 2).min(T2);
                        next_retx = Instant::now() + interval;
                    }
                }
            }
        }
    }

    /// Registers this client with PBX via SIP Digest Authentication
    pub async fn register(&self, expires: u32) -> Result<()> {
        self.register_with_expiry(expires).await.map(|_| ())
    }

    /// Registers this client and returns the lifetime (seconds) granted by the registrar.
    /// Emits [`SipEvent::RegistrationChanged`] whenever the registration state flips.
    pub async fn register_with_expiry(&self, expires: u32) -> Result<u32> {
        self.deregistered.store(expires == 0, Ordering::SeqCst);
        let res = self.do_register(expires).await;
        let now_registered = res.is_ok() && expires > 0;
        if let Ok(granted) = res {
            self.last_expires.store(granted, Ordering::SeqCst);
        }
        let was = self.is_registered.swap(now_registered, Ordering::SeqCst);
        if was != now_registered {
            let message = match &res {
                Ok(_) if now_registered => "SIP-Registrierung erfolgreich".to_string(),
                Ok(_) => "SIP-Registrierung aufgehoben".to_string(),
                Err(e) => format!("SIP-Registrierung fehlgeschlagen: {}", e),
            };
            let _ = self.event_tx.try_send(SipEvent::RegistrationChanged {
                registered: now_registered,
                message,
            });
        }
        res
    }

    async fn do_register(&self, mut expires: u32) -> Result<u32> {
        info!(
            "Registering SIP client {} to realm {} (Expires {})",
            self.config.username, self.config.realm, expires
        );

        let call_id = self.register_call_id.clone();
        let aor = self.config.local_aor();
        let to = format!("<{}>", aor);
        let from = format!("<{}>;tag={}", aor, self.register_tag);
        // CLMgr.exe sends `REGISTER sip:<server_ip>:<server_port> SIP/2.0` while using `<SipRealm>`
        // (e.g. `SWYXPBX01`) in `To` and `From`. Generic SIP registrars expect `sip:<realm>`.
        let uri = if self.swyx_context().is_some() {
            format!("sip:{}:{}", self.config.server_ip, self.config.server_port)
        } else {
            format!("sip:{}", self.config.realm)
        };
        let user_agent = self.config.user_agent();

        let mut rx = self.register_transaction(&call_id).await;
        let mut auth: Option<(String, String)> = None;

        let result: Result<u32> = async {
            for attempt in 0..4 {
                let cseq = self.next_cseq();
                let mut reg = SipMessage::new_request("REGISTER", &uri);
                reg.set_header("Via", &self.config.via(&new_branch()));
                reg.set_header("Max-Forwards", "70");
                reg.set_header("From", &from);
                reg.set_header("To", &to);
                reg.set_header("Call-ID", &call_id);
                reg.set_header("CSeq", &format!("{} REGISTER", cseq));
                reg.set_header("Contact", &self.config.contact_header());
                reg.set_header("Expires", &expires.to_string());
                reg.set_header("Allow", ALLOW_METHODS);
                reg.set_header("Supported", SUPPORTED_FEATURES);
                reg.set_header("User-Agent", &user_agent);
                if let Some((ref n, ref v)) = auth {
                    reg.set_header(n, v);
                }

                let resp = self
                    .send_and_wait_final(&reg, &mut rx, "REGISTER", cseq, Duration::from_secs(8))
                    .await?;
                let code = resp.status_code().unwrap_or(0);
                match code {
                    200..=299 => {
                        let granted = granted_expires(&resp, expires, &self.config);
                        info!(
                            "SIP Registration successful for {} (granted {}s)",
                            self.config.username, granted
                        );
                        return Ok(granted);
                    }
                    401 | 407 if attempt < 2 => {
                        let ch = resp
                            .get_header("WWW-Authenticate")
                            .or_else(|| resp.get_header("Proxy-Authenticate"))
                            .ok_or_else(|| anyhow!("Missing WWW-Authenticate header in challenge"))?;
                        auth = Some(
                            DigestAuth::authorize(
                                ch,
                                code == 407,
                                &self.config.username,
                                &self.config.password,
                                "REGISTER",
                                &uri,
                            )
                            .ok_or_else(|| anyhow!("Failed to parse challenge header: {}", ch))?,
                        );
                    }
                    423 => {
                        // Interval Too Brief -> Min-Expires übernehmen
                        let min = resp
                            .get_header("Min-Expires")
                            .and_then(|v| v.trim().parse::<u32>().ok())
                            .ok_or_else(|| anyhow!("SIP 423 ohne Min-Expires"))?;
                        expires = min;
                    }
                    _ => {
                        return Err(anyhow!(
                            "SIP registration rejected with status {} {}",
                            code,
                            reason_phrase(&resp)
                        ))
                    }
                }
            }
            Err(anyhow!("SIP registration failed: authentication rejected"))
        }
        .await;

        self.unregister_transaction(&call_id).await;
        result
    }

    /// Subscribes to SwyxWare's proprietary device event package (`Event: x-ippbx`),
    /// matching `CLMgr.exe` (`SwSIPActSubscr::ActOnStartSubscr`) right after `REGISTER 200 OK`.
    pub async fn subscribe_swyx_events(&self, expires: u32) -> Result<()> {
        let call_id = self.swyx_sub_call_id.clone();
        let uri_user = self.config.uri_user();
        let req_uri = format!("sip:{}@{}", uri_user, self.config.server_ip);
        let aor = self.config.local_aor();
        let from = format!("<{}>;tag={}", aor, self.swyx_sub_tag);
        let existing_to_tag = self.swyx_sub_remote_tag.read().ok().and_then(|g| g.clone());
        let to = match existing_to_tag {
            Some(ref t) => format!("<{}>;tag={}", aor, t),
            None => format!("<{}>", aor),
        };
        let user_agent = self.config.user_agent();

        let mut rx = self.register_transaction(&call_id).await;
        let mut auth: Option<(String, String)> = None;

        let res: Result<()> = async {
            for attempt in 0..3 {
                let cseq = self.next_cseq();
                let mut sub = SipMessage::new_request("SUBSCRIBE", &req_uri);
                sub.set_header("Via", &self.config.via(&new_branch()));
                sub.set_header("Max-Forwards", "70");
                sub.set_header("Contact", &self.config.contact_header());
                sub.set_header("To", &to);
                sub.set_header("From", &from);
                sub.set_header("Call-ID", &call_id);
                sub.set_header("CSeq", &format!("{} SUBSCRIBE", cseq));
                sub.set_header("Expires", &expires.to_string());
                sub.set_header("Allow", ALLOW_METHODS);
                sub.set_header("User-Agent", &user_agent);
                sub.set_header("Event", "x-ippbx");
                if let Some((ref n, ref v)) = auth {
                    sub.set_header(n, v);
                }

                let resp = self
                    .send_and_wait_final(&sub, &mut rx, "SUBSCRIBE", cseq, Duration::from_secs(5))
                    .await?;
                let code = resp.status_code().unwrap_or(0);
                match code {
                    200..=299 => {
                        if expires == 0 {
                            if let Ok(mut g) = self.swyx_sub_remote_tag.write() {
                                *g = None;
                            }
                        } else if let Some(t) = resp.to_tag() {
                            if let Ok(mut g) = self.swyx_sub_remote_tag.write() {
                                *g = Some(t);
                            }
                        }
                        info!("Swyx Event: x-ippbx subscription active (Expires {})", expires);
                        return Ok(());
                    }
                    401 | 407 if attempt < 2 => {
                        let ch = resp
                            .get_header("WWW-Authenticate")
                            .or_else(|| resp.get_header("Proxy-Authenticate"))
                            .ok_or_else(|| anyhow!("Missing authenticate header in SUBSCRIBE challenge"))?;
                        auth = Some(
                            DigestAuth::authorize(
                                ch,
                                code == 407,
                                &self.config.username,
                                &self.config.password,
                                "SUBSCRIBE",
                                &req_uri,
                            )
                            .ok_or_else(|| anyhow!("Failed to parse challenge in SUBSCRIBE"))?,
                        );
                    }
                    _ => {
                        return Err(anyhow!(
                            "SUBSCRIBE x-ippbx rejected with {} {}",
                            code,
                            reason_phrase(&resp)
                        ));
                    }
                }
            }
            Err(anyhow!("SUBSCRIBE x-ippbx authentication failed"))
        }
        .await;

        self.unregister_transaction(&call_id).await;
        res
    }

    /// Spawns a background task that keeps the SIP registration alive (refresh at half the
    /// granted lifetime, fast retry after failures). Stops automatically when the client is dropped.
    pub fn start_registration_keeper(self: &Arc<Self>, requested_expires: u32) -> tokio::task::JoinHandle<()> {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut last_sub_refresh = Instant::now();
            loop {
                let was_registered = {
                    let Some(client) = weak.upgrade() else { break };
                    if client.deregistered.load(Ordering::SeqCst) {
                        break;
                    }
                    client.is_registered()
                };
                let wait = {
                    let Some(client) = weak.upgrade() else { break };
                    if was_registered {
                        let granted = client.last_expires.load(Ordering::SeqCst).max(40);
                        Duration::from_secs(((granted / 2) as u64).clamp(20, 600))
                    } else {
                        Duration::from_secs(15)
                    }
                };
                tokio::time::sleep(wait).await;
                let Some(client) = weak.upgrade() else { break };
                if client.deregistered.load(Ordering::SeqCst) {
                    break;
                }
                match client.register_with_expiry(requested_expires).await {
                    Ok(_) => {
                        if client.swyx_context().is_some()
                            && (!was_registered || last_sub_refresh.elapsed() >= Duration::from_secs(1500))
                        {
                            if client.subscribe_swyx_events(requested_expires).await.is_ok() {
                                last_sub_refresh = Instant::now();
                            }
                        }
                    }
                    Err(e) => {
                        warn!("SIP re-registration failed: {}", e);
                    }
                }
            }
        })
    }

    /// Dials an outbound phone number or extension, establishing a call with RTP audio.
    /// Returns once the call is answered (or fails / is cancelled).
    pub async fn dial(&self, target_number: &str) -> Result<ActiveCall> {
        let call_id = generate_call_id();
        self.dial_with_call_id(&call_id, target_number).await
    }

    /// Like [`SipClient::dial`] but with a caller-provided Call-ID, so that the call can be
    /// cancelled via [`SipClient::cancel_outgoing`] while it is still ringing.
    pub async fn dial_with_call_id(&self, call_id: &str, target_number: &str) -> Result<ActiveCall> {
        let target_clean: String = target_number
            .chars()
            .filter(|c| !c.is_whitespace() && !matches!(c, '/' | '-' | '(' | ')'))
            .collect();
        if target_clean.is_empty() {
            return Err(anyhow!("Keine Rufnummer angegeben"));
        }
        let swyx_ctx = self.swyx_context();
        let (request_uri, to_header) = if target_clean.starts_with("sip:") {
            (target_clean.clone(), format!("<{}>", target_clean))
        } else if swyx_ctx.is_some() {
            // CLMgr.exe (`SwSIPCall::PrepareInviteInClient`):
            // - Request-URI targets `<server_ip>:<server_port>`
            // - `To` uses `<SipRealm>` plus `;user=phone;x-sendingcomplete` (en-bloc dialing)
            let escaped = escape_user(&target_clean);
            (
                format!(
                    "sip:{}@{}:{}",
                    escaped, self.config.server_ip, self.config.server_port
                ),
                format!(
                    "<sip:{}@{};user=phone;x-sendingcomplete>",
                    escaped, self.config.realm
                ),
            )
        } else {
            let uri = format!("sip:{}@{}", escape_user(&target_clean), self.config.realm);
            let hdr = format!("<{}>", uri);
            (uri, hdr)
        };
        info!(
            "Dialing {} via SIP (To: {}, Call-ID: {})",
            request_uri, to_header, call_id
        );

        let rtp = Arc::new(RtpSession::bind().await?);
        let mut rx = self.register_transaction(call_id).await;
        self.outgoing
            .lock()
            .await
            .insert(call_id.to_string(), OutgoingInvite { cancel_requested: false });

        let result = self
            .run_outgoing_invite(
                call_id,
                &request_uri,
                &to_header,
                &target_clean,
                swyx_ctx.as_ref(),
                &rtp,
                &mut rx,
            )
            .await;

        self.unregister_transaction(call_id).await;
        self.outgoing.lock().await.remove(call_id);

        if result.is_err() {
            rtp.stop();
        }
        result
    }

    /// Requests cancellation of a still ringing outgoing call. Returns false if no such call exists.
    pub async fn cancel_outgoing(&self, call_id: &str) -> bool {
        match self.outgoing.lock().await.get_mut(call_id) {
            Some(o) => {
                info!("Cancel requested for outgoing call {}", call_id);
                o.cancel_requested = true;
                true
            }
            None => false,
        }
    }

    async fn cancel_requested(&self, call_id: &str) -> bool {
        self.outgoing
            .lock()
            .await
            .get(call_id)
            .map(|o| o.cancel_requested)
            .unwrap_or(false)
    }

    async fn run_outgoing_invite(
        &self,
        call_id: &str,
        request_uri: &str,
        to_header: &str,
        target_number: &str,
        swyx_ctx: Option<&SwyxLineContext>,
        rtp: &Arc<RtpSession>,
        rx: &mut mpsc::Receiver<SipMessage>,
    ) -> Result<ActiveCall> {
        let sdp_offer = SdpSession::build_offer(&self.config.local_ip, rtp.local_port);
        let from_tag = new_tag();
        let from_header = format!("<{}>;tag={}", self.config.local_aor(), from_tag);
        let ring_deadline = Instant::now() + MAX_RING_TIME;

        let mut auth: Option<(String, String)> = None;
        let mut auth_attempts = 0;
        let mut ringing_emitted = false;
        let mut cancel_sent_at: Option<Instant> = None;
        let mut timed_out = false;

        loop {
            let cseq = self.next_cseq();
            let mut inv = SipMessage::new_request("INVITE", request_uri);
            inv.set_header("Via", &self.config.via(&new_branch()));
            inv.set_header("Max-Forwards", "70");
            inv.set_header("From", &from_header);
            inv.set_header("To", to_header);
            inv.set_header("Call-ID", call_id);
            inv.set_header("CSeq", &format!("{} INVITE", cseq));
            inv.set_header("Contact", &self.config.contact_header());
            inv.set_header("Allow", ALLOW_METHODS);
            inv.set_header("Supported", SUPPORTED_FEATURES);
            inv.set_header("User-Agent", &self.config.user_agent());
            if let Some(swyx) = swyx_ctx {
                if !swyx.own_extension.is_empty() {
                    let ppi = match swyx
                        .display_name
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        Some(name) => format!(
                            "\"{}\" <sip:{}@{};user=phone>",
                            name.replace('"', ""),
                            escape_user(&swyx.own_extension),
                            self.config.realm
                        ),
                        None => format!(
                            "<sip:{}@{};user=phone>",
                            escape_user(&swyx.own_extension),
                            self.config.realm
                        ),
                    };
                    inv.set_header("P-Preferred-Identity", &ppi);
                }
                inv.set_header("P-Early-Media", "supported");
                inv.set_header("X-IPPBXCallType", "0");
                if swyx.user_id > 0 {
                    inv.set_header("X-IPPBXOrigUserID", &swyx.user_id.to_string());
                }
                inv.set_header("X-IPPBXOrigSiteID", &swyx.site_id.max(1).to_string());
            }
            inv.set_header("Content-Type", "application/sdp");
            if let Some((ref n, ref v)) = auth {
                inv.set_header(n, v);
            }
            inv.body = sdp_offer.as_bytes().to_vec();
            let raw = inv.serialize();
            debug!("Sending SIP INVITE:\n{}", String::from_utf8_lossy(&raw));
            self.socket.send_to(&raw, self.server_addr()).await?;

            let mut any_response = false;
            let mut retx_interval = T1;
            let mut next_retx = Instant::now() + T1;
            let timer_b = Instant::now() + T1 * 64;

            // Warten auf Antworten zu dieser INVITE-Transaktion
            let next_attempt = loop {
                // Abbruch (Benutzer legt während des Klingelns auf / max. Klingeldauer)
                if Instant::now() >= ring_deadline && cancel_sent_at.is_none() && !timed_out {
                    info!("Outgoing call {} not answered within {:?} - cancelling", call_id, MAX_RING_TIME);
                    timed_out = true;
                    self.cancel_outgoing(call_id).await;
                }
                let cancel_requested = self.cancel_requested(call_id).await;
                if cancel_requested && cancel_sent_at.is_none() {
                    if any_response {
                        self.send_message(&cancel_for(&inv)).await?;
                        cancel_sent_at = Some(Instant::now());
                        info!("CANCEL sent for outgoing call {}", call_id);
                    } else if Instant::now() + Duration::from_secs(28) > timer_b {
                        // Noch keine Antwort nach ~4 s: CANCEL ist erst nach einer vorläufigen
                        // Antwort erlaubt - Rufaufbau lokal abbrechen.
                        return Err(anyhow!("Anruf abgebrochen"));
                    }
                }
                if let Some(t) = cancel_sent_at {
                    if t.elapsed() > Duration::from_secs(8) {
                        return Err(anyhow!("Anruf abgebrochen"));
                    }
                }
                if !any_response && Instant::now() >= timer_b {
                    return Err(anyhow!("Keine Antwort vom SIP-Server (Timeout)"));
                }

                let msg = match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                    Ok(Some(m)) => m,
                    Ok(None) => return Err(anyhow!("SIP-Transaktionskanal geschlossen")),
                    Err(_) => {
                        if !any_response && Instant::now() >= next_retx {
                            let _ = self.socket.send_to(&raw, self.server_addr()).await;
                            retx_interval *= 2;
                            next_retx = Instant::now() + retx_interval;
                        }
                        continue;
                    }
                };

                // Nur Antworten auf DIESE INVITE-Transaktion (z. B. nicht das 200 OK auf CANCEL)
                if !matches_cseq(&msg, cseq, "INVITE") {
                    continue;
                }
                any_response = true;
                let code = msg.status_code().unwrap_or(0);

                match code {
                    100..=199 => {
                        if (code == 180 || code == 183) && !ringing_emitted {
                            info!("Call {} ringing at destination ({})", call_id, code);
                            ringing_emitted = true;
                            let _ = self
                                .event_tx
                                .send(SipEvent::CallRinging {
                                    call_id: call_id.to_string(),
                                })
                                .await;
                        }
                        // Early Media (z. B. Freizeichen/Ansagen der Anlage) hörbar machen
                        let body = String::from_utf8_lossy(&msg.body).to_string();
                        if body.contains("m=audio") && apply_remote_sdp(rtp, &body).is_some() {
                            rtp.start();
                        }
                    }
                    200..=299 => {
                        let remote_tag = msg.to_tag();
                        let remote_to = msg.get_header("To").unwrap_or(to_header).to_string();
                        let remote_target = msg.contact_uri().unwrap_or_else(|| request_uri.to_string());

                        let body = String::from_utf8_lossy(&msg.body).to_string();
                        if body.contains("m=audio") {
                            apply_remote_sdp(rtp, &body);
                        } else {
                            warn!("200 OK for call {} without SDP answer", call_id);
                        }

                        // ACK für 2xx (eigene Transaktion, neuer Branch)
                        let mut ack = SipMessage::new_request("ACK", &remote_target);
                        ack.set_header("Via", &self.config.via(&new_branch()));
                        ack.set_header("Max-Forwards", "70");
                        ack.set_header("Contact", &self.config.contact_header());
                        ack.set_header("From", &from_header);
                        ack.set_header("To", &remote_to);
                        ack.set_header("Call-ID", call_id);
                        ack.set_header("CSeq", &format!("{} ACK", cseq));
                        ack.set_header("User-Agent", &self.config.user_agent());
                        if let Some((ref n, ref v)) = auth {
                            ack.set_header(n, v);
                        }
                        let ack_raw = ack.serialize();
                        self.socket.send_to(&ack_raw, self.server_addr()).await?;

                        let call = ActiveCall {
                            call_id: call_id.to_string(),
                            remote_tag,
                            local_tag: from_tag.clone(),
                            target_number: target_number.to_string(),
                            cseq,
                            rtp_session: Some(rtp.clone()),
                            on_hold: false,
                            local_header: from_header.clone(),
                            remote_header: remote_to,
                            remote_target,
                            is_incoming: false,
                        };

                        if cancel_sent_at.is_some() || self.cancel_requested(call_id).await {
                            // 200 OK hat sich mit CANCEL gekreuzt -> sofort wieder auflegen
                            info!("Call {} answered after cancel - hanging up", call_id);
                            let _ = self.hangup(&call).await;
                            return Err(anyhow!("Anruf abgebrochen"));
                        }

                        rtp.start();
                        self.established.lock().await.insert(
                            call_id.to_string(),
                            DialogState {
                                rtp: Some(rtp.clone()),
                                ok_response: None,
                                ack_received: Arc::new(AtomicBool::new(true)),
                                ack_request: Some(ack_raw),
                            },
                        );
                        info!("Call {} answered (200 OK)", call_id);
                        return Ok(call);
                    }
                    401 | 407 => {
                        self.send_message(&ack_for_non2xx(&inv, &msg)).await?;
                        if cancel_sent_at.is_some() || self.cancel_requested(call_id).await {
                            return Err(anyhow!("Anruf abgebrochen"));
                        }
                        auth_attempts += 1;
                        if auth_attempts > 2 {
                            return Err(anyhow!(sip_failure_reason(code, &reason_phrase(&msg))));
                        }
                        let ch = msg
                            .get_header("Proxy-Authenticate")
                            .or_else(|| msg.get_header("WWW-Authenticate"))
                            .ok_or_else(|| anyhow!("Missing authenticate header in INVITE challenge"))?;
                        auth = Some(
                            DigestAuth::authorize(
                                ch,
                                code == 407,
                                &self.config.username,
                                &self.config.password,
                                "INVITE",
                                request_uri,
                            )
                            .ok_or_else(|| anyhow!("Failed to parse challenge in INVITE"))?,
                        );
                        break true;
                    }
                    300..=699 => {
                        self.send_message(&ack_for_non2xx(&inv, &msg)).await?;
                        if code == 487 && (cancel_sent_at.is_some() || timed_out) {
                            return Err(anyhow!(if timed_out {
                                "Keine Annahme (Zeitüberschreitung)"
                            } else {
                                "Anruf abgebrochen"
                            }));
                        }
                        info!("Outgoing call {} failed: {} {}", call_id, code, reason_phrase(&msg));
                        return Err(anyhow!(sip_failure_reason(code, &reason_phrase(&msg))));
                    }
                    _ => {}
                }
            };

            if !next_attempt {
                return Err(anyhow!("Rufaufbau fehlgeschlagen"));
            }
        }
    }

    /// Sends a request inside an established dialog (BYE, REFER, INFO, ...) incl. digest auth
    async fn send_in_dialog_request(
        &self,
        call: &ActiveCall,
        method: &str,
        extra_headers: &[(&str, String)],
        body: Option<(&str, Vec<u8>)>,
    ) -> Result<SipMessage> {
        let mut rx = self.register_transaction(&call.call_id).await;
        let mut auth: Option<(String, String)> = None;

        let result: Result<SipMessage> = async {
            for attempt in 0..3 {
                let cseq = self.next_cseq();
                let mut req = SipMessage::new_request(method, &call.remote_target);
                req.set_header("Via", &self.config.via(&new_branch()));
                req.set_header("Max-Forwards", "70");
                req.set_header("From", &call.local_header);
                req.set_header("To", &call.remote_header);
                req.set_header("Call-ID", &call.call_id);
                req.set_header("CSeq", &format!("{} {}", cseq, method));
                req.set_header("Contact", &self.config.contact_header());
                req.set_header("User-Agent", &self.config.user_agent());
                for (k, v) in extra_headers {
                    req.set_header(k, v);
                }
                if let Some((ctype, ref bytes)) = body {
                    req.set_header("Content-Type", ctype);
                    req.body = bytes.clone();
                }
                if let Some((ref n, ref v)) = auth {
                    req.set_header(n, v);
                }

                let resp = self
                    .send_and_wait_final(&req, &mut rx, method, cseq, Duration::from_secs(8))
                    .await?;
                let code = resp.status_code().unwrap_or(0);
                if (code == 401 || code == 407) && attempt < 2 {
                    let ch = resp
                        .get_header("Proxy-Authenticate")
                        .or_else(|| resp.get_header("WWW-Authenticate"))
                        .ok_or_else(|| anyhow!("Missing authenticate header in {} challenge", method))?;
                    auth = DigestAuth::authorize(
                        ch,
                        code == 407,
                        &self.config.username,
                        &self.config.password,
                        method,
                        &call.remote_target,
                    );
                    continue;
                }
                return Ok(resp);
            }
            Err(anyhow!("{}: authentication failed", method))
        }
        .await;

        self.unregister_transaction(&call.call_id).await;
        result
    }

    /// Hangs up an active call with BYE
    pub async fn hangup(&self, call: &ActiveCall) -> Result<()> {
        info!("Hanging up call {}", call.call_id);

        if let Some(ref rtp) = call.rtp_session {
            rtp.stop();
        }
        if let Some(d) = self.established.lock().await.remove(&call.call_id) {
            d.ack_received.store(true, Ordering::SeqCst);
        }

        match self.send_in_dialog_request(call, "BYE", &[], None).await {
            Ok(resp) => {
                let code = resp.status_code().unwrap_or(0);
                if (200..300).contains(&code) || code == 481 {
                    info!("Call {} terminated", call.call_id);
                } else {
                    warn!("BYE for call {} answered with {}", call.call_id, code);
                }
            }
            Err(e) => warn!("BYE for call {} not confirmed: {}", call.call_id, e),
        }
        Ok(())
    }

    /// Answers a pending incoming call by sending 200 OK with SDP answer
    pub async fn answer_incoming_call(&self, call_id: &str) -> Result<ActiveCall> {
        info!("Answering incoming call {}", call_id);

        let dialog = {
            let mut dialogs = self.active_dialogs.lock().await;
            dialogs
                .remove(call_id)
                .ok_or_else(|| anyhow!("Anruf nicht mehr vorhanden (bereits beendet oder anderweitig angenommen)"))?
        };

        let rtp = Arc::new(RtpSession::bind().await?);
        let sdp_reply = if dialog.sdp.contains("m=audio") {
            let offer = match apply_remote_sdp(&rtp, &dialog.sdp) {
                Some(o) if o.has_supported_codec => o,
                _ => {
                    let mut resp = SipMessage::new_response(488, "Not Acceptable Here");
                    resp.set_header("Via", &dialog.via);
                    resp.set_header("From", &dialog.from);
                    resp.set_header("To", &add_tag(&dialog.to, &dialog.local_tag));
                    resp.set_header("Call-ID", call_id);
                    resp.set_header("CSeq", &dialog.cseq);
                    let _ = self.socket.send_to(&resp.serialize(), dialog.source_addr).await;
                    rtp.stop();
                    return Err(anyhow!("Kein gemeinsamer Audio-Codec (G.711 wird nicht angeboten)"));
                }
            };
            SdpSession::build_answer(
                &self.config.local_ip,
                rtp.local_port,
                offer.selected_payload_type,
                offer.dtmf_payload_type,
                MediaDirection::answer_for(offer.direction),
            )
        } else {
            // Delayed Offer: wir bieten an, die Antwort kommt im ACK
            SdpSession::build_offer(&self.config.local_ip, rtp.local_port)
        };
        rtp.start();

        let to_with_tag = add_tag(&dialog.to, &dialog.local_tag);
        let mut resp = SipMessage::new_response(200, "OK");
        resp.set_header("Via", &dialog.via);
        resp.set_header("From", &dialog.from);
        resp.set_header("To", &to_with_tag);
        resp.set_header("Call-ID", call_id);
        resp.set_header("CSeq", &dialog.cseq);
        resp.set_header("Contact", &self.config.contact_header());
        resp.set_header("Allow", ALLOW_METHODS);
        resp.set_header("Supported", SUPPORTED_FEATURES);
        resp.set_header("User-Agent", &self.config.user_agent());
        if let Some(ref rr) = dialog.record_route {
            resp.set_header("Record-Route", rr);
        }
        resp.set_header("Content-Type", "application/sdp");
        resp.body = sdp_reply.into_bytes();
        let raw = resp.serialize();
        self.socket.send_to(&raw, dialog.source_addr).await?;

        let ack_flag = Arc::new(AtomicBool::new(false));
        self.established.lock().await.insert(
            call_id.to_string(),
            DialogState {
                rtp: Some(rtp.clone()),
                ok_response: Some(raw.clone()),
                ack_received: ack_flag.clone(),
                ack_request: None,
            },
        );

        // 2xx-Retransmission bis zum ACK (RFC 3261 §13.3.1.4)
        let sock = self.socket.clone();
        let dest = dialog.source_addr;
        let cid = call_id.to_string();
        tokio::spawn(async move {
            let start = Instant::now();
            let mut interval = T1;
            loop {
                tokio::time::sleep(interval).await;
                if ack_flag.load(Ordering::SeqCst) {
                    break;
                }
                if start.elapsed() > T1 * 64 {
                    warn!("No ACK received for answered call {}", cid);
                    break;
                }
                let _ = sock.send_to(&raw, dest).await;
                interval = (interval * 2).min(T2);
            }
        });

        let caller_number = extract_user(&dialog.caller_display)
            .or_else(|| extract_user(&dialog.from))
            .unwrap_or_default();
        let remote_target = dialog
            .contact
            .clone()
            .or_else(|| extract_uri(&dialog.from))
            .unwrap_or_else(|| format!("sip:{}@{}", caller_number, self.config.realm));

        Ok(ActiveCall {
            call_id: call_id.to_string(),
            remote_tag: tag_of(&dialog.from),
            local_tag: dialog.local_tag.clone(),
            target_number: caller_number,
            cseq: 1,
            rtp_session: Some(rtp),
            on_hold: false,
            local_header: to_with_tag,
            remote_header: dialog.from.clone(),
            remote_target,
            is_incoming: true,
        })
    }

    /// Rejects a pending incoming call with a status code (e.g. 486 Busy Here or 603 Decline)
    pub async fn reject_incoming_call(&self, call_id: &str, code: u16) -> Result<()> {
        info!("Rejecting incoming call {} with code {}", call_id, code);

        let dialog = {
            let mut dialogs = self.active_dialogs.lock().await;
            dialogs
                .remove(call_id)
                .ok_or_else(|| anyhow!("Incoming call dialog not found: {}", call_id))?
        };

        let reason = match code {
            486 => "Busy Here",
            603 => "Decline",
            480 => "Temporarily Unavailable",
            _ => "User Unavailable",
        };

        let mut resp = SipMessage::new_response(code, reason);
        resp.set_header("Via", &dialog.via);
        resp.set_header("From", &dialog.from);
        resp.set_header("To", &add_tag(&dialog.to, &dialog.local_tag));
        resp.set_header("Call-ID", call_id);
        resp.set_header("CSeq", &dialog.cseq);

        self.socket.send_to(&resp.serialize(), dialog.source_addr).await?;
        Ok(())
    }

    /// Deflects a still ringing incoming call to another destination (302 Moved Temporarily)
    pub async fn redirect_incoming_call(&self, call_id: &str, target: &str) -> Result<()> {
        let dialog = {
            let mut dialogs = self.active_dialogs.lock().await;
            dialogs
                .remove(call_id)
                .ok_or_else(|| anyhow!("Incoming call dialog not found: {}", call_id))?
        };
        let target_clean: String = target.chars().filter(|c| !c.is_whitespace()).collect();
        let target_uri = if target_clean.starts_with("sip:") {
            target_clean
        } else {
            format!("sip:{}@{}", escape_user(&target_clean), self.config.realm)
        };
        info!("Redirecting incoming call {} to {}", call_id, target_uri);

        let mut resp = SipMessage::new_response(302, "Moved Temporarily");
        resp.set_header("Via", &dialog.via);
        resp.set_header("From", &dialog.from);
        resp.set_header("To", &add_tag(&dialog.to, &dialog.local_tag));
        resp.set_header("Call-ID", call_id);
        resp.set_header("CSeq", &dialog.cseq);
        resp.set_header("Contact", &format!("<{}>", target_uri));
        self.socket.send_to(&resp.serialize(), dialog.source_addr).await?;
        Ok(())
    }

    /// Sends a re-INVITE with the given media direction (hold / unhold)
    async fn reinvite(&self, call: &mut ActiveCall, direction: MediaDirection) -> Result<()> {
        let rtp_port = call
            .rtp_session
            .as_ref()
            .map(|r| r.local_port)
            .unwrap_or(10000);
        let sdp_offer = SdpSession::build_offer_with_direction(&self.config.local_ip, rtp_port, direction);

        let mut rx = self.register_transaction(&call.call_id).await;
        let mut auth: Option<(String, String)> = None;

        let result: Result<()> = async {
            for attempt in 0..3 {
                let cseq = self.next_cseq();
                call.cseq = cseq;
                let mut req = SipMessage::new_request("INVITE", &call.remote_target);
                req.set_header("Via", &self.config.via(&new_branch()));
                req.set_header("Max-Forwards", "70");
                req.set_header("From", &call.local_header);
                req.set_header("To", &call.remote_header);
                req.set_header("Call-ID", &call.call_id);
                req.set_header("CSeq", &format!("{} INVITE", cseq));
                req.set_header("Contact", &self.config.contact_header());
                req.set_header("Allow", ALLOW_METHODS);
                req.set_header("Supported", SUPPORTED_FEATURES);
                req.set_header("User-Agent", &self.config.user_agent());
                req.set_header("Content-Type", "application/sdp");
                if let Some((ref n, ref v)) = auth {
                    req.set_header(n, v);
                }
                req.body = sdp_offer.as_bytes().to_vec();

                let resp = self
                    .send_and_wait_final(&req, &mut rx, "INVITE", cseq, Duration::from_secs(15))
                    .await?;
                let code = resp.status_code().unwrap_or(0);

                if (200..300).contains(&code) {
                    let mut ack = SipMessage::new_request("ACK", &call.remote_target);
                    ack.set_header("Via", &self.config.via(&new_branch()));
                    ack.set_header("Max-Forwards", "70");
                    ack.set_header("From", &call.local_header);
                    ack.set_header("To", resp.get_header("To").unwrap_or(&call.remote_header));
                    ack.set_header("Call-ID", &call.call_id);
                    ack.set_header("CSeq", &format!("{} ACK", cseq));
                    if let Some((ref n, ref v)) = auth {
                        ack.set_header(n, v);
                    }
                    self.send_message(&ack).await?;

                    let body = String::from_utf8_lossy(&resp.body).to_string();
                    if body.contains("m=audio") {
                        if let Some(ref rtp) = call.rtp_session {
                            apply_remote_sdp(rtp, &body);
                        }
                    }
                    return Ok(());
                }

                self.send_message(&ack_for_non2xx(&req, &resp)).await?;
                if (code == 401 || code == 407) && attempt < 2 {
                    let ch = resp
                        .get_header("Proxy-Authenticate")
                        .or_else(|| resp.get_header("WWW-Authenticate"))
                        .ok_or_else(|| anyhow!("Missing authenticate header in re-INVITE challenge"))?;
                    auth = DigestAuth::authorize(
                        ch,
                        code == 407,
                        &self.config.username,
                        &self.config.password,
                        "INVITE",
                        &call.remote_target,
                    );
                    continue;
                }
                return Err(anyhow!("re-INVITE rejected by peer: status {}", code));
            }
            Err(anyhow!("re-INVITE: authentication failed"))
        }
        .await;

        self.unregister_transaction(&call.call_id).await;
        result
    }

    /// Puts an active call on hold via re-INVITE with a=sendonly and pauses RTP audio
    pub async fn hold(&self, call: &mut ActiveCall) -> Result<()> {
        if call.on_hold {
            info!("Call {} is already on hold", call.call_id);
            return Ok(());
        }
        info!("Putting call {} on hold...", call.call_id);
        self.reinvite(call, MediaDirection::SendOnly).await?;
        if let Some(ref rtp) = call.rtp_session {
            rtp.pause();
        }
        call.on_hold = true;
        info!("Call {} placed on hold successfully", call.call_id);
        Ok(())
    }

    /// Resumes a held call via re-INVITE with a=sendrecv and resumes RTP audio
    pub async fn unhold(&self, call: &mut ActiveCall) -> Result<()> {
        if !call.on_hold {
            info!("Call {} is not on hold", call.call_id);
            return Ok(());
        }
        info!("Resuming call {} from hold...", call.call_id);
        self.reinvite(call, MediaDirection::SendRecv).await?;
        if let Some(ref rtp) = call.rtp_session {
            rtp.resume();
        }
        call.on_hold = false;
        info!("Call {} resumed from hold successfully", call.call_id);
        Ok(())
    }

    /// Transfers an active call to a new destination via SIP REFER (RFC 3515)
    pub async fn transfer(&self, call: &mut ActiveCall, target: &str) -> Result<()> {
        info!("Initiating call transfer for {} to {}...", call.call_id, target);

        let target_clean: String = target.chars().filter(|c| !c.is_whitespace()).collect();
        let refer_to_uri = if target_clean.starts_with("sip:") {
            target_clean
        } else {
            format!("sip:{}@{}", escape_user(&target_clean), self.config.realm)
        };

        let headers = [
            ("Refer-To", format!("<{}>", refer_to_uri)),
            ("Referred-By", format!("<{}>", self.config.local_aor())),
        ];
        let resp = self.send_in_dialog_request(call, "REFER", &headers, None).await?;
        let code = resp.status_code().unwrap_or(0);
        if (200..300).contains(&code) {
            info!("Call transfer accepted with status {}! Terminating our leg...", code);
            let _ = self.hangup(call).await;
            info!("Transfer of call {} to {} complete", call.call_id, target);
            Ok(())
        } else {
            Err(anyhow!("Transfer rejected by peer with status {}", code))
        }
    }

    /// Performs an attended transfer (consultative transfer): transfers call1 to call2 via SIP REFER with Replaces header (RFC 3891)
    pub async fn attended_transfer(&self, call1: &mut ActiveCall, call2: &ActiveCall) -> Result<()> {
        info!(
            "Initiating Attended Transfer: connecting call {} to call {}",
            call1.call_id, call2.call_id
        );

        let remote_tag = call2.remote_tag.as_deref().unwrap_or("");
        let replaces_param = format!(
            "Replaces={}%3Bto-tag%3D{}%3Bfrom-tag%3D{}",
            call2.call_id.replace('@', "%40"),
            remote_tag,
            call2.local_tag
        );
        let refer_to_target = if call2.target_number.starts_with("sip:") {
            call2.target_number.clone()
        } else if !call2.target_number.is_empty() && !call2.target_number.contains('<') {
            format!("sip:{}@{}", escape_user(&call2.target_number), self.config.realm)
        } else {
            call2.remote_target.clone()
        };

        let headers = [
            ("Refer-To", format!("<{}?{}>", refer_to_target, replaces_param)),
            ("Referred-By", format!("<{}>", self.config.local_aor())),
        ];
        let resp = self.send_in_dialog_request(call1, "REFER", &headers, None).await?;
        let code = resp.status_code().unwrap_or(0);
        if !(200..300).contains(&code) {
            return Err(anyhow!("Attended transfer rejected by peer with status {}", code));
        }

        info!("Attended transfer accepted with status {}! Terminating our legs...", code);
        let _ = self.hangup(call1).await;
        // Die Gegenstelle ersetzt call2 per INVITE/Replaces und beendet ihn selbst -
        // nur falls das nicht passiert, legen wir nach kurzer Zeit auf.
        tokio::time::sleep(Duration::from_secs(3)).await;
        if self.established.lock().await.contains_key(&call2.call_id) {
            let _ = self.hangup(call2).await;
        }
        info!("Attended transfer of call {} to {} complete", call1.call_id, call2.call_id);
        Ok(())
    }

    /// Sends a lightweight SIP OPTIONS keepalive packet to maintain NAT pinholes
    pub async fn send_keepalive(&self) -> Result<()> {
        let uri = format!("sip:{}", self.config.realm);
        let mut msg = SipMessage::new_request("OPTIONS", &uri);
        msg.set_header(
            "Via",
            &format!(
                "SIP/2.0/UDP {}:{};branch=z9hG4bKkeepalive{}",
                self.config.local_ip, self.config.local_port, Uuid::new_v4()
            ),
        );
        msg.set_header("Max-Forwards", "70");
        msg.set_header(
            "From",
            &format!("<{}>;tag={}", self.config.local_aor(), new_tag()),
        );
        msg.set_header("To", &format!("<sip:{}>", self.config.realm));
        msg.set_header("Call-ID", &format!("keepalive-{}@{}", Uuid::new_v4(), self.config.local_ip));
        msg.set_header("CSeq", &format!("{} OPTIONS", self.next_cseq()));

        self.send_message(&msg).await
    }

    /// Spawns a background task maintaining NAT keepalive every `interval_secs` and re-registering
    pub fn start_keepalive_task(self: Arc<Self>, interval_secs: u64) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut tick_count = 0u64;
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(interval_secs)).await;
                tick_count += interval_secs;

                let _ = self.send_keepalive().await;

                // Re-register every 1800 seconds (30 minutes)
                if tick_count >= 1800 {
                    tick_count = 0;
                    info!("Refreshing periodic SIP registration...");
                    let _ = self.register(3600).await;
                }
            }
        })
    }

    /// Sends a SIP MESSAGE (RFC 3428) text message to target_number (generic SIP providers)
    pub async fn send_instant_message(&self, target_number: &str, content: &str) -> Result<()> {
        self.send_message_request(
            target_number,
            "text/plain; charset=utf-8",
            content.as_bytes().to_vec(),
            &[],
        )
        .await
    }

    /// Sends a Swyx chat message (`application/x-ippbxim+xml`), compatible with SwyxIt!/Swyx Messenger.
    /// `orig_user_id` = eigene Swyx-UserID, `dest_user_id` = Swyx-UserID (EmployeeId) des Empfängers.
    pub async fn send_swyx_instant_message(
        &self,
        target_number: &str,
        content: &str,
        orig_user_id: Option<i32>,
        dest_user_id: Option<i32>,
    ) -> Result<()> {
        static IM_ID: AtomicU32 = AtomicU32::new(1);
        let id = IM_ID.fetch_add(1, Ordering::SeqCst) as i32;
        let im = super::ippbx_im::IpPbxIm::new_message(id, content);

        let mut headers: Vec<(String, String)> = Vec::new();
        headers.push(("X-IPPBXDestIP".into(), "0.0.0.0:0".into()));
        headers.push(("X-IPPBXOrigIP".into(), "0.0.0.0:0".into()));
        if let Some(uid) = dest_user_id {
            headers.push(("X-IPPBXDestUserID".into(), uid.to_string()));
        } else {
            warn!("Sending Swyx IM without X-IPPBXDestUserID; message routing may fail on PBX");
        }
        let site_id = self
            .swyx_context()
            .map(|c| c.site_id.max(1))
            .unwrap_or(1)
            .to_string();
        headers.push(("X-IPPBXDestSiteID".into(), site_id.clone()));
        if let Some(uid) = orig_user_id {
            headers.push(("X-IPPBXOrigUserID".into(), uid.to_string()));
        }
        headers.push(("X-IPPBXOrigSiteID".into(), site_id));

        self.send_message_request(
            target_number,
            super::ippbx_im::CONTENT_TYPE,
            im.to_xml().into_bytes(),
            &headers,
        )
        .await
    }

    /// Generic SIP MESSAGE sender incl. digest authentication
    async fn send_message_request(
        &self,
        target_number: &str,
        content_type: &str,
        body: Vec<u8>,
        extra_headers: &[(String, String)],
    ) -> Result<()> {
        let server_addr = self.server_addr();
        do_send_message_with_auth(
            &self.socket,
            server_addr,
            &self.config,
            &self.cseq,
            &self.pending_transactions,
            target_number,
            content_type,
            body,
            extra_headers,
        )
        .await
    }

    /// Publishes user presence via standard SIP PUBLISH (RFC 3903 / PIDF+XML)
    pub async fn publish_presence(&self, open: bool, note: &str) -> Result<()> {
        let uri = format!("sip:{}@{}", self.config.uri_user(), self.config.realm);
        let mut msg = SipMessage::new_request("PUBLISH", &uri);
        let branch = format!("z9hG4bK{}", Uuid::new_v4().simple());
        msg.set_header(
            "Via",
            &format!(
                "SIP/2.0/UDP {}:{};branch={};rport",
                self.config.local_ip, self.config.local_port, branch
            ),
        );
        msg.set_header("Max-Forwards", "70");
        msg.set_header(
            "From",
            &format!("<{}>;tag={:x}", uri, rand::random::<u32>()),
        );
        msg.set_header("To", &format!("<{}>", uri));
        msg.set_header(
            "Call-ID",
            &format!("pub-{}@{}", Uuid::new_v4(), self.config.local_ip),
        );
        msg.set_header("CSeq", &format!("{} PUBLISH", self.next_cseq()));
        msg.set_header("Event", "presence");
        msg.set_header("Expires", "3600");
        msg.set_header("Content-Type", "application/pidf+xml");

        let basic = if open { "open" } else { "closed" };
        let pidf = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<presence xmlns="urn:ietf:params:xml:ns:pidf" entity="{uri}">
  <tuple id="deskphone">
    <status><basic>{basic}</basic></status>
    <note>{note}</note>
  </tuple>
</presence>"#
        );
        msg.body = pidf.into_bytes();
        self.send_message(&msg).await
    }

    /// Subscribes to standard SIP Message Waiting Indicator (RFC 3842 Event: message-summary)
    pub async fn subscribe_voicemail(&self) -> Result<()> {
        let uri_user = self.config.uri_user();
        let uri = format!("sip:{}@{}", uri_user, self.config.realm);
        let mut msg = SipMessage::new_request("SUBSCRIBE", &uri);
        let branch = format!("z9hG4bK{}", Uuid::new_v4().simple());
        msg.set_header(
            "Via",
            &format!(
                "SIP/2.0/UDP {}:{};branch={};rport",
                self.config.local_ip, self.config.local_port, branch
            ),
        );
        msg.set_header("Max-Forwards", "70");
        msg.set_header(
            "From",
            &format!("<{}>;tag={:x}", uri, rand::random::<u32>()),
        );
        msg.set_header("To", &format!("<{}>", uri));
        msg.set_header(
            "Call-ID",
            &format!("mwi-{}@{}", Uuid::new_v4(), self.config.local_ip),
        );
        msg.set_header("CSeq", &format!("{} SUBSCRIBE", self.next_cseq()));
        msg.set_header("Event", "message-summary");
        msg.set_header("Accept", "application/simple-message-summary");
        msg.set_header("Expires", "3600");
        msg.set_header(
            "Contact",
            &format!(
                "<sip:{}@{}:{}>",
                uri_user, self.config.local_ip, self.config.local_port
            ),
        );
        self.send_message(&msg).await
    }
}

/// Sends a SIP MESSAGE request with digest authentication (RFC 3428 / RFC 2617).
/// Used by both outgoing instant messages and incoming Swyx IM acknowledgements.
pub async fn do_send_message_with_auth(
    socket: &UdpSocket,
    server_addr: SocketAddr,
    config: &SipConfig,
    cseq_gen: &AtomicU32,
    pending_transactions: &Mutex<HashMap<String, mpsc::Sender<SipMessage>>>,
    target_uri: &str,
    content_type: &str,
    body: Vec<u8>,
    extra_headers: &[(String, String)],
) -> Result<()> {
    let call_id = format!("msg-{}@{}", Uuid::new_v4(), config.local_ip);
    let uri_user = config.uri_user();
    let user_agent = config.user_agent();
    let is_swyx_guid_binding =
        super::ippbx_im::is_ippbx_im_content_type(content_type) && config.username.starts_with('{');

    let (to_uri, to_header) = if is_swyx_guid_binding {
        // CLMgr.exe (0xac78dc..0xac7913 -> SwSIPEndp::SendMessageRequest 0x9e7e69)
        // addresses Request-URI, From, and To to the client's registered SwSIPBinding
        // (`sip:%7b<SipRegisterId>%7d@<realm>`), while routing to the peer via X-IPPBXDestUserID.
        let binding_uri = format!("sip:{}@{}", uri_user, config.realm);
        let hdr = if !target_uri.starts_with("sip:") && !target_uri.is_empty() {
            format!("\"{}\" <{}>", target_uri, binding_uri)
        } else {
            format!("<{}>", binding_uri)
        };
        (binding_uri, hdr)
    } else {
        let u = if target_uri.starts_with("sip:") {
            target_uri.to_string()
        } else {
            format!("sip:{}@{}", target_uri, config.realm)
        };
        let hdr = format!("<{}>", u);
        (u, hdr)
    };

    let from_header = format!(
        "<sip:{}@{}>;tag={:x}",
        uri_user,
        config.realm,
        rand::random::<u32>()
    );

    let cseq = cseq_gen.fetch_add(1, Ordering::SeqCst);
    let (tx, mut rx) = mpsc::channel(16);
    {
        let mut pending = pending_transactions.lock().await;
        pending.insert(call_id.clone(), tx);
    }

    // 1. Send initial unauthenticated MESSAGE
    let mut msg1 = SipMessage::new_request("MESSAGE", &to_uri);
    msg1.set_header(
        "Via",
        &format!(
            "SIP/2.0/UDP {}:{};branch=z9hG4bK{};rport",
            config.local_ip, config.local_port, Uuid::new_v4()
        ),
    );
    msg1.set_header("Max-Forwards", "70");
    msg1.set_header("From", &from_header);
    msg1.set_header("To", &to_header);
    msg1.set_header("Call-ID", &call_id);
    msg1.set_header("CSeq", &format!("{} MESSAGE", cseq));
    for (k, v) in extra_headers {
        msg1.set_header(k, v);
    }
    msg1.set_header("Content-Type", content_type);
    msg1.set_header("User-Agent", &user_agent);
    msg1.body = body.clone();

    socket.send_to(&msg1.serialize(), server_addr).await?;

    // 2. Wait for final response (200/202, 401 or 407), skipping provisional 1xx
    let resp1 = loop {
        let m = match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Some(m)) => m,
            _ => {
                let mut pending = pending_transactions.lock().await;
                pending.remove(&call_id);
                return Err(anyhow!("Timeout waiting for MESSAGE response"));
            }
        };
        if m.status_code().unwrap_or(0) >= 200 {
            break m;
        }
    };

    let code = resp1.status_code().unwrap_or(0);
    if code == 200 || code == 202 {
        let mut pending = pending_transactions.lock().await;
        pending.remove(&call_id);
        debug!("Instant message delivered to {}", to_uri);
        return Ok(());
    }

    if code != 401 && code != 407 {
        let mut pending = pending_transactions.lock().await;
        pending.remove(&call_id);
        return Err(anyhow!("Server rejected instant message with SIP {}", code));
    }

    // 3. Digest challenge authentication
    let auth_hdr = resp1
        .get_header("Proxy-Authenticate")
        .or_else(|| resp1.get_header("WWW-Authenticate"))
        .ok_or_else(|| anyhow!("Missing authenticate header in MESSAGE challenge"))?;

    let (realm, nonce, qop) = DigestAuth::parse_challenge(auth_hdr)
        .ok_or_else(|| anyhow!("Failed to parse challenge in MESSAGE"))?;

    let cnonce = DigestAuth::generate_cnonce();
    let digest_resp = DigestAuth::compute_response(
        &config.username,
        &config.password,
        &realm,
        &nonce,
        "MESSAGE",
        &to_uri,
        qop.as_deref(),
        1,
        Some(&cnonce),
    );

    let (h_name, h_val) = DigestAuth::format_authorization_header(
        code == 407,
        &config.username,
        &realm,
        &nonce,
        &to_uri,
        &digest_resp,
        qop.as_deref(),
        1,
        Some(&cnonce),
    );

    // 4. Send authenticated MESSAGE
    let cseq2 = cseq_gen.fetch_add(1, Ordering::SeqCst);
    let mut msg2 = SipMessage::new_request("MESSAGE", &to_uri);
    msg2.set_header(
        "Via",
        &format!(
            "SIP/2.0/UDP {}:{};branch=z9hG4bK{};rport",
            config.local_ip, config.local_port, Uuid::new_v4()
        ),
    );
    msg2.set_header("Max-Forwards", "70");
    msg2.set_header("From", &from_header);
    msg2.set_header("To", &to_header);
    msg2.set_header("Call-ID", &call_id);
    msg2.set_header("CSeq", &format!("{} MESSAGE", cseq2));
    msg2.set_header(&h_name, &h_val);
    for (k, v) in extra_headers {
        msg2.set_header(k, v);
    }
    msg2.set_header("Content-Type", content_type);
    msg2.set_header("User-Agent", &user_agent);
    msg2.body = body;

    socket.send_to(&msg2.serialize(), server_addr).await?;

    let resp2 = match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
        Ok(Some(m)) => m,
        _ => {
            let mut pending = pending_transactions.lock().await;
            pending.remove(&call_id);
            return Err(anyhow!("Timeout waiting for authenticated MESSAGE response"));
        }
    };

    let mut pending = pending_transactions.lock().await;
    pending.remove(&call_id);

    let code2 = resp2.status_code().unwrap_or(0);
    if code2 == 200 || code2 == 202 {
        info!("Instant message delivered successfully to {}", to_uri);
        Ok(())
    } else {
        Err(anyhow!("Server rejected instant message with SIP {}", code2))
    }
}

/// Parses RFC 3842 `application/simple-message-summary` body
pub fn parse_mwi_summary(body: &str) -> (bool, u32, u32, Option<String>) {
    let mut waiting = false;
    let mut new_msgs = 0u32;
    let mut old_msgs = 0u32;
    let mut account = None;

    for line in body.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("messages-waiting:") {
            let val = trimmed["messages-waiting:".len()..].trim();
            waiting = val.eq_ignore_ascii_case("yes");
        } else if lower.starts_with("message-account:") {
            let val = trimmed["message-account:".len()..].trim().to_string();
            if !val.is_empty() {
                account = Some(val);
            }
        } else if lower.starts_with("voice-message:") {
            let val = trimmed["voice-message:".len()..].trim();
            // Format: "2/5 (1/0)" or "2/5"
            let counts_part = val.split_whitespace().next().unwrap_or("");
            let mut parts = counts_part.split('/');
            if let Some(n) = parts.next().and_then(|s| s.parse::<u32>().ok()) {
                new_msgs = n;
            }
            if let Some(o) = parts.next().and_then(|s| s.parse::<u32>().ok()) {
                old_msgs = o;
            }
        }
    }

    if new_msgs > 0 {
        waiting = true;
    }

    (waiting, new_msgs, old_msgs, account)
}

/// Filters out non-chat SIP control payloads (such as Swyx `application/x-ippbxcti+xml`
/// `<CtiPairingRelease/>`, CSTA XML, PIDF, dialog-info, or empty keepalives) after `IpPbxIM`
/// chat messages have already been handled.
pub(crate) fn is_non_chat_sip_message(content_type: &str, body_text: &str) -> bool {
    let trimmed = body_text.trim_start_matches('\u{feff}').trim();
    if trimmed.is_empty() {
        return true;
    }

    let media_type = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if !media_type.is_empty() && media_type != "text/plain" && media_type != "text/html" {
        return true;
    }

    let lower_head = trimmed
        .chars()
        .take(64)
        .collect::<String>()
        .to_ascii_lowercase();
    lower_head.starts_with("<?xml")
        || lower_head.starts_with("<messages")
        || lower_head.starts_with("<cti")
}

impl ActiveCall {
    /// Sends an RFC 4733 DTMF tone sequence ('0'-'9', '*', '#') over the active call
    pub async fn send_dtmf(&self, digit: char) -> Result<()> {
        if let Some(ref rtp) = self.rtp_session {
            rtp.send_dtmf(digit).await
        } else {
            Err(anyhow!("No active RTP session for call {}", self.call_id))
        }
    }

    /// Sets microphone mute state on the call's RTP session
    pub fn set_muted(&self, muted: bool) {
        if let Some(ref rtp) = self.rtp_session {
            rtp.set_muted(muted);
        }
    }

    /// Returns true if the microphone is muted on the call's RTP session
    pub fn is_muted(&self) -> bool {
        self.rtp_session.as_ref().map(|r| r.is_muted()).unwrap_or(false)
    }

    /// Puts the call on hold using the provided SipClient
    pub async fn hold(&mut self, sip: &SipClient) -> Result<()> {
        sip.hold(self).await
    }

    /// Resumes the call from hold using the provided SipClient
    pub async fn unhold(&mut self, sip: &SipClient) -> Result<()> {
        sip.unhold(self).await
    }

    /// Transfers the call to another target using the provided SipClient
    pub async fn transfer(&mut self, sip: &SipClient, target: &str) -> Result<()> {
        sip.transfer(self, target).await
    }

    /// Performs an attended transfer connecting this call to target_call (RFC 3891)
    pub async fn attended_transfer(&mut self, sip: &SipClient, target_call: &ActiveCall) -> Result<()> {
        sip.attended_transfer(self, target_call).await
    }
}
