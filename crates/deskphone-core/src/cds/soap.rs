use quick_xml::events::Event;
use quick_xml::reader::Reader;
use uuid::Uuid;

use super::models::{
    AuthenticationResult, CallDirection, CallJournalEntry, PresenceStatus, RichPresence,
    UserNumberInfo, UserSipCredentials, VoiceMessageEntry,
};
use anyhow::{anyhow, Result};

pub const SOAP_ENV_NS: &str = "http://schemas.xmlsoap.org/soap/envelope/";
pub const TEMPURI_NS: &str = "http://tempuri.org/";
pub const IPPBX_HEADER_NS: &str = "LANPHONE.COM";
pub const MODEL_NS: &str = "http://schemas.datacontract.org/2004/07/IpPbx.Web.Model";

pub struct SoapBuilder;

impl SoapBuilder {
    /// Builds the SOAP Login request envelope
    pub fn build_login_request(username: &str, password: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="{SOAP_ENV_NS}">
  <s:Body>
    <Login xmlns="{TEMPURI_NS}">
      <credentials xmlns:d4p1="{MODEL_NS}" xmlns:i="http://www.w3.org/2001/XMLSchema-instance">
        <d4p1:Password>{password}</d4p1:Password>
        <d4p1:UserName>{username}</d4p1:UserName>
      </credentials>
    </Login>
  </s:Body>
</s:Envelope>"#
        )
    }

    /// Builds a SOAP request with IPPBX_HEADER authentication for IPhoneClientFacade
    pub fn build_authenticated_request(
        action_xml: &str,
        username: &str,
        access_token: &str,
    ) -> String {
        let req_id = Uuid::new_v4().to_string();
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="{SOAP_ENV_NS}">
  <s:Header>
    <h:IPPBX_HEADER xmlns:h="{IPPBX_HEADER_NS}">
      <Key>
        &lt;SCustomHeaderData xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xsd="http://www.w3.org/2001/XMLSchema"&gt;
          &lt;SelectedIpPbxUserName&gt;{username}&lt;/SelectedIpPbxUserName&gt;
          &lt;ClientVersion&gt;14.21.0&lt;/ClientVersion&gt;
          &lt;AccessToken&gt;{access_token}&lt;/AccessToken&gt;
          &lt;RequestId&gt;{req_id}&lt;/RequestId&gt;
        &lt;/SCustomHeaderData&gt;
      </Key>
    </h:IPPBX_HEADER>
  </s:Header>
  <s:Body>
    {action_xml}
  </s:Body>
</s:Envelope>"#
        )
    }

    pub fn build_get_sip_credentials(user_id: i32, username: &str, access_token: &str) -> String {
        let action = format!(
            r#"<GetSipCredentials xmlns="{TEMPURI_NS}"><userId>{user_id}</userId></GetSipCredentials>"#
        );
        Self::build_authenticated_request(&action, username, access_token)
    }

    pub fn build_get_psk(user_id: i32, username: &str, access_token: &str) -> String {
        let action = format!(
            r#"<GetCurrentPSKEx xmlns="{TEMPURI_NS}"><UserID>{user_id}</UserID></GetCurrentPSKEx>"#
        );
        Self::build_authenticated_request(&action, username, access_token)
    }

    pub fn build_get_numbers(user_id: i32, username: &str, access_token: &str) -> String {
        let action = format!(
            r#"<GetNumbers xmlns="{TEMPURI_NS}"><userId>{user_id}</userId></GetNumbers>"#
        );
        Self::build_authenticated_request(&action, username, access_token)
    }

    pub fn build_get_call_journal(user_id: i32, username: &str, access_token: &str) -> String {
        let action = format!(
            r#"<GetCallJournal xmlns="{TEMPURI_NS}"><userId>{user_id}</userId></GetCallJournal>"#
        );
        Self::build_authenticated_request(&action, username, access_token)
    }

    pub fn build_get_voice_messages(user_id: i32, username: &str, access_token: &str) -> String {
        let action = format!(
            r#"<GetVoiceMessages xmlns="{TEMPURI_NS}"><userId>{user_id}</userId></GetVoiceMessages>"#
        );
        Self::build_authenticated_request(&action, username, access_token)
    }

    pub fn build_get_presence(user_id: i32, username: &str, access_token: &str) -> String {
        let action = format!(
            r#"<GetRichPresenceInfo xmlns="{TEMPURI_NS}"><userId>{user_id}</userId></GetRichPresenceInfo>"#
        );
        Self::build_authenticated_request(&action, username, access_token)
    }

    pub fn build_set_presence(
        user_id: i32,
        status_code: i32,
        status_text: &str,
        username: &str,
        access_token: &str,
    ) -> String {
        let action = format!(
            r#"<SetRichPresenceInfo xmlns="{TEMPURI_NS}"><userId>{user_id}</userId><status>{status_code}</status><statusText>{status_text}</statusText></SetRichPresenceInfo>"#
        );
        Self::build_authenticated_request(&action, username, access_token)
    }

    pub fn build_set_call_forwarding(
        user_id: i32,
        enabled: bool,
        target_number: &str,
        username: &str,
        access_token: &str,
    ) -> String {
        let action = format!(
            r#"<SetCallForwarding xmlns="{TEMPURI_NS}"><userId>{user_id}</userId><unconditionalForwarding>{enabled}</unconditionalForwarding><unconditionalNumber>{target_number}</unconditionalNumber></SetCallForwarding>"#
        );
        Self::build_authenticated_request(&action, username, access_token)
    }
}

pub struct SoapParser;

impl SoapParser {
    /// Extracts text inside a specific XML tag
    pub fn extract_tag_value(xml: &str, tag_name: &str) -> Option<String> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut buf = Vec::new();
        let mut in_tag = false;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let local_name = e.local_name();
                    if local_name.as_ref() == tag_name.as_bytes() {
                        in_tag = true;
                    }
                }
                Ok(Event::Text(e)) => {
                    if in_tag {
                        if let Ok(text) = e.unescape() {
                            return Some(text.into_owned());
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    if e.local_name().as_ref() == tag_name.as_bytes() {
                        in_tag = false;
                    }
                }
                Ok(Event::Eof) => break,
                Err(_) => break,
                _ => {}
            }
            buf.clear();
        }
        None
    }

    /// Parses the LoginResponse XML into AuthenticationResult
    pub fn parse_login_response(xml: &str) -> Result<AuthenticationResult> {
        let access_token = Self::extract_tag_value(xml, "AccessToken")
            .ok_or_else(|| anyhow!("Missing AccessToken in response: {}", xml))?;
        let refresh_token = Self::extract_tag_value(xml, "RefreshToken").unwrap_or_default();
        let user_id_str = Self::extract_tag_value(xml, "UserId")
            .ok_or_else(|| anyhow!("Missing UserId in response: {}", xml))?;
        let user_id = user_id_str.parse::<i32>().unwrap_or(0);

        Ok(AuthenticationResult {
            access_token,
            refresh_token,
            user_id,
            login_succeeded: true,
        })
    }

    /// Parses GetSipCredentialsResponse
    pub fn parse_sip_credentials(xml: &str, user_id: i32) -> Result<UserSipCredentials> {
        let sip_user_name = Self::extract_tag_value(xml, "SipUserName")
            .or_else(|| Self::extract_tag_value(xml, "m_SipUserName"))
            .unwrap_or_default();

        let sip_user_id = Self::extract_tag_value(xml, "SipUserID")
            .or_else(|| Self::extract_tag_value(xml, "m_SipUserID"))
            .unwrap_or_default();

        let sip_realm = Self::extract_tag_value(xml, "SipRealm")
            .or_else(|| Self::extract_tag_value(xml, "m_SipRealm"))
            .unwrap_or_default();

        Ok(UserSipCredentials {
            user_id,
            sip_user_name,
            sip_user_id,
            sip_realm,
            psk: None,
        })
    }

    /// Parses GetCurrentPSKExResponse
    pub fn parse_psk(xml: &str) -> Result<String> {
        Self::extract_tag_value(xml, "GetCurrentPSKExResult")
            .or_else(|| Self::extract_tag_value(xml, "GetCurrentPSKResult"))
            .ok_or_else(|| anyhow!("Missing PSK in response"))
    }

    /// Parses GetNumbersResponse
    pub fn parse_numbers(xml: &str, user_id: i32) -> Result<Vec<UserNumberInfo>> {
        let internal = Self::extract_tag_value(xml, "InternalNumber")
            .or_else(|| Self::extract_tag_value(xml, "m_strInternalNumber"));
        let public = Self::extract_tag_value(xml, "PublicNumber")
            .or_else(|| Self::extract_tag_value(xml, "m_strPublicNumber"));

        Ok(vec![UserNumberInfo {
            user_id,
            internal_number: internal,
            public_number: public,
            is_fax: false,
        }])
    }

    /// Parses GetCallJournalResponse
    pub fn parse_call_journal(xml: &str) -> Result<Vec<CallJournalEntry>> {
        let mut entries = Vec::new();
        let caller = Self::extract_tag_value(xml, "CallerNumber")
            .or_else(|| Self::extract_tag_value(xml, "PeerNumber"))
            .or_else(|| Self::extract_tag_value(xml, "m_strCallerNumber"));

        if let Some(c) = caller {
            entries.push(CallJournalEntry {
                id: 1,
                caller_number: c,
                called_number: Self::extract_tag_value(xml, "CalledNumber").unwrap_or_default(),
                caller_name: Self::extract_tag_value(xml, "CallerName")
                    .or_else(|| Self::extract_tag_value(xml, "PeerName")),
                timestamp: chrono::Utc::now().to_rfc3339(),
                duration_seconds: 0,
                direction: CallDirection::Inbound,
                was_connected: true,
            });
        }
        Ok(entries)
    }

    /// Parses GetRichPresenceInfoResponse
    pub fn parse_presence(xml: &str, user_id: i32) -> Result<RichPresence> {
        let status_code = Self::extract_tag_value(xml, "Status")
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(0);
        let status_text = Self::extract_tag_value(xml, "StatusText").unwrap_or_default();

        Ok(RichPresence {
            user_id,
            status: PresenceStatus::from_i32(status_code),
            status_text,
        })
    }

    /// Parses GetVoiceMessagesResponse
    pub fn parse_voice_messages(xml: &str) -> Result<Vec<VoiceMessageEntry>> {
        let mut entries = Vec::new();
        if let Some(caller) = Self::extract_tag_value(xml, "CallerNumber")
            .or_else(|| Self::extract_tag_value(xml, "CallingNumber"))
        {
            let id = Self::extract_tag_value(xml, "VoicemailId")
                .or_else(|| Self::extract_tag_value(xml, "Id"))
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(1);
            let caller_name = Self::extract_tag_value(xml, "CallerName")
                .or_else(|| Self::extract_tag_value(xml, "CallingName"));
            let timestamp = Self::extract_tag_value(xml, "Timestamp")
                .or_else(|| Self::extract_tag_value(xml, "StartTime"))
                .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
            let duration_seconds = Self::extract_tag_value(xml, "Duration")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            let is_new = Self::extract_tag_value(xml, "IsNew")
                .map(|s| s.eq_ignore_ascii_case("true"))
                .unwrap_or(true);

            entries.push(VoiceMessageEntry {
                id,
                caller_number: caller,
                caller_name,
                timestamp,
                duration_seconds,
                is_new,
            });
        }
        Ok(entries)
    }
}
