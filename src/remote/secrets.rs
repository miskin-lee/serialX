//! The passwords and key passphrases saved hosts remember, kept in the
//! platform's own credential store: the Keychain on macOS, the Credential
//! Manager on Windows, the Secret Service (GNOME Keyring, KWallet) on
//! Linux. `remotes.json` only says that a host has one; the secret is
//! filed under the host's id and never written to a file.
//!
//! The workbench stores and forgets them. Reading one is the askpass
//! helper's job: `ssh` starts it when the host asks, so a secret leaves the
//! store only for a connection that needs it.

use keyring::{Entry, Error};

/// The service the entries are filed under.
const SERVICE: &str = "serialX SSH";

/// What the platform's store is called, for the dialog to name it.
pub(crate) const STORE_NAME: &str = if cfg!(target_os = "macos") {
    "the Keychain"
} else if cfg!(windows) {
    "Credential Manager"
} else {
    "the system keyring"
};

fn entry(id: u64) -> Result<Entry, String> {
    if cfg!(test) {
        // A test run must never reach the account's own store.
        return Err("no credential store under test".to_string());
    }
    Entry::new(SERVICE, &format!("host-{id}")).map_err(|error| error.to_string())
}

/// Keeps the secret for the host with `id`, over any kept before.
pub(crate) fn store(id: u64, secret: &str) -> Result<(), String> {
    entry(id)?
        .set_password(secret)
        .map_err(|error| error.to_string())
}

/// The secret kept for the host with `id`, if there is one.
pub(crate) fn load(id: u64) -> Result<Option<String>, String> {
    match entry(id)?.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(Error::NoEntry) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// Lets the secret for the host with `id` go; one never kept is no error.
pub(crate) fn forget(id: u64) -> Result<(), String> {
    match entry(id)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}
