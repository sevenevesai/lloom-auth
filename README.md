# License activation client for desktop apps

[<img alt="github" src="https://img.shields.io/badge/github-sevenevesai/lloom--auth-8da0cb?style=for-the-badge&labelColor=555555&logo=github" height="20">](https://github.com/sevenevesai/lloom-auth)
[<img alt="crates.io" src="https://img.shields.io/crates/v/lloom-auth.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/lloom-auth)

Async license-activation client for a Tauri desktop app: machine
fingerprinting, key activation / validation / deactivation, time-limited
trials, and a file-backed offline cache. Talks to a JSON API that
follows the activate / validate / deactivate / trial-register pattern
(e.g. a Next.js or Cloudflare Workers backend).

**Not a server.** This crate is the client half only. It assumes your
backend exposes `/api/licenses/activate`, `/api/licenses/validate`,
`/api/licenses/deactivate`, and `/api/trial/register`.

<br>

## Install

```toml
[dependencies]
lloom-auth = "0.1"
```

<br>

## Usage

```rust,no_run
use lloom_auth::{LicenseManager, LicenseStatus};

#[tokio::main]
async fn main() -> Result<(), lloom_auth::Error> {
    let mgr = LicenseManager::new(
        "https://lloom.app",       // license API base URL
        "~/.myapp/license.json",   // local cache path
        "1.0.0",                   // app version
    )?;

    // Check offline cache first (no network).
    match mgr.status() {
        LicenseStatus::Licensed { key_prefix, .. } => {
            println!("licensed ({key_prefix}...)");
        }
        LicenseStatus::Trial { days_remaining } => {
            println!("{days_remaining} trial days left");
        }
        LicenseStatus::TrialExpired => {
            println!("trial expired");
        }
        LicenseStatus::Unlicensed => {
            // First launch — register a trial.
            let status = mgr.register_trial().await?;
            println!("trial started: {status:?}");
        }
    }

    // Activate a license key.
    let status = mgr.activate("K8G4Z-ABCDE-12345-FGHIJ").await?;
    println!("activated: {status:?}");

    // Re-validate with the server (refreshes the offline cache).
    let status = mgr.validate("K8G4Z-ABCDE-12345-FGHIJ").await?;
    println!("validated: {status:?}");

    // Deactivate this machine.
    mgr.deactivate("K8G4Z-ABCDE-12345-FGHIJ").await?;

    Ok(())
}
```

<br>

## API

| Type / function | What it does |
|-----------------|--------------|
| `LicenseManager::new(url, cache_path, version)` | Build the manager. Generates the machine fingerprint, creates the HTTP client and cache. |
| `mgr.status()` | Evaluate the local cache into a `LicenseStatus` — no network. |
| `mgr.activate(key)` | Activate a license key on this machine. Caches the result. |
| `mgr.validate(key)` | Re-validate with the server. On network failure, falls back to the cache. |
| `mgr.revalidate()` | Background re-validation using the cached key — no-op within 24h of the last successful check. See "Offline behavior". |
| `mgr.deactivate(key)` | Deactivate this machine's activation and clear the cache. |
| `mgr.register_trial()` | Register or check a trial. Caches the trial info. |
| `mgr.clear_cache()` | Wipe the local cache (logout / key re-entry). |
| `mgr.fingerprint()` | The machine fingerprint string (SHA-256 hex). |
| `LicenseStatus` | Enum: `Licensed`, `Trial`, `TrialExpired`, `Unlicensed`. |
| `Error` | Enum: `Network`, `Api`, `Parse`, `Cache`, `Fingerprint`. |

<br>

## Machine fingerprint

On Windows, the fingerprint is `SHA-256(MachineGuid || volume_serial)`.
`MachineGuid` is read from the registry
(`HKLM\SOFTWARE\Microsoft\Cryptography`), and the volume serial comes
from `vol C:`. The result is a deterministic 64-character hex string
that stays stable across reboots but changes if the OS is reinstalled.

On macOS (since 0.2.0), the fingerprint is `SHA-256(IOPlatformUUID)`.
The platform UUID is read from
`ioreg -rd1 -c IOPlatformExpertDevice` — it identifies the logic board,
so it stays stable across reboots and OS reinstalls. Only the one-way
hash ever leaves the machine; the raw UUID is not transmitted or stored.

Other platforms are not yet supported — `generate()` returns
`Error::Fingerprint` on Linux.

<br>

## Offline behavior

The file-backed cache (`LicenseCache`) stores the last successful
server response as JSON. On startup or when offline:

- **Licensed:** trusted if `valid_until` (set by the server) has not
  passed. Once it expires, the status drops to `Unlicensed` and the
  app should attempt a `validate` call before locking features.
- **Trial:** trusted if `expires_at` has not passed. After that,
  `TrialExpired`.
- **Missing or corrupt cache:** `Unlicensed`.

`validate` and `register_trial` catch network errors and fall back to
the cached status, so the app degrades gracefully when offline.

### Background revalidation (`mgr.revalidate()`, 0.3+)

Call `revalidate()` on app start and on a periodic tick. It contacts the
server only when the cached license was last validated more than 24 hours
ago (`REVALIDATE_AFTER_HOURS`); a successful check slides `valid_until`
forward — a rolling offline grace window — and a server-side revocation
clears the cache so the app locks. Network failure falls back to the
cached status. A machine that stays offline past `valid_until` self-heals
on its next online revalidation without the user re-entering the key.

To make this possible, **the cleartext license key is stored in the local
cache file from 0.3 on** (`CachedLicense.key`). What this means for trust:
the key is the user's own credential, written to the user's own disk, with
the same file permissions as the rest of the app's data — nothing new
leaves the machine, and the only network calls remain the explicit
activate/validate/deactivate/trial endpoints, now also reachable via the
background `revalidate()` at most once per 24 hours. Caches written by
0.2.x lack the stored key; they keep the old offline-grace-only behavior
until the next manual activation rewrites the cache.

<br>

## Known limitations

- **Windows and macOS only** for fingerprinting. Linux support needs a
  platform identifier (`/etc/machine-id`) — add it when a Linux build
  actually ships.
- The offline cache trusts the clock. A user who sets their system time
  back can extend `valid_until` / `expires_at`. For a desktop app this
  is an acceptable tradeoff — clock manipulation is detectable on the
  next server validation.
- No built-in encryption of the cache file. If tamper resistance is
  required, encrypt the JSON blob before writing or use OS-level
  credential storage.

<br>

#### License

<sup>
Licensed under either of <a href="LICENSE-APACHE">Apache License, Version
2.0</a> or <a href="LICENSE-MIT">MIT license</a> at your option.
</sup>

<br>

<sub>
Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this crate by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
</sub>
