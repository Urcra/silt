//! How deltas are kept in databases. Each stored delta carries the format
//! it was written in, so a later Silt can change the encoding and still
//! read what is already stored.

use crate::Delta;

/// The format deltas are written in now: postcard of [`Delta`].
pub const DELTA_FORMAT: i64 = 0;

#[derive(Debug, thiserror::Error)]
pub enum StoredError {
    #[error("a stored delta is in format {0}, written by a newer Silt; upgrade to read it")]
    UnknownFormat(i64),
    #[error("a stored delta could not be decoded")]
    Decode(#[from] postcard::Error),
}

pub fn encode_stored(delta: &Delta) -> Result<Vec<u8>, postcard::Error> {
    postcard::to_allocvec(delta)
}

pub fn decode_stored(format: i64, bytes: &[u8]) -> Result<Delta, StoredError> {
    match format {
        DELTA_FORMAT => Ok(postcard::from_bytes(bytes)?),
        other => Err(StoredError::UnknownFormat(other)),
    }
}

#[cfg(test)]
mod tests {
    use silt_types::{ActorId, Frontier, ThreadId};

    use super::*;
    use crate::{Author, ConvOp, Family, Op};

    #[test]
    fn deltas_read_back_and_newer_formats_are_refused() {
        let family = Family::new();
        let delta = family.delta(ActorId(1), ThreadId(2), Author::User, 3, Frontier::root(), Op::Conv(ConvOp::Title("t".into())));
        let bytes = encode_stored(&delta).expect("encodes");
        assert_eq!(decode_stored(DELTA_FORMAT, &bytes).expect("decodes"), delta);
        assert!(matches!(decode_stored(DELTA_FORMAT + 1, &bytes), Err(StoredError::UnknownFormat(1))));
    }
}
