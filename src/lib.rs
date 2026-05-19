pub mod cache;
pub mod client;
pub mod fingerprint;
pub mod types;

use cache::{cached_license_from_info, cached_trial_from_response, LicenseCache};
use client::LicenseClient;
use types::{CachedState, LicenseStatus};

/// Crate-wide error type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Network(String),

    #[error("API error ({code}): {message}")]
    Api { code: String, message: String },

    #[error("parse error: {0}")]
    Parse(String),

    #[error("cache error: {0}")]
    Cache(String),

    #[error("fingerprint error: {0}")]
    Fingerprint(String),
}

/// High-level license manager combining client, cache, and fingerprint.
///
/// This is the entry point for the Tauri app. Create one at startup,
/// register it as managed state, and call its methods from commands.
/// Cheaply cloneable (all fields are `Clone`).
#[derive(Clone)]
pub struct LicenseManager {
    client: LicenseClient,
    cache: LicenseCache,
    fingerprint: String,
    app_version: String,
}

impl LicenseManager {
    /// Create a new `LicenseManager`.
    ///
    /// `api_url` is the base URL of the license API (e.g., `https://lloom.app`).
    /// `cache_path` is where the local cache file lives (e.g., app data dir).
    /// `app_version` is sent with activate/trial calls.
    pub fn new(
        api_url: &str,
        cache_path: impl Into<std::path::PathBuf>,
        app_version: &str,
    ) -> Result<Self, Error> {
        let client = LicenseClient::new(api_url)?;
        let cache = LicenseCache::new(cache_path);
        let fp = fingerprint::generate()?;

        Ok(Self {
            client,
            cache,
            fingerprint: fp,
            app_version: app_version.to_string(),
        })
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Current license status from the local cache (no network).
    pub fn status(&self) -> LicenseStatus {
        self.cache.status()
    }

    /// Activate a license key on this machine.
    ///
    /// On success, caches the license info and returns the status.
    /// On failure, returns the API error (INVALID_KEY, ACTIVATION_LIMIT, etc).
    pub async fn activate(&self, key: &str) -> Result<LicenseStatus, Error> {
        let resp = self
            .client
            .activate(
                key,
                &self.fingerprint,
                Some(&self.app_version),
                Some(platform()),
            )
            .await?;

        if let Some(cached) = cached_license_from_info(&resp.license) {
            self.cache
                .save(&CachedState::Licensed(cached.clone()))?;

            Ok(LicenseStatus::Licensed {
                key_prefix: cached.key_prefix,
                updates_until: cached.updates_until,
            })
        } else {
            Err(Error::Parse(
                "activate response missing required license fields".into(),
            ))
        }
    }

    /// Validate the current activation with the server.
    ///
    /// If the server says valid, refreshes the cache (extends `valid_until`).
    /// If the server says invalid, clears the license cache and returns the reason.
    /// On network error, falls back to the cached status.
    pub async fn validate(&self, key: &str) -> Result<LicenseStatus, Error> {
        match self.client.validate(key, &self.fingerprint).await {
            Ok(resp) => {
                if resp.valid {
                    if let Some(info) = &resp.license {
                        if let Some(cached) = cached_license_from_info(info) {
                            self.cache.save(&CachedState::Licensed(cached.clone()))?;
                            return Ok(LicenseStatus::Licensed {
                                key_prefix: cached.key_prefix,
                                updates_until: cached.updates_until,
                            });
                        }
                    }
                    Ok(self.cache.status())
                } else {
                    let reason = resp.reason.as_deref().unwrap_or("UNKNOWN");
                    tracing::info!(reason, "server says license is invalid");
                    self.cache.clear()?;
                    Ok(LicenseStatus::Unlicensed)
                }
            }
            Err(Error::Network(msg)) => {
                tracing::warn!(msg, "validate failed (network), using cached status");
                Ok(self.cache.status())
            }
            Err(e) => Err(e),
        }
    }

    /// Deactivate this machine's activation.
    pub async fn deactivate(&self, key: &str) -> Result<(), Error> {
        self.client.deactivate(key, &self.fingerprint).await?;
        self.cache.clear()?;
        Ok(())
    }

    /// Register or check a trial for this machine.
    ///
    /// Returns the trial status. Caches the trial info locally.
    pub async fn register_trial(&self) -> Result<LicenseStatus, Error> {
        match self
            .client
            .register_trial(
                &self.fingerprint,
                Some(&self.app_version),
                Some(platform()),
            )
            .await
        {
            Ok(resp) => {
                let cached = cached_trial_from_response(&resp);
                self.cache.save(&CachedState::Trial(cached))?;

                if resp.expired == Some(true) || resp.days_remaining == 0 {
                    Ok(LicenseStatus::TrialExpired)
                } else {
                    Ok(LicenseStatus::Trial {
                        days_remaining: resp.days_remaining,
                    })
                }
            }
            Err(Error::Network(msg)) => {
                tracing::warn!(msg, "trial register failed (network), using cached status");
                Ok(self.cache.status())
            }
            Err(e) => Err(e),
        }
    }

    /// Clear the local cache (for logout / key re-entry).
    pub fn clear_cache(&self) -> Result<(), Error> {
        self.cache.clear()
    }
}

fn platform() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}
