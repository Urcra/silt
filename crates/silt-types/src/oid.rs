use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A git object id (SHA-1).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GitOid(pub [u8; 20]);

impl fmt::Display for GitOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for GitOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GitOid({self})")
    }
}

impl FromStr for GitOid {
    type Err = crate::ParseIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        crate::hex::parse("git object id", s).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_hex() {
        let hex = "df6945893cde711c0fe5b23b6141047263697483";
        let oid: GitOid = hex.parse().expect("valid oid");
        assert_eq!(oid.to_string(), hex);
    }

    #[test]
    fn rejects_bad_input() {
        assert!("df69".parse::<GitOid>().is_err());
        assert!("zz6945893cde711c0fe5b23b6141047263697483".parse::<GitOid>().is_err());
    }
}
