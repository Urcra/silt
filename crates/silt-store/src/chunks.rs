//! Stored deltas packed into one blob: each one's format, then its length,
//! then its bytes.

use crate::StoreError;

/// Stored deltas a chunk holds; a family's last ones stay unpacked.
pub(crate) const CHUNK_ROWS: usize = 1000;

pub(crate) fn pack<'a>(rows: impl IntoIterator<Item = (i64, &'a [u8])>) -> Vec<u8> {
    let mut packed = Vec::new();
    for (format, data) in rows {
        packed.extend_from_slice(&format.to_le_bytes());
        packed.extend_from_slice(&(data.len() as u64).to_le_bytes());
        packed.extend_from_slice(data);
    }
    packed
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

    #[test]
    fn a_chunk_unpacks_to_what_was_packed() {
        let rows: Vec<(i64, Vec<u8>)> = vec![(0, vec![1, 2, 3]), (0, Vec::new()), (7, vec![255; 300])];
        let packed = pack(rows.iter().map(|(format, data)| (*format, data.as_slice())));
        let unpacked: Vec<(i64, Vec<u8>)> =
            unpack(&packed).expect("unpacks").into_iter().map(|(format, data)| (format, data.to_vec())).collect();
        assert_eq!(unpacked, rows);
        assert!(unpack(&packed[..packed.len() - 1]).is_err(), "a cut chunk is corrupt");
    }
}
