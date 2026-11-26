//! What this machine may connect to on its own (ADR 0132). With
//! `SILT_OFFLINE=1` it starts no connection of its own beyond the servers
//! it's signed in to; what the person asks for, such as a model download,
//! still goes ahead.

/// Whether `SILT_OFFLINE` is set to `1` or `true`.
pub fn offline() -> bool {
    std::env::var("SILT_OFFLINE").is_ok_and(|value| is_on(&value))
}

/// Whether the daemon checks for a newer Silt, and plugins' newer
/// versions, by itself: not offline, nor with `SILT_UPDATE_CHECK=never`.
pub(crate) fn checks_for_updates() -> bool {
    !offline() && !std::env::var("SILT_UPDATE_CHECK").is_ok_and(|value| value == "never")
}

/// Where artifacts' libraries come from when this machine hasn't kept them.
pub(crate) fn vendor_reach() -> silt_vendor::Reach {
    if offline() { silt_vendor::Reach::KeptOnly } else { silt_vendor::Reach::Cdn }
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
}
