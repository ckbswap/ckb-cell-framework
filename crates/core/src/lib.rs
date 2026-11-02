//! Pure, no_std contract primitives. No syscalls or application-specific economics.
#![no_std]
extern crate alloc;

pub mod args;
pub mod codec;
pub mod hash;
pub mod layout;
pub mod math;
pub mod schema;
pub mod since;
pub mod token;
pub mod wire;

pub use codec::{BinaryCodec, Reader, Writer};
pub use hash::{ckb_hash, entity_id, hash_parts};

pub type Hash = [u8; 32];
pub const ZERO_HASH: Hash = [0; 32];
pub const CKB: u64 = 100_000_000;

/// Stable framework validation codes; business errors may wrap or extend them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i8)]
pub enum Error {
    Encoding = 5,
    Arithmetic = 6,
    Structure = 7,
    Authorization = 8,
    Capacity = 9,
    Token = 10,
    Genesis = 11,
    Transition = 12,
}

pub type Result<T> = core::result::Result<T, Error>;

/// Implement business transitions independently of transaction assembly.
/// Asset and capacity flows still need explicit verification by the contract.
pub trait StateTransition: BinaryCodec + PartialEq {
    type Action: BinaryCodec<Error = Self::Error>;

    fn apply(&self, action: &Self::Action) -> core::result::Result<Self, Self::Error>;
}

/// Exact balance equation: after + spent == before + received.
pub fn check_flow(before: u128, after: u128, spent: u128, received: u128) -> Result<()> {
    let left = after.checked_add(spent).ok_or(Error::Arithmetic)?;
    let right = before.checked_add(received).ok_or(Error::Arithmetic)?;
    if left != right {
        return Err(Error::Token);
    }
    Ok(())
}

/// Supply equation: outputs + burned == inputs + minted.
pub fn check_supply(inputs: u128, outputs: u128, minted: u128, burned: u128) -> Result<()> {
    check_flow(inputs, outputs, burned, minted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flows_reject_theft_extra_mint_and_overflow() {
        assert_eq!(check_flow(100, 80, 30, 10), Ok(()));
        assert_eq!(check_flow(100, 79, 30, 10), Err(Error::Token));
        assert_eq!(check_supply(100, 110, 10, 0), Ok(()));
        assert_eq!(check_supply(100, 111, 10, 0), Err(Error::Token));
        assert_eq!(check_supply(100, 90, 0, 10), Ok(()));
        assert_eq!(check_flow(0, u128::MAX, 1, 0), Err(Error::Arithmetic));
    }
}
