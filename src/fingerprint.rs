#[cfg(any(windows, target_os = "macos"))]
use sha2::{Digest, Sha256};

/// Generate a machine fingerprint.
///
/// - **Windows**: `sha256(MachineGuid || volume_serial)` — `MachineGuid` from
///   the registry, volume serial of the system drive via `vol C:`. Stable
///   across reboots; changes on OS reinstall.
/// - **macOS**: `sha256(IOPlatformUUID)` — the hardware platform UUID from
///   `ioreg -rd1 -c IOPlatformExpertDevice`. Tied to the logic board: stable
///   across reboots AND OS reinstalls.
/// - Other platforms: returns `Error::Fingerprint`.
pub fn generate() -> Result<String, crate::Error> {
    #[cfg(windows)]
    {
        windows_fingerprint()
    }
    #[cfg(target_os = "macos")]
    {
        macos_fingerprint()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        Err(crate::Error::Fingerprint(
            "fingerprint generation is only supported on Windows and macOS".into(),
        ))
    }
}

#[cfg(windows)]
fn windows_fingerprint() -> Result<String, crate::Error> {
    let machine_guid = read_machine_guid()?;
    let volume_serial = read_volume_serial()?;

    let mut hasher = Sha256::new();
    hasher.update(machine_guid.as_bytes());
    hasher.update(volume_serial.as_bytes());
    let hash = hasher.finalize();

    Ok(to_hex(&hash))
}

#[cfg(windows)]
fn read_machine_guid() -> Result<String, crate::Error> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm
        .open_subkey("SOFTWARE\\Microsoft\\Cryptography")
        .map_err(|e| crate::Error::Fingerprint(format!("cannot open Cryptography key: {e}")))?;
    let guid: String = key
        .get_value("MachineGuid")
        .map_err(|e| crate::Error::Fingerprint(format!("cannot read MachineGuid: {e}")))?;
    Ok(guid)
}

#[cfg(windows)]
fn read_volume_serial() -> Result<String, crate::Error> {
    use std::os::windows::process::CommandExt;

    let output = std::process::Command::new("cmd")
        .args(["/C", "vol", "C:"])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .map_err(|e| crate::Error::Fingerprint(format!("vol command failed: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(serial) = line
            .trim()
            .strip_prefix("Volume Serial Number is ")
        {
            return Ok(serial.trim().to_string());
        }
    }
    Err(crate::Error::Fingerprint(
        "could not parse volume serial from `vol C:` output".into(),
    ))
}

#[cfg(target_os = "macos")]
fn macos_fingerprint() -> Result<String, crate::Error> {
    // ioreg lives in /usr/sbin, NOT /usr/bin (0.2.0 shipped the wrong path and
    // failed on every Mac). Absolute path on purpose: no PATH lookup.
    let output = std::process::Command::new("/usr/sbin/ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .map_err(|e| crate::Error::Fingerprint(format!("ioreg command failed: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let uuid = parse_ioplatform_uuid(&stdout).ok_or_else(|| {
        crate::Error::Fingerprint("could not parse IOPlatformUUID from ioreg output".into())
    })?;

    let mut hasher = Sha256::new();
    hasher.update(uuid.as_bytes());
    Ok(to_hex(&hasher.finalize()))
}

/// Extract the `IOPlatformUUID` value from
/// `ioreg -rd1 -c IOPlatformExpertDevice` output, which contains a line like:
///
/// ```text
///       "IOPlatformUUID" = "AA0E2D4C-1B2D-5E6A-8F9C-0123456789AB"
/// ```
///
/// Compiled under `test` on all platforms so the parsing is covered by CI
/// that never runs on a Mac.
#[cfg(any(target_os = "macos", test))]
fn parse_ioplatform_uuid(ioreg_output: &str) -> Option<&str> {
    for line in ioreg_output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("\"IOPlatformUUID\"") {
            let value = rest.trim_start().strip_prefix('=')?.trim_start();
            let value = value.strip_prefix('"')?;
            let end = value.find('"')?;
            return Some(&value[..end]);
        }
    }
    None
}

#[cfg(any(windows, target_os = "macos", test))]
fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_encoding() {
        assert_eq!(to_hex(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
        assert_eq!(to_hex(&[]), "");
        assert_eq!(to_hex(&[0x00, 0xff]), "00ff");
    }

    #[test]
    fn parses_ioplatform_uuid_from_realistic_output() {
        let output = r#"+-o J293AP  <class IOPlatformExpertDevice, id 0x100000110, registered, matched, active, busy 0 (12 ms), retain 38>
    {
      "IOPlatformSerialNumber" = "C02XXXXXXXXX"
      "IOPlatformUUID" = "AA0E2D4C-1B2D-5E6A-8F9C-0123456789AB"
      "board-id" = <"Mac-XXXXXXXXXXXXXXXX">
    }
"#;
        assert_eq!(
            parse_ioplatform_uuid(output),
            Some("AA0E2D4C-1B2D-5E6A-8F9C-0123456789AB")
        );
    }

    #[test]
    fn ioplatform_uuid_missing_returns_none() {
        assert_eq!(parse_ioplatform_uuid(""), None);
        assert_eq!(
            parse_ioplatform_uuid("\"IOPlatformSerialNumber\" = \"C02X\""),
            None
        );
    }

    #[test]
    fn ioplatform_uuid_malformed_value_returns_none() {
        // Key present but value not a quoted string.
        assert_eq!(parse_ioplatform_uuid("\"IOPlatformUUID\" = 42"), None);
        assert_eq!(parse_ioplatform_uuid("\"IOPlatformUUID\""), None);
    }

    #[test]
    fn does_not_match_other_quoted_keys() {
        // strip_prefix leaves `Legacy" = ...`, which fails the `=` check →
        // None rather than a false match on a longer key name.
        let output = "\"IOPlatformUUIDLegacy\" = \"NOT-THIS-ONE\"";
        assert_eq!(parse_ioplatform_uuid(output), None);
    }
}

#[cfg(all(test, any(windows, target_os = "macos")))]
mod platform_tests {
    use super::*;

    #[test]
    fn fingerprint_is_64_hex() {
        let fp = generate().expect("fingerprint should succeed on this platform");
        assert_eq!(fp.len(), 64);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn fingerprint_is_deterministic() {
        let a = generate().unwrap();
        let b = generate().unwrap();
        assert_eq!(a, b);
    }
}
