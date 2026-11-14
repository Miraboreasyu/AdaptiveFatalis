//! HunterPie sequential MHWPartyMemberStructure: 9 pointer/long fields + 4 ints.
pub const STRIDE: usize = 0x58;
pub fn addresses(raw: &[u8]) -> Option<[u64; 4]> {
    if raw.len() != STRIDE * 4 {
        return None;
    }
    let mut result = [0; 4];
    for (i, r) in result.iter_mut().enumerate() {
        *r = u64::from_le_bytes(raw[i * STRIDE..i * STRIDE + 8].try_into().ok()?);
    }
    Some(result)
}
/// HunterPie uses character-name matching, not roster-address == entity.
/// Require a unique nonempty match; duplicated names are ambiguous.
pub fn local_slot(names: &[Option<Vec<u8>>; 4], local: Option<&[u8]>) -> Option<usize> {
    let local = local.filter(|s| !s.is_empty())?;
    let matches = names
        .iter()
        .enumerate()
        .filter(|(_, n)| n.as_deref() == Some(local))
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    (matches.len() == 1).then(|| matches[0])
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sequential_layout_and_empty_slots() {
        let mut b = vec![0; STRIDE * 4];
        for (i, p) in [123u64, 456, 789, 0].iter().enumerate() {
            b[i * STRIDE..i * STRIDE + 8].copy_from_slice(&p.to_le_bytes());
        }
        assert_eq!(addresses(&b), Some([123, 456, 789, 0]));
        assert_eq!(addresses(&b[..STRIDE]), None);
    }
    #[test]
    fn unique_name_not_pointer_identity() {
        let mut n = [Some(b"host".to_vec()), Some(b"me".to_vec()), None, None];
        assert_eq!(local_slot(&n, Some(b"me")), Some(1));
        n[2] = Some(b"me".to_vec());
        assert_eq!(local_slot(&n, Some(b"me")), None);
        assert_eq!(local_slot(&n, None), None);
    }
}
