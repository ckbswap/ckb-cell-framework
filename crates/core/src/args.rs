use crate::{Error, Hash, Result};
use alloc::vec::Vec;

/// Framework convention: role:u8 || entity_id:bytes32 || application payload.
/// This is not a universal CKB argument format; applications opt in explicitly.
#[derive(Debug, PartialEq, Eq)]
pub struct RoleArgs<'a> {
    pub role: u8,
    pub id: Hash,
    pub payload: &'a [u8],
}

impl<'a> RoleArgs<'a> {
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        if data.len() < 33 {
            return Err(Error::Encoding);
        }
        Ok(Self {
            role: data[0],
            id: data[1..33].try_into().map_err(|_| Error::Encoding)?,
            payload: &data[33..],
        })
    }

    pub fn require_payload_len(&self, len: usize) -> Result<()> {
        if self.payload.len() != len {
            return Err(Error::Encoding);
        }
        Ok(())
    }
}

pub fn encode(role: u8, id: &Hash, payload: &[u8]) -> Vec<u8> {
    let mut args = Vec::with_capacity(33 + payload.len());
    args.push(role);
    args.extend_from_slice(id);
    args.extend_from_slice(payload);
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_lengths_are_explicit() {
        assert_eq!(RoleArgs::decode(&[0; 32]), Err(Error::Encoding));
        let bytes = encode(2, &[3; 32], &[4; 32]);
        let args = RoleArgs::decode(&bytes).unwrap();
        assert_eq!(args.role, 2);
        assert_eq!(args.id, [3; 32]);
        assert_eq!(args.require_payload_len(32), Ok(()));
        assert_eq!(args.require_payload_len(0), Err(Error::Encoding));
    }
}
