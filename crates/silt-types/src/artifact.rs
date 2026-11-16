//! An artifact's id (ADR 0068): 12 hex characters, kept as 6 bytes so the
//! ops and comment targets that name one stay small and `Copy` (ADR 0072).

use std::fmt;
use std::str::FromStr;

use crate::hex::ParseIdError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactId(pub [u8; 6]);

impl fmt::Display for ArtifactId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for ArtifactId {
    type Err = ParseIdError;

    /// Exactly 12 lowercase hex characters, as the daemon makes them.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if !text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(ParseIdError::new("artifact id", 12, text));
        }
        crate::hex::parse::<6>("artifact id", text).map(Self)
    }
}

impl serde::Serialize for ArtifactId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() { serializer.collect_str(self) } else { self.0.serialize(serializer) }
    }
}

impl<'de> serde::Deserialize<'de> for ArtifactId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if deserializer.is_human_readable() {
            let text = String::deserialize(deserializer)?;
            text.parse().map_err(serde::de::Error::custom)
        } else {
            <[u8; 6]>::deserialize(deserializer).map(Self)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_read_and_print_as_twelve_hex_characters() {
        let id: ArtifactId = "0123456789ab".parse().expect("an id");
        assert_eq!(id.to_string(), "0123456789ab");
        assert_eq!(serde_json::to_string(&id).expect("json"), "\"0123456789ab\"");
        for bad in ["0123456789AB", "0123456789a", "0123456789abc", "../../etc/pw", ""] {
            assert!(bad.parse::<ArtifactId>().is_err(), "{bad}");
        }
    }
}
