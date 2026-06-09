use serde::{Deserialize, Serialize};

// -- Requests -----------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ActivateRequest<'a> {
    pub key: &'a str,
    pub fingerprint: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_version: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<&'a str>,
}

#[derive(Debug, Serialize)]
pub struct ValidateRequest<'a> {
    pub key: &'a str,
    pub fingerprint: &'a str,
}

#[derive(Debug, Serialize)]
pub struct DeactivateRequest<'a> {
    pub key: &'a str,
    pub fingerprint: &'a str,
}

#[derive(Debug, Serialize)]
pub struct TrialRegisterRequest<'a> {
    pub fingerprint: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_version: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<&'a str>,
}

// -- Responses ----------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LicenseInfo {
    pub key_prefix: String,
    #[serde(default)]
    pub cohort: Option<String>,
    #[serde(default)]
    pub updates_until: Option<String>,
    #[serde(default)]
    pub valid_until: Option<String>,
    #[serde(default)]
    pub max_activations: Option<u32>,
    #[serde(default)]
    pub active_activations: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ActivateResponse {
    pub ok: bool,
    pub license: LicenseInfo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ValidateResponse {
    pub ok: bool,
    pub valid: bool,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub license: Option<LicenseInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeactivateResponse {
    pub ok: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrialResponse {
    pub ok: bool,
    pub started_at: String,
    pub expires_at: String,
    pub days_remaining: u32,
    pub fresh: bool,
    #[serde(default)]
    pub expired: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorEnvelope {
    pub error: ApiError,
    #[serde(default)]
    pub active_activations: Option<u32>,
    #[serde(default)]
    pub max_activations: Option<u32>,
}

// -- Cached state -------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedLicense {
    pub key_prefix: String,
    pub cohort: String,
    pub updates_until: String,
    pub valid_until: String,
    pub max_activations: u32,
    pub active_activations: u32,
    pub last_validated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedTrial {
    pub started_at: String,
    pub expires_at: String,
    pub last_checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum CachedState {
    #[serde(rename = "licensed")]
    Licensed(CachedLicense),
    #[serde(rename = "trial")]
    Trial(CachedTrial),
    #[serde(rename = "unlicensed")]
    Unlicensed,
}

/// High-level license state for the app to switch on.
#[derive(Debug, Clone, PartialEq)]
pub enum LicenseStatus {
    Licensed {
        key_prefix: String,
        updates_until: String,
    },
    Trial {
        hours_remaining: u32,
    },
    TrialExpired,
    Unlicensed,
}
