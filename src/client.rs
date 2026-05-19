use crate::types::*;
use crate::Error;
use reqwest::Client;

/// HTTP client for the Lloom license API.
#[derive(Clone)]
pub struct LicenseClient {
    http: Client,
    base_url: String,
}

impl LicenseClient {
    pub fn new(base_url: &str) -> Result<Self, Error> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| Error::Network(format!("failed to build HTTP client: {e}")))?;

        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    pub async fn activate(
        &self,
        key: &str,
        fingerprint: &str,
        app_version: Option<&str>,
        platform: Option<&str>,
    ) -> Result<ActivateResponse, Error> {
        let body = ActivateRequest {
            key,
            fingerprint,
            app_version,
            platform,
        };

        let resp = self
            .http
            .post(format!("{}/api/licenses/activate", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Network(e.to_string()))?;

        if resp.status().is_success() {
            resp.json().await.map_err(|e| Error::Parse(e.to_string()))
        } else {
            let envelope: ErrorEnvelope = resp
                .json()
                .await
                .map_err(|e| Error::Parse(e.to_string()))?;
            Err(Error::Api {
                code: envelope.error.code,
                message: envelope.error.message,
            })
        }
    }

    pub async fn validate(
        &self,
        key: &str,
        fingerprint: &str,
    ) -> Result<ValidateResponse, Error> {
        let body = ValidateRequest { key, fingerprint };

        let resp = self
            .http
            .post(format!("{}/api/licenses/validate", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Network(e.to_string()))?;

        if resp.status().is_success() {
            resp.json().await.map_err(|e| Error::Parse(e.to_string()))
        } else {
            let envelope: ErrorEnvelope = resp
                .json()
                .await
                .map_err(|e| Error::Parse(e.to_string()))?;
            Err(Error::Api {
                code: envelope.error.code,
                message: envelope.error.message,
            })
        }
    }

    pub async fn deactivate(
        &self,
        key: &str,
        fingerprint: &str,
    ) -> Result<DeactivateResponse, Error> {
        let body = DeactivateRequest { key, fingerprint };

        let resp = self
            .http
            .post(format!("{}/api/licenses/deactivate", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Network(e.to_string()))?;

        if resp.status().is_success() {
            resp.json().await.map_err(|e| Error::Parse(e.to_string()))
        } else {
            let envelope: ErrorEnvelope = resp
                .json()
                .await
                .map_err(|e| Error::Parse(e.to_string()))?;
            Err(Error::Api {
                code: envelope.error.code,
                message: envelope.error.message,
            })
        }
    }

    pub async fn register_trial(
        &self,
        fingerprint: &str,
        app_version: Option<&str>,
        platform: Option<&str>,
    ) -> Result<TrialResponse, Error> {
        let body = TrialRegisterRequest {
            fingerprint,
            app_version,
            platform,
        };

        let resp = self
            .http
            .post(format!("{}/api/trial/register", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Network(e.to_string()))?;

        if resp.status().is_success() {
            resp.json().await.map_err(|e| Error::Parse(e.to_string()))
        } else {
            let envelope: ErrorEnvelope = resp
                .json()
                .await
                .map_err(|e| Error::Parse(e.to_string()))?;
            Err(Error::Api {
                code: envelope.error.code,
                message: envelope.error.message,
            })
        }
    }
}
