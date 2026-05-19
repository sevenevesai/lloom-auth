#[cfg(windows)]
use sha2::{Digest, Sha256};

/// Generate a machine fingerprint: `sha256(MachineGuid || volume_serial)`.
///
/// On Windows, reads `MachineGuid` from the registry and the volume serial
/// of the system drive via `vol C:`. On other platforms, returns an error.
pub fn generate() -> Result<String, crate::Error> {
    #[cfg(windows)]
    {
        windows_fingerprint()
    }
    #[cfg(not(windows))]
    {
        Err(crate::Error::Fingerprint(
            "fingerprint generation is only supported on Windows".into(),
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

#[cfg(windows)]
fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn hex_encoding() {
        assert_eq!(to_hex(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
        assert_eq!(to_hex(&[]), "");
        assert_eq!(to_hex(&[0x00, 0xff]), "00ff");
    }

    #[test]
    fn fingerprint_is_64_hex() {
        let fp = generate().expect("fingerprint should succeed on Windows");
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
