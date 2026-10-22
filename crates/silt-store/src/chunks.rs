//! Stored deltas packed into one blob: each one's format, then its length,
//! then its bytes; compressed by codec `ZSTD` (ADR 0035).

use crate::StoreError;

/// Stored deltas a chunk holds; a family's last ones stay unpacked.
pub(crate) const CHUNK_ROWS: usize = 1000;

/// The packing as it is, as chunks were stored before ADR 0035.
pub(crate) const PLAIN: i64 = 0;
pub(crate) const ZSTD: i64 = 1;
/// Fast, and still about seven times smaller on transcript lines.
const ZSTD_LEVEL: i32 = 1;

pub(crate) fn pack<'a>(rows: impl IntoIterator<Item = (i64, &'a [u8])>) -> Vec<u8> {
    let mut packed = Vec::new();
    for (format, data) in rows {
        packed.extend_from_slice(&format.to_le_bytes());
        packed.extend_from_slice(&(data.len() as u64).to_le_bytes());
        packed.extend_from_slice(data);
    }
    packed
}

/// `rows` packed and compressed, as a new chunk is stored.
pub(crate) fn compress<'a>(rows: impl IntoIterator<Item = (i64, &'a [u8])>) -> Result<Vec<u8>, StoreError> {
    zstd::bulk::compress(&pack(rows), ZSTD_LEVEL).map_err(StoreError::Chunk)
}

/// Chunks stored as `PLAIN` packings, compressed as a new chunk is, on
/// threads of their own.
pub(crate) fn compress_packed_each(packed: &[&[u8]]) -> Result<Vec<Vec<u8>>, StoreError> {
    std::thread::scope(|scope| {
        let workers: Vec<_> = packed
            .iter()
            .map(|packed| scope.spawn(|| zstd::bulk::compress(packed, ZSTD_LEVEL).map_err(StoreError::Chunk)))
            .collect();
        workers.into_iter().map(|worker| worker.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))).collect()
    })
}

/// Each chunk's rows compressed, on threads of their own when there are
/// several, as importing a long session completes many at once.
pub(crate) fn compress_each(chunks: &[[(i64, &[u8]); CHUNK_ROWS]]) -> Result<Vec<Vec<u8>>, StoreError> {
    if chunks.len() < 2 {
        return chunks.iter().map(|rows| compress(rows.iter().copied())).collect();
    }
    std::thread::scope(|scope| {
        let workers: Vec<_> = chunks.iter().map(|rows| scope.spawn(|| compress(rows.iter().copied()))).collect();
        workers.into_iter().map(|worker| worker.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))).collect()
    })
}

/// A stored chunk's packing, decompressed if `codec` says so.
pub(crate) fn decode(codec: i64, data: &[u8]) -> Result<std::borrow::Cow<'_, [u8]>, StoreError> {
    match codec {
        PLAIN => Ok(std::borrow::Cow::Borrowed(data)),
        ZSTD => zstd::stream::decode_all(data).map(std::borrow::Cow::Owned).map_err(StoreError::Chunk),
        codec => Err(StoreError::UnknownCodec(codec)),
    }
}

pub(crate) fn unpack(mut packed: &[u8]) -> Result<Vec<(i64, &[u8])>, StoreError> {
    let corrupt = || StoreError::Corrupt { what: "delta chunk" };
    let mut rows = Vec::new();
    while !packed.is_empty() {
        let (format, rest) = packed.split_first_chunk::<8>().ok_or_else(corrupt)?;
        let (len, rest) = rest.split_first_chunk::<8>().ok_or_else(corrupt)?;
        let len = usize::try_from(u64::from_le_bytes(*len)).map_err(|_| corrupt())?;
        if rest.len() < len {
            return Err(corrupt());
        }
        let (data, rest) = rest.split_at(len);
        rows.push((i64::from_le_bytes(*format), data));
        packed = rest;
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<(i64, Vec<u8>)> {
        vec![(0, vec![1, 2, 3]), (0, Vec::new()), (7, vec![255; 300])]
    }

    fn owned(unpacked: Vec<(i64, &[u8])>) -> Vec<(i64, Vec<u8>)> {
        unpacked.into_iter().map(|(format, data)| (format, data.to_vec())).collect()
    }

    #[test]
    fn a_chunk_unpacks_to_what_was_packed() {
        let rows = rows();
        let packed = pack(rows.iter().map(|(format, data)| (*format, data.as_slice())));
        assert_eq!(owned(unpack(&packed).expect("unpacks")), rows);
        assert!(unpack(&packed[..packed.len() - 1]).is_err(), "a cut chunk is corrupt");
    }

    #[test]
    fn a_compressed_chunk_decodes_to_what_was_packed() {
        let rows = rows();
        let compressed = compress(rows.iter().map(|(format, data)| (*format, data.as_slice()))).expect("compresses");
        let packed = pack(rows.iter().map(|(format, data)| (*format, data.as_slice())));
        assert!(compressed.len() < packed.len());
        assert_eq!(*decode(ZSTD, &compressed).expect("decompresses"), *packed);
        assert_eq!(*decode(PLAIN, &packed).expect("as it is"), *packed);
        assert!(decode(2, &packed).is_err(), "an unknown codec");
    }
}
