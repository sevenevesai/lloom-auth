//! License activation client for desktop apps.
//!
//! Async client for a JSON license API that follows the
//! activate / validate / deactivate / trial-register pattern, plus a
//! machine fingerprint (Windows + macOS) and a file-backed offline cache. Designed
//! for Tauri apps but has no GUI coupling — just `reqwest` + `tokio` +
//! `serde`.
//!
//! [`LicenseManager`] is the entry point. Construct one at startup with
//! the API base URL, a cache file path, and the app version; call
//! [`LicenseManager::status`] for the offline decision and
//! [`LicenseManager::activate`] / [`LicenseManager::validate`] /
//! [`LicenseManager::deactivate`] / [`LicenseManager::register_trial`]
//! to talk to the server.
//!
//! See the README for the full request/response contract and offline
//! behavior.

pub mod cache;
pub mod client;
pub mod fingerprint;
pub mod types;

use cache::{cached_license_from_info, cached_trial_from_response, LicenseCache};
use chrono::Utc;
use client::LicenseClient;
use types::{CachedState, LicenseStatus};

/// How long a successful validation stays fresh before
/// [`LicenseManager::revalidate`] contacts the server again.
pub const REVALIDATE_AFTER_HOURS: i64 = 24;

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

    #[cfg(test)]
    pub(crate) fn cache(&self) -> &LicenseCache {
        &self.cache
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

        if let Some(cached) = cached_license_from_info(&resp.license, key) {
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
                        if let Some(cached) = cached_license_from_info(info, key) {
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

    /// Background re-validation of the cached license.
    ///
    /// Safe to call on every app start / periodic tick: it only contacts the
    /// server when the cached license was last validated more than
    /// [`REVALIDATE_AFTER_HOURS`] ago. Outcomes:
    ///
    /// - Fresh cache (validated recently) → cached status, no network.
    /// - Stale + cached key → full [`Self::validate`]: success slides
    ///   `valid_until` forward (rolling offline grace); server-says-invalid
    ///   (e.g. revoked) clears the cache so the app locks; network failure
    ///   falls back to the cached status.
    /// - Decides on the raw cached *state*, not its offline evaluation — so a
    ///   machine that was offline past `valid_until` self-heals on the next
    ///   online call without the user re-entering the key.
    /// - No cached key (pre-0.3 cache), trial, or unlicensed → cached status.
    pub async fn revalidate(&self) -> Result<LicenseStatus, Error> {
        let state = match self.cache.load() {
            Ok(s) => s,
            Err(_) => return Ok(LicenseStatus::Unlicensed),
        };
        let CachedState::Licensed(lic) = state else {
            return Ok(self.cache.status());
        };
        let Some(key) = lic.key else {
            return Ok(self.cache.status());
        };
        if validation_is_fresh(&lic.last_validated_at, Utc::now()) {
            return Ok(self.cache.status());
        }
        self.validate(&key).await
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
                    // Derive hours from the cached expires_at for precision
                    // rather than converting the server's day-granularity value.
                    Ok(self.cache.status())
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

/// An unparseable timestamp counts as stale — revalidating too often is
/// harmless; never revalidating is the bug this function exists to prevent.
fn validation_is_fresh(last_validated_at: &str, now: chrono::DateTime<Utc>) -> bool {
    chrono::DateTime::parse_from_rfc3339(last_validated_at)
        .map(|dt| now - dt.with_timezone(&Utc) < chrono::Duration::hours(REVALIDATE_AFTER_HOURS))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::CachedLicense;

    fn fresh_ts() -> String {
        Utc::now().to_rfc3339()
    }

    fn stale_ts() -> String {
        (Utc::now() - chrono::Duration::hours(REVALIDATE_AFTER_HOURS + 1)).to_rfc3339()
    }

    fn cached_lic(key: Option<&str>, last_validated_at: String) -> CachedLicense {
        CachedLicense {
            key_prefix: "TESTK".into(),
            cohort: "standard".into(),
            updates_until: "2999-12-31T23:59:59Z".into(),
            valid_until: (Utc::now() + chrono::Duration::days(14)).to_rfc3339(),
            max_activations: 3,
            active_activations: 1,
            last_validated_at,
            key: key.map(String::from),
        }
    }

    #[test]
    fn freshness_window() {
        assert!(validation_is_fresh(&fresh_ts(), Utc::now()));
        assert!(!validation_is_fresh(&stale_ts(), Utc::now()));
        // Garbage timestamps are stale, not fresh — fail toward revalidating.
        assert!(!validation_is_fresh("not-a-date", Utc::now()));
        assert!(!validation_is_fresh("", Utc::now()));
    }

    #[test]
    fn old_cache_without_key_field_deserializes() {
        // A pre-0.3 cache file has no `key` member; it must load with
        // key = None instead of failing as corrupt.
        let json = r#"{
            "kind": "licensed",
            "key_prefix": "OLDKY",
            "cohort": "comp",
            "updates_until": "2999-12-31T23:59:59Z",
            "valid_until": "2026-06-19T00:00:00Z",
            "max_activations": 3,
            "active_activations": 1,
            "last_validated_at": "2026-06-05T00:00:00Z"
        }"#;
        let state: CachedState = serde_json::from_str(json).unwrap();
        match state {
            CachedState::Licensed(lic) => assert_eq!(lic.key, None),
            other => panic!("expected Licensed, got {other:?}"),
        }
    }

    #[test]
    fn key_survives_cache_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cache = LicenseCache::new(tmp.path().join("cache.json"));
        cache
            .save(&CachedState::Licensed(cached_lic(Some("LLOOM-AAAAA"), fresh_ts())))
            .unwrap();
        match cache.load().unwrap() {
            CachedState::Licensed(lic) => assert_eq!(lic.key.as_deref(), Some("LLOOM-AAAAA")),
            other => panic!("expected Licensed, got {other:?}"),
        }
    }

    // The revalidate tests construct a real LicenseManager, which generates a
    // machine fingerprint — only supported on Windows and macOS.
    #[cfg(any(windows, target_os = "macos"))]
    mod revalidate {
        use super::*;
        use std::io::{Read, Write};

        /// Minimal one-shot HTTP server: answers the first connection with
        /// the given JSON body and records that it was contacted.
        fn one_shot_server(body: &'static str) -> (String, std::sync::mpsc::Receiver<()>) {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                if let Ok((mut stream, _)) = listener.accept() {
                    let _ = tx.send(());
                    let mut buf = [0u8; 8192];
                    let _ = stream.read(&mut buf);
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                }
            });
            (format!("http://{addr}"), rx)
        }

        fn manager(api_url: &str, tmp: &tempfile::TempDir) -> LicenseManager {
            LicenseManager::new(api_url, tmp.path().join("cache.json"), "0.0.0-test").unwrap()
        }

        const REVOKED_BODY: &str = r#"{"ok":true,"valid":false,"reason":"KEY_REVOKED"}"#;
        const VALID_BODY: &str = r#"{"ok":true,"valid":true,"license":{"key_prefix":"TESTK","cohort":"standard","updates_until":"2999-12-31T23:59:59Z","valid_until":"2999-01-01T00:00:00Z","max_activations":3,"active_activations":1}}"#;

        #[tokio::test]
        async fn stale_and_revoked_clears_cache_and_locks() {
            let tmp = tempfile::TempDir::new().unwrap();
            let (url, _rx) = one_shot_server(REVOKED_BODY);
            let mgr = manager(&url, &tmp);
            mgr.cache()
                .save(&CachedState::Licensed(cached_lic(Some("LLOOM-AAAAA"), stale_ts())))
                .unwrap();

            let status = mgr.revalidate().await.unwrap();
            assert!(matches!(status, LicenseStatus::Unlicensed));
            // Cache cleared: a fresh status read is unlicensed too.
            assert!(matches!(mgr.status(), LicenseStatus::Unlicensed));
        }

        #[tokio::test]
        async fn stale_and_valid_slides_window_and_keeps_key() {
            let tmp = tempfile::TempDir::new().unwrap();
            let (url, _rx) = one_shot_server(VALID_BODY);
            let mgr = manager(&url, &tmp);
            let stale = stale_ts();
            mgr.cache()
                .save(&CachedState::Licensed(cached_lic(Some("LLOOM-AAAAA"), stale.clone())))
                .unwrap();

            let status = mgr.revalidate().await.unwrap();
            assert!(matches!(status, LicenseStatus::Licensed { .. }));
            match mgr.cache().load().unwrap() {
                CachedState::Licensed(lic) => {
                    // last_validated_at refreshed (window slid forward)…
                    assert_ne!(lic.last_validated_at, stale);
                    assert!(validation_is_fresh(&lic.last_validated_at, Utc::now()));
                    // …and the key survived the refresh (regression guard:
                    // a refresh that drops the key disables all future
                    // revalidation silently).
                    assert_eq!(lic.key.as_deref(), Some("LLOOM-AAAAA"));
                }
                other => panic!("expected Licensed, got {other:?}"),
            }
        }

        #[tokio::test]
        async fn fresh_cache_skips_network() {
            let tmp = tempfile::TempDir::new().unwrap();
            // Server would say REVOKED — if revalidate contacted it, status
            // would flip. Staying licensed proves the freshness short-circuit.
            let (url, rx) = one_shot_server(REVOKED_BODY);
            let mgr = manager(&url, &tmp);
            mgr.cache()
                .save(&CachedState::Licensed(cached_lic(Some("LLOOM-AAAAA"), fresh_ts())))
                .unwrap();

            let status = mgr.revalidate().await.unwrap();
            assert!(matches!(status, LicenseStatus::Licensed { .. }));
            assert!(
                rx.try_recv().is_err(),
                "fresh cache must not contact the server"
            );
        }

        #[tokio::test]
        async fn stale_without_key_returns_cached_status() {
            let tmp = tempfile::TempDir::new().unwrap();
            let (url, rx) = one_shot_server(REVOKED_BODY);
            let mgr = manager(&url, &tmp);
            mgr.cache()
                .save(&CachedState::Licensed(cached_lic(None, stale_ts())))
                .unwrap();

            // Pre-0.3 cache: no key to revalidate with — offline-grace
            // behavior only, server never contacted.
            let status = mgr.revalidate().await.unwrap();
            assert!(matches!(status, LicenseStatus::Licensed { .. }));
            assert!(rx.try_recv().is_err());
        }

        #[tokio::test]
        async fn network_failure_keeps_cached_status() {
            let tmp = tempfile::TempDir::new().unwrap();
            // Unroutable port: connection refused → Error::Network → grace.
            let mgr = manager("http://127.0.0.1:1", &tmp);
            mgr.cache()
                .save(&CachedState::Licensed(cached_lic(Some("LLOOM-AAAAA"), stale_ts())))
                .unwrap();

            let status = mgr.revalidate().await.unwrap();
            assert!(matches!(status, LicenseStatus::Licensed { .. }));
        }
    }
}
