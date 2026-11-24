pub mod models;
pub mod rest;
pub mod soap;

use anyhow::{anyhow, Result};
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use tracing::{debug, info};

pub use models::*;
pub use rest::{SwyxRestClient, SwyxUserProfile};
pub use soap::{SoapBuilder, SoapParser};

/// Client for communicating with the Swyx ConfigDataStore (CDS)
#[derive(Debug, Clone)]
pub struct CdsClient {
    pub host: String,
    pub http_port: u16,
    pub use_tls: bool,
    client: reqwest::Client,
}

impl CdsClient {
    pub fn new(host: String, http_port: u16, use_tls: bool) -> Self {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true) // Swyx often uses self-signed IpPbx.cer
            .build()
            .unwrap_or_default();

        Self {
            host,
            http_port,
            use_tls,
            client,
        }
    }

    fn base_url(&self) -> String {
        let proto = if self.use_tls { "https" } else { "http" };
        format!("{}://{}:{}", proto, self.host, self.http_port)
    }

    /// Sends a SOAP POST request to a CDS service endpoint with the required SOAPAction header
    async fn send_soap(&self, endpoint: &str, soap_action: &str, body: &str) -> Result<String> {
        let url = format!("{}/{}", self.base_url(), endpoint);
        debug!("Sending SOAP request to {} [Action: {}]", url, soap_action);

        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("text/xml; charset=utf-8"),
        );
        headers.insert(
            "SOAPAction",
            HeaderValue::from_str(&format!("\"{}\"", soap_action))?,
        );

        let resp = self
            .client
            .post(&url)
            .headers(headers)
            .body(body.to_string())
            .send()
            .await?;

        let status = resp.status();
        let resp_text = resp.text().await?;

        if !status.is_success() {
            return Err(anyhow!(
                "SOAP request failed with HTTP {}: {}",
                status,
                resp_text
            ));
        }

        Ok(resp_text)
    }

    /// Authenticates with username and password, returns an AuthenticationResult
    pub async fn login(&self, username: &str, password: &str) -> Result<AuthenticationResult> {
        info!("Logging in to Swyx CDS at {} for user {}", self.host, username);
        let xml = SoapBuilder::build_login_request(username, password);
        let resp = self
            .send_soap(
                "ConfigDataStore/CLoginImpl",
                "http://tempuri.org/ILogin/Login",
                &xml,
            )
            .await?;

        let auth = SoapParser::parse_login_response(&resp)?;
        info!(
            "Login successful! UserID: {}, AccessToken: {}...",
            auth.user_id,
            &auth.access_token.chars().take(16).collect::<String>()
        );
        Ok(auth)
    }

    /// Retrieves SIP credentials for the specified user ID
    pub async fn get_sip_credentials(
        &self,
        user_id: i32,
        username: &str,
        access_token: &str,
    ) -> Result<UserSipCredentials> {
        let xml = SoapBuilder::build_get_sip_credentials(user_id, username, access_token);
        let resp = self
            .send_soap(
                "ConfigDataStore/CPhoneClientFacadeImpl",
                "http://tempuri.org/IPhoneClientFacade/GetSipCredentials",
                &xml,
            )
            .await?;

        let mut creds = SoapParser::parse_sip_credentials(&resp, user_id)?;

        // If realm is empty, default to server host
        if creds.sip_realm.is_empty() {
            creds.sip_realm = self.host.clone();
        }

        // Fetch PSK (SIP password)
        if let Ok(psk) = self.get_psk(user_id, username, access_token).await {
            creds.psk = Some(psk);
        }

        Ok(creds)
    }

    /// Retrieves the SIP pre-shared key (password) for the user
    pub async fn get_psk(&self, user_id: i32, username: &str, access_token: &str) -> Result<String> {
        let xml = SoapBuilder::build_get_psk(user_id, username, access_token);
        let resp = self
            .send_soap(
                "ConfigDataStore/CPhoneClientFacadeImpl",
                "http://tempuri.org/IPhoneClientFacade/GetCurrentPSKEx",
                &xml,
            )
            .await?;

        SoapParser::parse_psk(&resp)
    }

    /// Retrieves telephone numbers and internal extensions
    pub async fn get_numbers(
        &self,
        user_id: i32,
        username: &str,
        access_token: &str,
    ) -> Result<Vec<UserNumberInfo>> {
        let xml = SoapBuilder::build_get_numbers(user_id, username, access_token);
        let resp = self
            .send_soap(
                "ConfigDataStore/CPhoneClientFacadeImpl",
                "http://tempuri.org/IPhoneClientFacade/GetNumbers",
                &xml,
            )
            .await?;

        SoapParser::parse_numbers(&resp, user_id)
    }

    /// Retrieves call journal (call history)
    pub async fn get_call_journal(
        &self,
        user_id: i32,
        username: &str,
        access_token: &str,
    ) -> Result<Vec<CallJournalEntry>> {
        let xml = SoapBuilder::build_get_call_journal(user_id, username, access_token);
        let resp = self
            .send_soap(
                "ConfigDataStore/CPhoneClientFacadeImpl",
                "http://tempuri.org/IPhoneClientFacade/GetCallJournal",
                &xml,
            )
            .await?;

        SoapParser::parse_call_journal(&resp)
    }

    /// Retrieves rich presence
    pub async fn get_presence(
        &self,
        user_id: i32,
        username: &str,
        access_token: &str,
    ) -> Result<RichPresence> {
        let xml = SoapBuilder::build_get_presence(user_id, username, access_token);
        let resp = self
            .send_soap(
                "ConfigDataStore/CPhoneClientFacadeImpl",
                "http://tempuri.org/IPhoneClientFacade/GetRichPresenceInfo",
                &xml,
            )
            .await?;

        SoapParser::parse_presence(&resp, user_id)
    }

    /// Sets rich presence via SOAP
    pub async fn set_presence(
        &self,
        user_id: i32,
        status: PresenceStatus,
        status_text: &str,
        username: &str,
        access_token: &str,
    ) -> Result<()> {
        let xml = SoapBuilder::build_set_presence(
            user_id,
            status as i32,
            status_text,
            username,
            access_token,
        );
        let _ = self
            .send_soap(
                "ConfigDataStore/CPhoneClientFacadeImpl",
                "http://tempuri.org/IPhoneClientFacade/SetRichPresenceInfo",
                &xml,
            )
            .await?;
        Ok(())
    }

    /// Sets call forwarding via SOAP
    pub async fn set_call_forwarding(
        &self,
        user_id: i32,
        enabled: bool,
        target_number: &str,
        username: &str,
        access_token: &str,
    ) -> Result<()> {
        let xml = SoapBuilder::build_set_call_forwarding(
            user_id,
            enabled,
            target_number,
            username,
            access_token,
        );
        let _ = self
            .send_soap(
                "ConfigDataStore/CPhoneClientFacadeImpl",
                "http://tempuri.org/IPhoneClientFacade/SetCallForwarding",
                &xml,
            )
            .await?;
        Ok(())
    }

    /// Retrieves voicemail messages via SOAP
    pub async fn get_voice_messages(
        &self,
        user_id: i32,
        username: &str,
        access_token: &str,
    ) -> Result<Vec<VoiceMessageEntry>> {
        let xml = SoapBuilder::build_get_voice_messages(user_id, username, access_token);
        let resp = self
            .send_soap(
                "ConfigDataStore/CPhoneClientFacadeImpl",
                "http://tempuri.org/IPhoneClientFacade/GetVoiceMessages",
                &xml,
            )
            .await?;
        SoapParser::parse_voice_messages(&resp)
    }
}
