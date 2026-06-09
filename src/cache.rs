use crate::types::{CachedLicense, CachedState, CachedTrial, LicenseStatus};
use crate::Error;
use chrono::Utc;
use std::path::{Path, PathBuf};

/// File-backed license/trial state cache.
///
/// Stores a JSON file at the provided path containing a [`CachedState`].
/// The app checks this cache on startup and when offline to determine
/// feature access without contacting the server.
#[derive(Clone)]
pub struct LicenseCache {
    path: PathBuf,
}

impl LicenseCache {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<CachedState, Error> {
        match std::fs::read_to_string(&self.path) {
            Ok(contents) => serde_json::from_str(&contents)
                .map_err(|e| Error::Cache(format!("corrupt cache file: {e}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CachedState::Unlicensed),
            Err(e) => Err(Error::Cache(format!("cannot read cache: {e}"))),
        }
    }

    pub fn save(&self, state: &CachedState) -> Result<(), Error> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Cache(format!("cannot create cache dir: {e}")))?;
        }
        let json = serde_json::to_string_pretty(state)
            .map_err(|e| Error::Cache(format!("cannot serialize cache: {e}")))?;
        std::fs::write(&self.path, json)
            .map_err(|e| Error::Cache(format!("cannot write cache: {e}")))?;
        Ok(())
    }

    pub fn clear(&self) -> Result<(), Error> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Cache(format!("cannot remove cache: {e}"))),
        }
    }

    /// Evaluate the cached state into a [`LicenseStatus`] the app can switch on.
    ///
    /// This is the offline decision function. It trusts `valid_until` and
    /// `expires_at` from the cache — those were set by the server on the
    /// last successful API call.
    pub fn status(&self) -> LicenseStatus {
        let state = match self.load() {
            Ok(s) => s,
            Err(_) => return LicenseStatus::Unlicensed,
        };

        let now = Utc::now();

        match state {
            CachedState::Licensed(lic) => {
                let valid_until = chrono::DateTime::parse_from_rfc3339(&lic.valid_until)
                    .map(|dt| dt.with_timezone(&Utc));

                match valid_until {
                    Ok(vt) if now <= vt => LicenseStatus::Licensed {
                        key_prefix: lic.key_prefix,
                        updates_until: lic.updates_until,
                    },
                    // Offline grace expired — need to re-validate.
                    // Treat as unlicensed rather than revoking outright;
                    // the app should attempt a validate call before locking.
                    _ => LicenseStatus::Unlicensed,
                }
            }
            CachedState::Trial(trial) => {
                let expires = chrono::DateTime::parse_from_rfc3339(&trial.expires_at)
                    .map(|dt| dt.with_timezone(&Utc));

                match expires {
                    Ok(exp) if now < exp => {
                        let remaining = (exp - now).num_hours().max(1) as u32;
                        LicenseStatus::Trial { hours_remaining: remaining }
                    }
                    _ => LicenseStatus::TrialExpired,
                }
            }
            CachedState::Unlicensed => LicenseStatus::Unlicensed,
        }
    }
}

/// Build a [`CachedLicense`] from a successful activate or validate response.
pub fn cached_license_from_info(info: &crate::types::LicenseInfo) -> Option<CachedLicense> {
    Some(CachedLicense {
        key_prefix: info.key_prefix.clone(),
        cohort: info.cohort.clone().unwrap_or_default(),
        updates_until: info.updates_until.clone()?,
        valid_until: info.valid_until.clone()?,
        max_activations: info.max_activations.unwrap_or(3),
        active_activations: info.active_activations.unwrap_or(1),
        last_validated_at: Utc::now().to_rfc3339(),
    })
}

/// Build a [`CachedTrial`] from a successful trial register response.
pub fn cached_trial_from_response(resp: &crate::types::TrialResponse) -> CachedTrial {
    CachedTrial {
        started_at: resp.started_at.clone(),
        expires_at: resp.expires_at.clone(),
        last_checked_at: Utc::now().to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn cache_in(dir: &TempDir) -> LicenseCache {
        LicenseCache::new(dir.path().join("license-cache.json"))
    }

    #[test]
    fn missing_cache_is_unlicensed() {
        let dir = TempDir::new().unwrap();
        let cache = cache_in(&dir);
        assert_eq!(cache.status(), LicenseStatus::Unlicensed);
    }

    #[test]
    fn roundtrip_licensed() {
        let dir = TempDir::new().unwrap();
        let cache = cache_in(&dir);

        let future = (Utc::now() + chrono::Duration::days(14)).to_rfc3339();
        let updates = (Utc::now() + chrono::Duration::days(365)).to_rfc3339();

        let state = CachedState::Licensed(CachedLicense {
            key_prefix: "K8G4Z".into(),
            cohort: "early_bird".into(),
            updates_until: updates.clone(),
            valid_until: future,
            max_activations: 3,
            active_activations: 1,
            last_validated_at: Utc::now().to_rfc3339(),
        });
        cache.save(&state).unwrap();

        match cache.status() {
            LicenseStatus::Licensed { key_prefix, updates_until } => {
                assert_eq!(key_prefix, "K8G4Z");
                assert_eq!(updates_until, updates);
            }
            other => panic!("expected Licensed, got {other:?}"),
        }
    }

    #[test]
    fn expired_valid_until_is_unlicensed() {
        let dir = TempDir::new().unwrap();
        let cache = cache_in(&dir);

        let past = (Utc::now() - chrono::Duration::days(1)).to_rfc3339();
        let state = CachedState::Licensed(CachedLicense {
            key_prefix: "K8G4Z".into(),
            cohort: "early_bird".into(),
            updates_until: "2027-01-01T00:00:00Z".into(),
            valid_until: past,
            max_activations: 3,
            active_activations: 1,
            last_validated_at: Utc::now().to_rfc3339(),
        });
        cache.save(&state).unwrap();

        assert_eq!(cache.status(), LicenseStatus::Unlicensed);
    }

    #[test]
    fn trial_active() {
        let dir = TempDir::new().unwrap();
        let cache = cache_in(&dir);

        let started = (Utc::now() - chrono::Duration::days(3)).to_rfc3339();
        let expires = (Utc::now() + chrono::Duration::days(4)).to_rfc3339();

        let state = CachedState::Trial(CachedTrial {
            started_at: started,
            expires_at: expires,
            last_checked_at: Utc::now().to_rfc3339(),
        });
        cache.save(&state).unwrap();

        match cache.status() {
            LicenseStatus::Trial { hours_remaining } => {
                // 4 days = 96 hours; 3 days ago start means ~96h remaining
                assert!(hours_remaining > 70 && hours_remaining <= 96,
                    "expected ~96h remaining, got {hours_remaining}");
            }
            other => panic!("expected Trial, got {other:?}"),
        }
    }

    #[test]
    fn trial_expired() {
        let dir = TempDir::new().unwrap();
        let cache = cache_in(&dir);

        let state = CachedState::Trial(CachedTrial {
            started_at: "2026-05-01T00:00:00Z".into(),
            expires_at: "2026-05-08T00:00:00Z".into(),
            last_checked_at: "2026-05-08T00:00:00Z".into(),
        });
        cache.save(&state).unwrap();
        assert_eq!(cache.status(), LicenseStatus::TrialExpired);
    }

    #[test]
    fn clear_removes_file() {
        let dir = TempDir::new().unwrap();
        let cache = cache_in(&dir);
        cache.save(&CachedState::Unlicensed).unwrap();
        assert!(cache.path().exists());
        cache.clear().unwrap();
        assert!(!cache.path().exists());
    }

    #[test]
    fn clear_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let cache = cache_in(&dir);
        cache.clear().unwrap(); // file doesn't exist yet — should not error
    }
}
