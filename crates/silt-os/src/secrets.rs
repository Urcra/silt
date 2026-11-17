//! The person's secrets in their system's keyring: the Keychain on macOS,
//! the Credential Manager on Windows, a Secret Service on Linux. A machine
//! with none, as a server without a desktop, says so when one is asked for.

const SERVICE: &str = "silt";

fn text(error: &keyring::Error) -> String {
    format!("this computer's keyring: {error}")
}

/// Silt's secret named `name`, none when the keyring keeps none by it.
pub fn secret(name: &str) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(SERVICE, name).map_err(|error| text(&error))?;
    match entry.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(text(&error)),
    }
}

/// Keeps `value` as Silt's secret named `name`, or forgets it.
pub fn set_secret(name: &str, value: Option<&str>) -> Result<(), String> {
    let entry = keyring::Entry::new(SERVICE, name).map_err(|error| text(&error))?;
    match value {
        Some(value) => entry.set_password(value).map_err(|error| text(&error)),
        None => match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(text(&error)),
        },
    }
}
