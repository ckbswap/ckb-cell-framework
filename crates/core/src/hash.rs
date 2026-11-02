use crate::Hash;
use blake2b_ref::Blake2bBuilder;

pub fn hash_parts(parts: &[&[u8]]) -> Hash {
    let mut out = [0; 32];
    let mut hasher = Blake2bBuilder::new(32)
        .personal(b"ckb-default-hash")
        .build();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize(&mut out);
    out
}

pub fn ckb_hash(data: &[u8]) -> Hash {
    hash_parts(&[data])
}

/// ID convention shared with Type ID: serialized CellInput (44 bytes) + output u64 LE.
/// This computes an ID; uniqueness also requires enforcing the creation lifecycle.
pub fn entity_id(first_input: &[u8; 44], output_index: u64) -> Hash {
    hash_parts(&[first_input, &output_index.to_le_bytes()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_depend_on_input_and_output_index() {
        let input = [7; 44];
        assert_ne!(entity_id(&input, 0), entity_id(&input, 1));
        assert_ne!(entity_id(&input, 0), entity_id(&[8; 44], 0));
        assert_eq!(hash_parts(&[b"a", b"bc"]), ckb_hash(b"abc"));
    }
}
