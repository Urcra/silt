use serde::{Deserialize, Serialize};

/// Which permission requests the daemon answers for the person (ADR 0022).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// Edits, reads and searches go through; anything else asks.
    #[default]
    AskBeforeCommands,
    AskForEverything,
    /// Nothing asks.
    FullAccess,
    /// Every prompt asks for a plan first; reads and searches go through,
    /// and anything else asks (ADR 0044).
    Plan,
}

impl Preset {
    pub const ALL: [Self; 4] = [Self::AskForEverything, Self::Plan, Self::AskBeforeCommands, Self::FullAccess];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::AskBeforeCommands => "ask_before_commands",
            Self::AskForEverything => "ask_for_everything",
            Self::FullAccess => "full_access",
            Self::Plan => "plan",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.as_str() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::AskBeforeCommands => "Ask before commands",
            Self::AskForEverything => "Ask for everything",
            Self::FullAccess => "Full access",
            Self::Plan => "Plan",
        }
    }

    /// How much goes through without asking. Plan lets fewer tools through
    /// than Ask before commands, so a cap there allows it.
    fn latitude(self) -> u8 {
        match self {
            Self::AskForEverything => 0,
            Self::Plan => 1,
            Self::AskBeforeCommands => 2,
            Self::FullAccess => 3,
        }
    }

    /// Whether this preset stays within `cap`.
    pub fn within(self, cap: Self) -> bool {
        self.latitude() <= cap.latitude()
    }

    /// The stricter of two caps.
    pub fn stricter(self, other: Self) -> Self {
        if other.latitude() < self.latitude() { other } else { self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_order_presets_by_what_goes_through_without_asking() {
        use Preset::*;
        let cases = [
            (AskForEverything, AskForEverything, true),
            (Plan, AskForEverything, false),
            (Plan, AskBeforeCommands, true),
            (AskBeforeCommands, Plan, false),
            (FullAccess, AskBeforeCommands, false),
            (FullAccess, FullAccess, true),
        ];
        for (preset, cap, within) in cases {
            assert_eq!(preset.within(cap), within, "{preset:?} within {cap:?}");
        }
        assert_eq!(FullAccess.stricter(Plan), Plan);
        assert_eq!(AskForEverything.stricter(FullAccess), AskForEverything);
    }

    #[test]
    fn names_round_trip() {
        for preset in Preset::ALL {
            assert_eq!(Preset::parse(preset.as_str()), Some(preset));
            assert_eq!(serde_json::to_string(&preset).unwrap(), format!("\"{}\"", preset.as_str()));
        }
        assert_eq!(Preset::parse("yolo"), None);
    }
}
