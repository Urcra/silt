//! What this machine connects to on its own (ADR 0132): the switches in
//! `$SILT_HOME/network.json`, which the environment overrides. Offline, it
//! connects only to the servers it's signed in to; what the person asks
//! for, such as a model download, still goes ahead.

use std::path::Path;

use crate::core::CoreError;
use crate::protocol::{NetworkSettings, NetworkView};

const FILE: &str = "network.json";

/// The switches as saved; `Err` says why a file that's there can't be read.
fn saved(data_dir: &Path) -> Result<NetworkSettings, String> {
    match std::fs::read_to_string(data_dir.join(FILE)) {
        Ok(text) => serde_json::from_str(&text).map_err(|error| format!("{FILE} isn't valid: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(NetworkSettings::default()),
        Err(error) => Err(format!("couldn't read {FILE}: {error}")),
    }
}

pub(crate) fn save(data_dir: &Path, settings: NetworkSettings) -> Result<(), CoreError> {
    let json = serde_json::to_vec_pretty(&settings).map_err(|error| CoreError::Invalid(error.to_string()))?;
    crate::design::write_atomically(&data_dir.join(FILE), &json)
}

/// The switches with what the environment sets, for Settings › Network.
pub(crate) fn view(data_dir: &Path) -> Result<NetworkView, CoreError> {
    let settings = saved(data_dir).map_err(CoreError::Invalid)?;
    let offline_env = env_offline();
    Ok(NetworkView { settings, offline_env, no_update_checks_env: env_no_update_checks(), offline: offline_env || settings.offline })
}

/// What applies now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Allowed {
    pub(crate) offline: bool,
    pub(crate) update_checks: bool,
    pub(crate) plan_usage: bool,
    pub(crate) agent_registry: bool,
}

impl Allowed {
    /// Why plan usage isn't asked for, when it isn't.
    pub(crate) fn no_plan_usage(self) -> Option<&'static str> {
        match (self.offline, self.plan_usage) {
            (true, _) => Some("this machine is offline"),
            (false, false) => Some("plan usage is off in Settings › Network"),
            (false, true) => None,
        }
    }

    /// Where artifacts' libraries come from when this machine hasn't kept them.
    pub(crate) fn vendor_reach(self) -> silt_vendor::Reach {
        if self.offline { silt_vendor::Reach::KeptOnly } else { silt_vendor::Reach::Cdn }
    }
}

/// What this machine may connect to now. A file that can't be read counts
/// as offline, so a damaged file never turns a switch back on.
pub(crate) fn allowed(data_dir: &Path) -> Allowed {
    let (offline, settings) = match saved(data_dir) {
        Ok(settings) => (settings.offline, settings),
        Err(_) => (true, NetworkSettings::default()),
    };
    resolve(offline || env_offline(), settings, env_no_update_checks())
}

fn resolve(offline: bool, settings: NetworkSettings, no_update_checks: bool) -> Allowed {
    Allowed {
        offline,
        update_checks: !offline && settings.update_checks && !no_update_checks,
        plan_usage: !offline && settings.plan_usage,
        agent_registry: !offline && settings.agent_registry,
    }
}

fn env_offline() -> bool {
    std::env::var("SILT_OFFLINE").is_ok_and(|value| is_on(&value))
}

fn env_no_update_checks() -> bool {
    std::env::var("SILT_UPDATE_CHECK").is_ok_and(|value| value == "never")
}

fn is_on(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_one_and_true_turn_it_on() {
        for on in ["1", "true", "TRUE", " true "] {
            assert!(is_on(on), "{on:?}");
        }
        for off in ["", "0", "false", "no", "yes"] {
            assert!(!is_on(off), "{off:?}");
        }
    }

    #[test]
    fn offline_turns_every_switch_off_and_keeps_their_values() {
        let settings = NetworkSettings { offline: true, update_checks: true, plan_usage: false, agent_registry: true };
        let allowed = resolve(true, settings, false);
        assert_eq!(
            allowed,
            Allowed { offline: true, update_checks: false, plan_usage: false, agent_registry: false }
        );
        assert_eq!(allowed.no_plan_usage(), Some("this machine is offline"));
        assert_eq!(allowed.vendor_reach(), silt_vendor::Reach::KeptOnly);
        let online = resolve(false, settings, true);
        assert_eq!(online, Allowed { offline: false, update_checks: false, plan_usage: false, agent_registry: true });
        assert_eq!(online.no_plan_usage(), Some("plan usage is off in Settings › Network"));
    }

    #[test]
    fn a_missing_file_allows_everything_and_a_damaged_one_nothing() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert_eq!(saved(dir.path()), Ok(NetworkSettings::default()));
        save(dir.path(), NetworkSettings { plan_usage: false, ..NetworkSettings::default() }).expect("saved");
        assert!(!saved(dir.path()).expect("read").plan_usage);
        std::fs::write(dir.path().join(FILE), "{\"offline\": tru").expect("write");
        assert!(saved(dir.path()).is_err());
        assert!(allowed(dir.path()).offline || env_offline());
        assert!(view(dir.path()).is_err());
    }
}
