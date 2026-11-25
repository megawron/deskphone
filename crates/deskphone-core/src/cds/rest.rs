use anyhow::{anyhow, Result};
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use super::models::{
    AuthenticationResult, CallDirection, CallJournalEntry, PresenceStatus, UserNumberInfo,
    VoiceMessageEntry,
};
use crate::events::PhonebookEntry;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestLoginRequest {
    #[serde(rename = "UserName")]
    pub username: String,
    #[serde(rename = "Password")]
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestAuthResponse {
    #[serde(rename = "AccessToken")]
    pub access_token: String,
    #[serde(rename = "RefreshToken")]
    pub refresh_token: String,
    #[serde(rename = "UserId")]
    pub user_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestNumberInfo {
    #[serde(rename = "InternalNumber")]
    pub internal_number: Option<String>,
    #[serde(rename = "Owner")]
    pub owner: Option<String>,
    #[serde(rename = "PublicNumbers")]
    pub public_numbers: Option<Vec<String>>,
    #[serde(rename = "Type")]
    pub number_type: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestSipRegisterTokenResponse {
    #[serde(rename = "PresharedKey")]
    pub preshared_key: Vec<u8>,
    #[serde(rename = "SipRegisterId")]
    pub sip_register_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestTenantInfoResponse {
    #[serde(rename = "TenantDomain")]
    pub tenant_domain: Option<String>,
    #[serde(rename = "TenantName")]
    pub tenant_name: Option<String>,
    #[serde(rename = "TenantId")]
    pub tenant_id: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestUserTenant {
    #[serde(rename = "SiteId")]
    pub site_id: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestUserProfileResponse {
    #[serde(rename = "UserId")]
    pub user_id: i32,
    #[serde(rename = "Name")]
    pub name: Option<String>,
    #[serde(rename = "Numbers")]
    pub numbers: Option<Vec<RestNumberInfo>>,
    #[serde(rename = "Tenant")]
    pub tenant: Option<RestUserTenant>,
}

#[derive(Debug, Clone)]
pub struct SwyxUserProfile {
    pub user_id: i32,
    pub display_name: Option<String>,
    pub own_extension: Option<String>,
    pub site_id: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestPhonebookResponse {
    #[serde(rename = "GlobalPhonebookEntries")]
    pub global_phonebook_entries: Option<Vec<RestPhonebookEntry>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestPhonebookEntry {
    #[serde(rename = "Name")]
    pub name: Option<String>,
    #[serde(rename = "DialNumber")]
    pub dial_number: Option<String>,
    #[serde(rename = "DisplayNumber")]
    pub display_number: Option<String>,
    #[serde(rename = "Description")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestEmployeeNumber {
    #[serde(rename = "Number")]
    pub number: String,
    #[serde(rename = "NumberType")]
    pub number_type: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestEmployeePresence {
    #[serde(rename = "Status")]
    pub status: Option<i32>,
    #[serde(rename = "StatusText")]
    pub status_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestEmployeeBitmap {
    #[serde(rename = "FileId")]
    pub file_id: Option<String>,
    #[serde(rename = "FileName")]
    pub file_name: Option<String>,
    #[serde(rename = "Compressed")]
    pub compressed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestEmployee {
    #[serde(rename = "EmployeeId")]
    pub employee_id: i32,
    #[serde(rename = "Name")]
    pub name: Option<String>,
    #[serde(rename = "Number")]
    pub number: Option<String>,
    #[serde(rename = "AllEmployeeNumbers")]
    pub all_numbers: Option<Vec<RestEmployeeNumber>>,
    #[serde(rename = "Presence")]
    pub presence: Option<RestEmployeePresence>,
    #[serde(rename = "Bitmap")]
    pub bitmap: Option<RestEmployeeBitmap>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestCallItem {
    #[serde(rename = "CallId")]
    pub call_id: i64,
    #[serde(rename = "CalledName")]
    pub called_name: Option<String>,
    #[serde(rename = "CalledNumber")]
    pub called_number: Option<String>,
    #[serde(rename = "CallingName")]
    pub calling_name: Option<String>,
    #[serde(rename = "CallingNumber")]
    pub calling_number: Option<String>,
    #[serde(rename = "ConnectedName")]
    pub connected_name: Option<String>,
    #[serde(rename = "ConnectedNumber")]
    pub connected_number: Option<String>,
    #[serde(rename = "Direction")]
    pub direction: i32,
    #[serde(rename = "StartTime")]
    pub start_time: Option<String>,
    #[serde(rename = "EndTime")]
    pub end_time: Option<String>,
    #[serde(rename = "State")]
    pub state: Option<i32>,
}

/// Client for communicating with the modern SwyxWare REST API (Port 9100)
#[derive(Debug, Clone)]
pub struct SwyxRestClient {
    pub host: String,
    pub port: u16,
    client: reqwest::Client,
}

impl SwyxRestClient {
    pub fn new(host: String, port: u16) -> Self {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(true) // Swyx typically uses self-signed internal certificates
            .timeout(std::time::Duration::from_secs(12))
            .build()
            .unwrap_or_else(|e| {
                tracing::warn!("SwyxRestClient builder error: {}", e);
                reqwest::Client::new()
            });

        Self { host, port, client }
    }

    fn base_url(&self) -> String {
        format!("https://{}:{}/ippbx/client/v1.0", self.host, self.port)
    }

    /// Performs modern REST login and returns AccessToken, RefreshToken, and UserId
    pub async fn login(&self, username: &str, password: &str) -> Result<AuthenticationResult> {
        let url = format!("{}/login/acquiretoken", self.base_url());
        debug!("Swyx REST Login to {}", url);

        let req = RestLoginRequest {
            username: username.to_string(),
            password: password.to_string(),
        };

        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("REST Login failed with HTTP {}: {}", status, err_text));
        }

        let auth: RestAuthResponse = resp.json().await?;
        info!(
            "REST Login successful! UserId: {}, AccessToken: {}...",
            auth.user_id,
            auth.access_token.chars().take(16).collect::<String>()
        );

        Ok(AuthenticationResult {
            access_token: auth.access_token,
            refresh_token: auth.refresh_token,
            user_id: auth.user_id,
            login_succeeded: true,
        })
    }

    /// Retrieves assigned phone numbers and extensions for the user
    pub async fn get_numbers(&self, user_id: i32, access_token: &str) -> Result<Vec<UserNumberInfo>> {
        let url = format!("{}/user/{}/numbers", self.base_url(), user_id);
        debug!("Fetching user numbers from {}", url);

        let resp = self
            .client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {}", access_token))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Fetching numbers failed with HTTP {}: {}", status, err_text));
        }

        let raw_numbers: Vec<RestNumberInfo> = resp.json().await?;
        let mut result = Vec::new();
        for r in raw_numbers {
            let pub_num = r.public_numbers.and_then(|pn| pn.into_iter().next());
            result.push(UserNumberInfo {
                user_id,
                internal_number: r.internal_number,
                public_number: pub_num,
                is_fax: false,
            });
        }

        Ok(result)
    }

    /// Resolves the SwyxServer SIP realm (e.g. `"SWYXPBX01"`) via `/login/tenantinfo`
    pub async fn get_sip_realm(&self) -> Option<String> {
        let url = format!("{}/login/tenantinfo", self.base_url());
        debug!("Fetching Swyx tenant info from {}", url);

        let resp = self.client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let info: RestTenantInfoResponse = resp.json().await.ok()?;
        let realm = info
            .tenant_domain
            .filter(|s| !s.trim().is_empty())
            .or_else(|| info.tenant_name.filter(|s| !s.trim().is_empty()))?;
        info!("Resolved Swyx SIP realm via REST tenantinfo: {}", realm);
        Some(realm)
    }

    /// Retrieves the logged-in user's profile (display name, primary own extension, SiteId)
    pub async fn get_user_profile(
        &self,
        user_id: i32,
        access_token: &str,
    ) -> Result<SwyxUserProfile> {
        let url = format!("{}/user/{}", self.base_url(), user_id);
        debug!("Fetching Swyx user profile from {}", url);

        let resp = self
            .client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {}", access_token))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Fetching user profile failed with HTTP {}: {}",
                status,
                err_text
            ));
        }

        let raw: RestUserProfileResponse = resp.json().await?;
        let nums = raw.numbers.unwrap_or_default();
        let primary = nums
            .iter()
            .find(|n| n.number_type == Some(0) && n.internal_number.as_deref().unwrap_or("").len() > 0)
            .or_else(|| nums.first());

        let own_extension = primary.and_then(|n| n.internal_number.clone());
        let display_name = raw
            .name
            .filter(|s| !s.trim().is_empty())
            .or_else(|| primary.and_then(|n| n.owner.clone()).filter(|s| !s.trim().is_empty()));
        let site_id = raw
            .tenant
            .and_then(|t| t.site_id)
            .filter(|&s| s > 0)
            .unwrap_or(1);

        Ok(SwyxUserProfile {
            user_id: raw.user_id,
            display_name,
            own_extension,
            site_id,
        })
    }

    /// Obtains the Pre-Shared Key (PSK) and SipRegisterId for native SIP registration
    pub async fn get_sip_register_token(
        &self,
        user_id: i32,
        access_token: &str,
    ) -> Result<RestSipRegisterTokenResponse> {
        let url = format!("{}/user/{}/sipregistertoken", self.base_url(), user_id);
        debug!("Requesting SIP Register Token from {}", url);

        let resp = self
            .client
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {}", access_token))
            .header(CONTENT_LENGTH, 0)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Requesting SIP Register Token failed with HTTP {}: {}", status, err_text));
        }

        let tok: RestSipRegisterTokenResponse = resp.json().await?;
        Ok(tok)
    }

    /// Retrieves company phonebook / directory
    pub async fn get_phonebook(&self, access_token: &str) -> Result<Vec<PhonebookEntry>> {
        let url = format!("{}/phonebook/?clientTimestamp=0", self.base_url());
        debug!("Fetching phonebook from {}", url);

        let resp = self
            .client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {}", access_token))
            .send()
            .await?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!("Fetching phonebook failed with HTTP 401 Unauthorized"));
        }
        if !resp.status().is_success() {
            return Ok(Vec::new());
        }

        let pb: Result<RestPhonebookResponse, _> = resp.json().await;
        let mut entries = Vec::new();
        if let Ok(pb) = pb {
            if let Some(list) = pb.global_phonebook_entries {
                for (idx, item) in list.into_iter().enumerate() {
                    let num = item.dial_number.or(item.display_number).unwrap_or_default();
                    if !num.is_empty() {
                        entries.push(PhonebookEntry {
                            id: Some(format!("pb-{}-{}", idx, num)),
                            name: item.name.unwrap_or_else(|| "Unbekannt".into()),
                            number: num.clone(),
                            extension: Some(num),
                            department: item.description,
                            presence_status: None,
                            avatar_file_id: None,
                        });
                    }
                }
            }
        }

        Ok(entries)
    }

    /// Retrieves all corporate employees / colleagues from the live Swyx directory
    pub async fn get_employees(&self, access_token: &str) -> Result<Vec<PhonebookEntry>> {
        let url = format!("{}/employees/?paginationIndex=0&paginationCount=500", self.base_url());
        debug!("Fetching employees directory from {}", url);

        let resp = self
            .client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {}", access_token))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Fetching employees failed with HTTP {}: {}", status, err_text));
        }

        let raw_employees: Vec<RestEmployee> = resp.json().await?;
        let mut entries = Vec::with_capacity(raw_employees.len());

        for emp in raw_employees {
            let name = emp.name.unwrap_or_else(|| "Unbekannt".into());
            let mut internal_ext = None;
            let mut public_num = None;

            if let Some(ref nums) = emp.all_numbers {
                for n in nums {
                    if n.number_type == 1 && internal_ext.is_none() {
                        internal_ext = Some(n.number.clone());
                    } else if n.number_type == 0 && public_num.is_none() {
                        public_num = Some(n.number.clone());
                    }
                }
            }

            // Fallback for internal extension
            if internal_ext.is_none() {
                if let Some(ref num) = emp.number {
                    if num.len() <= 4 && num.chars().all(|c| c.is_ascii_digit()) {
                        internal_ext = Some(num.clone());
                    }
                }
            }

            let number = public_num
                .or_else(|| emp.number.clone())
                .or_else(|| internal_ext.clone())
                .unwrap_or_default();

            let presence_status = emp.presence.as_ref().and_then(|p| {
                if let Some(ref txt) = p.status_text {
                    let lower = txt.to_lowercase();
                    if lower.contains("ringing") || lower.contains("klingel") || lower.contains("alerting") {
                        return Some(6);
                    }
                }
                p.status
            });

            let avatar_file_id = emp
                .bitmap
                .and_then(|b| b.file_id)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            entries.push(PhonebookEntry {
                id: Some(format!("swyx-{}", emp.employee_id)),
                name,
                number,
                extension: internal_ext,
                department: Some("Swyx Intern".into()),
                presence_status,
                avatar_file_id,
            });
        }

        info!("Fetched {} employee directory entries from Swyx REST", entries.len());
        Ok(entries)
    }

    /// Downloads a binary file (e.g. colleague avatar bitmap) by Swyx FileId (`/files/{fileId}`)
    pub async fn get_file(&self, file_id: &str, access_token: &str) -> Result<Option<Vec<u8>>> {
        let clean_id = file_id.trim();
        if clean_id.is_empty() {
            return Ok(None);
        }
        let url = format!("{}/files/{}", self.base_url(), clean_id);
        debug!("Downloading Swyx file from {}", url);

        let resp = self
            .client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {}", access_token))
            .send()
            .await?;

        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Fetching file {} failed with HTTP {}: {}", clean_id, status, err_text));
        }

        let bytes = resp.bytes().await?.to_vec();
        if bytes.is_empty() {
            Ok(None)
        } else {
            Ok(Some(bytes))
        }
    }

    /// Downloads the authenticated user's own avatar bitmap (`GET /user/{userId}/image`)
    pub async fn get_user_avatar(&self, user_id: i32, access_token: &str) -> Result<Option<Vec<u8>>> {
        let url = format!("{}/user/{}/image", self.base_url(), user_id);
        debug!("Downloading own Swyx avatar from {}", url);

        let resp = self
            .client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {}", access_token))
            .send()
            .await?;

        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Fetching own avatar failed with HTTP {}: {}", status, err_text));
        }

        let bytes = resp.bytes().await?.to_vec();
        if bytes.is_empty() {
            Ok(None)
        } else {
            Ok(Some(bytes))
        }
    }

    /// Uploads a new avatar image for the authenticated user (`PUT /user/{userId}/image`, raw image bytes)
    pub async fn upload_user_avatar(
        &self,
        user_id: i32,
        access_token: &str,
        image_bytes: Vec<u8>,
    ) -> Result<String> {
        let url = format!("{}/user/{}/image", self.base_url(), user_id);
        debug!("Uploading own Swyx avatar ({} bytes) to {}", image_bytes.len(), url);

        let resp = self
            .client
            .put(&url)
            .header(AUTHORIZATION, format!("Bearer {}", access_token))
            .header("Content-Type", "application/octet-stream")
            .body(image_bytes)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Uploading own avatar failed with HTTP {}: {}", status, err_text));
        }

        let raw = resp.text().await.unwrap_or_default();
        let file_id = serde_json::from_str::<String>(&raw)
            .unwrap_or_else(|_| raw.trim().trim_matches('"').to_string());
        info!("Uploaded own Swyx avatar for user {}, FileId={}", user_id, file_id);
        Ok(file_id)
    }

    /// Deletes a file from the Swyx file store (`DELETE /files/{fileId}`), used e.g. to remove own avatar
    pub async fn delete_file(&self, file_id: &str, access_token: &str) -> Result<bool> {
        let clean_id = file_id.trim();
        if clean_id.is_empty() {
            return Ok(true);
        }
        let url = format!("{}/files/{}", self.base_url(), clean_id);
        debug!("Deleting Swyx file at {}", url);

        let resp = self
            .client
            .delete(&url)
            .header(AUTHORIZATION, format!("Bearer {}", access_token))
            .send()
            .await?;

        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(true);
        }
        if !status.is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Deleting file {} failed with HTTP {}: {}", clean_id, status, err_text));
        }

        let raw = resp.text().await.unwrap_or_default();
        let ok = serde_json::from_str::<bool>(&raw).unwrap_or(true);
        info!("Deleted Swyx file {} (ok={})", clean_id, ok);
        Ok(ok)
    }

    /// Retrieves call history journal for the current user
    pub async fn get_calls(&self, user_id: i32, access_token: &str) -> Result<Vec<CallJournalEntry>> {
        let url = format!("{}/user/{}/calls", self.base_url(), user_id);
        debug!("Fetching calls from {}", url);

        let resp = self
            .client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {}", access_token))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Fetching call journal failed with HTTP {}: {}", status, err_text));
        }

        let raw_calls: Vec<RestCallItem> = resp.json().await?;
        let mut entries = Vec::with_capacity(raw_calls.len());

        for c in raw_calls {
            let direction = match c.direction {
                1 => CallDirection::Inbound,
                2 => CallDirection::Outbound,
                _ => CallDirection::Unknown,
            };
            let is_inbound = matches!(direction, CallDirection::Inbound);

            let caller_number = if is_inbound {
                c.calling_number.clone().unwrap_or_default()
            } else {
                c.called_number.clone().unwrap_or_default()
            };

            let caller_name = if is_inbound {
                c.calling_name.clone().or_else(|| c.connected_name.clone())
            } else {
                c.called_name.clone().or_else(|| c.connected_name.clone())
            };

            let called_number = if is_inbound {
                c.called_number.clone().unwrap_or_default()
            } else {
                c.calling_number.clone().unwrap_or_default()
            };

            let duration_seconds = match (parse_ms_date(&c.start_time), parse_ms_date(&c.end_time)) {
                (Some(s), Some(e)) if e > s => ((e - s) / 1000) as u32,
                _ => 0,
            };

            let was_connected = c.state == Some(0) || duration_seconds > 0;
            let timestamp = c.start_time.unwrap_or_default();

            entries.push(CallJournalEntry {
                id: c.call_id,
                caller_number,
                called_number,
                caller_name,
                timestamp,
                duration_seconds,
                direction,
                was_connected,
            });
        }

        info!("Fetched {} call journal entries from Swyx REST", entries.len());
        Ok(entries)
    }

    /// Updates the user's rich presence status on the Swyx server via REST API
    pub async fn set_presence(
        &self,
        user_id: i32,
        access_token: &str,
        status: PresenceStatus,
        status_text: &str,
    ) -> Result<()> {
        let url = format!("{}/user/{}/presence", self.base_url(), user_id);
        let swyx_status_code = match status {
            PresenceStatus::Available => 0,
            PresenceStatus::Away => 1,
            PresenceStatus::DoNotDisturb | PresenceStatus::Busy => 2,
            PresenceStatus::Offline => 4,
        };

        let payload = serde_json::json!({
            "Status": swyx_status_code,
            "StatusText": status_text,
        });

        debug!("Updating Swyx REST presence at {}: {:?}", url, payload);

        let resp = self
            .client
            .put(&url)
            .header(AUTHORIZATION, format!("Bearer {}", access_token))
            .json(&payload)
            .send()
            .await?;

        if !resp.status().is_success() {
            // Fallback to POST if PUT is not supported by this Swyx version
            let _ = self
                .client
                .post(&url)
                .header(AUTHORIZATION, format!("Bearer {}", access_token))
                .json(&payload)
                .send()
                .await;
        }

        info!(
            "Updated Swyx user presence for user {} to {:?} ('{}')",
            user_id, status, status_text
        );
        Ok(())
    }

    /// Updates unconditional call forwarding on the Swyx server via REST API
    pub async fn set_call_forwarding(
        &self,
        user_id: i32,
        access_token: &str,
        enabled: bool,
        target_number: &str,
    ) -> Result<()> {
        let url = format!("{}/user/{}/callforwarding", self.base_url(), user_id);
        let payload = serde_json::json!({
            "UnconditionalForwarding": enabled,
            "UnconditionalNumber": target_number,
            "CallForwardingUnconditional": enabled,
            "ForwardingNumber": target_number,
        });

        debug!("Updating Swyx REST call forwarding at {}: {:?}", url, payload);

        let resp = self
            .client
            .put(&url)
            .header(AUTHORIZATION, format!("Bearer {}", access_token))
            .json(&payload)
            .send()
            .await?;

        if !resp.status().is_success() {
            let _ = self
                .client
                .post(&url)
                .header(AUTHORIZATION, format!("Bearer {}", access_token))
                .json(&payload)
                .send()
                .await;
        }

        info!(
            "Updated Swyx call forwarding for user {}: enabled={}, target='{}'",
            user_id, enabled, target_number
        );
        Ok(())
    }

    /// Retrieves voicemail messages from the Swyx server via REST API
    pub async fn get_voicemails(
        &self,
        user_id: i32,
        access_token: &str,
    ) -> Result<Vec<VoiceMessageEntry>> {
        let url = format!("{}/user/{}/voicemails", self.base_url(), user_id);
        debug!("Fetching voicemails from {}", url);

        let resp = self
            .client
            .get(&url)
            .header(AUTHORIZATION, format!("Bearer {}", access_token))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(Vec::new());
        }

        let val: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        let arr = if let Some(a) = val.as_array() {
            a.clone()
        } else if let Some(a) = val.get("Voicemails").and_then(|v| v.as_array()) {
            a.clone()
        } else {
            return Ok(Vec::new());
        };

        let mut entries = Vec::with_capacity(arr.len());
        for item in arr {
            let id = item
                .get("VoicemailId")
                .or_else(|| item.get("Id"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let caller_number = item
                .get("CallerNumber")
                .or_else(|| item.get("CallingNumber"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let caller_name = item
                .get("CallerName")
                .or_else(|| item.get("CallingName"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let timestamp = item
                .get("StartTime")
                .or_else(|| item.get("Timestamp"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let duration_seconds = item
                .get("Duration")
                .or_else(|| item.get("DurationSeconds"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            let is_new = item
                .get("IsNew")
                .and_then(|v| v.as_bool())
                .or_else(|| item.get("IsRead").and_then(|v| v.as_bool()).map(|r| !r))
                .unwrap_or(false);

            entries.push(VoiceMessageEntry {
                id,
                caller_number,
                caller_name,
                timestamp,
                duration_seconds,
                is_new,
            });
        }

        Ok(entries)
    }
}

fn parse_ms_date(val: &Option<String>) -> Option<i64> {
    let s = val.as_ref()?;
    let stripped = s.trim_start_matches("/Date(").trim_end_matches(")/");
    let digits: String = stripped.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<i64>().ok()
}
