//! API keys: the env var first, then the key store of the OS (Windows Credential Manager, macOS Keychain).
//! Keys are never logged.

use crate::Provider;

/// The name of fterm in the key store.
pub const SERVICE: &str = "fterm";

/// The key for this provider, or `None`.
pub fn get(provider: &Provider) -> Option<String> {
    if let Some(key) = std::env::var(provider.key_env())
        .ok()
        .filter(|k| !k.trim().is_empty())
    {
        return Some(key.trim().to_owned());
    }
    keyring::Entry::new(SERVICE, &provider.name)
        .ok()?
        .get_password()
        .ok()
        .filter(|k| !k.trim().is_empty())
}

/// Saves the key in the key store (an empty key deletes it).
pub fn set(provider_name: &str, key: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE, provider_name).map_err(|err| err.to_string())?;
    if key.trim().is_empty() {
        return match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.to_string()),
        };
    }
    entry
        .set_password(key.trim())
        .map_err(|err| err.to_string())
}
