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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid git object id {0:?}: expected 40 hex digits")]
pub struct ParseGitOidError(String);

impl FromStr for GitOid {
    type Err = ParseGitOidError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let error = || ParseGitOidError(s.to_owned());
        if s.len() != 40 || !s.is_ascii() {
            return Err(error());
        }
        let mut bytes = [0u8; 20];
        let (pairs, _) = s.as_bytes().as_chunks::<2>();
        for (byte, pair) in bytes.iter_mut().zip(pairs) {
            let pair = std::str::from_utf8(pair).map_err(|_| error())?;
            *byte = u8::from_str_radix(pair, 16).map_err(|_| error())?;
        }
        Ok(Self(bytes))
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
