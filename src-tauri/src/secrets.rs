//! API keys, one per provider, encrypted with Windows DPAPI for the current user. They never
//! go into settings.json, the log or the frontend - the settings window only learns whether a
//! key is set.

use std::collections::HashMap;
use std::path::Path;

pub fn load(path: &Path) -> HashMap<String, String> {
    let Ok(blob) = std::fs::read(path) else { return HashMap::new() };
    match unprotect(&blob) {
        Ok(plain) => serde_json::from_slice(&plain).unwrap_or_default(),
        Err(e) => {
            crate::diag::warn(format!("API keys unreadable ({e}) - they have to be entered again"));
            HashMap::new()
        }
    }
}

pub fn save(path: &Path, keys: &HashMap<String, String>) -> Result<(), String> {
    let plain = serde_json::to_vec(keys).map_err(|e| e.to_string())?;
    let blob = protect(&plain)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, blob).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(windows)]
pub(crate) fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Security::Cryptography::{CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut output = CRYPT_INTEGER_BLOB::default();
        CryptProtectData(&input, windows::core::w!("Amber"), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output)
            .map_err(|e| e.to_string())?;
        Ok(take_blob(output))
    }
}

#[cfg(windows)]
pub(crate) fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut output = CRYPT_INTEGER_BLOB::default();
        CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut output).map_err(|e| e.to_string())?;
        Ok(take_blob(output))
    }
}

/// Copies DPAPI's output and frees its buffer.
#[cfg(windows)]
unsafe fn take_blob(blob: windows::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB) -> Vec<u8> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    let out = std::slice::from_raw_parts(blob.pbData, blob.cbData as usize).to_vec();
    let _ = LocalFree(Some(HLOCAL(blob.pbData as *mut _)));
    out
}

// Elsewhere (only for `cargo check` off Windows) the keys are stored as they are.
#[cfg(not(windows))]
pub(crate) fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
    Ok(data.to_vec())
}

#[cfg(not(windows))]
pub(crate) fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    Ok(data.to_vec())
}
