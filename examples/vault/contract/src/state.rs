//! Reusable fixed-principal, per-owner plain xUDT account protocol.
//!
//! No syscalls. The calling contract must verify
//! authorization, actual reserves, asset flows, creation and close lifecycles.

use ckb_contract_core::{token::PlainXudtAsset, Error, Hash, Result, StateTransition};

#[cfg(test)]
use ckb_contract_core::BinaryCodec;
use ckb_contract_derive::CellCodec;

#[derive(CellCodec, Clone, Debug, PartialEq, Eq)]
#[codec(prefix = b"VLT01")]
pub struct State {
    pub owner: Hash,
    pub asset: PlainXudtAsset,
    pub nonce: u64,
    pub balance: u128,
}

impl State {
    pub fn new(owner: Hash, asset: PlainXudtAsset) -> Self {
        Self {
            owner,
            asset,
            nonce: 0,
            balance: 0,
        }
    }

    /// Destruction has no successor State, so it is separate from `apply`.
    pub fn require_close(&self) -> Result<()> {
        if self.balance != 0 {
            return Err(Error::Transition);
        }
        Ok(())
    }
}

#[derive(CellCodec, Clone, Copy, Debug, PartialEq, Eq)]
#[codec(prefix = b"VLA01")]
pub enum Action {
    #[codec(tag = 0)]
    Deposit(u128),
    #[codec(tag = 1)]
    Withdraw(u128),
    #[codec(tag = 2)]
    Close,
}

impl StateTransition for State {
    type Action = Action;

    fn apply(&self, action: &Action) -> Result<Self> {
        let balance = match *action {
            Action::Deposit(0) | Action::Withdraw(0) | Action::Close => {
                return Err(Error::Transition);
            }
            Action::Deposit(amount) => self.balance.checked_add(amount).ok_or(Error::Arithmetic)?,
            Action::Withdraw(amount) => {
                self.balance.checked_sub(amount).ok_or(Error::Transition)?
            }
        };
        let mut after = self.clone();
        after.nonce = self.nonce.checked_add(1).ok_or(Error::Arithmetic)?;
        after.balance = balance;
        Ok(after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deposits_and_withdrawals_preserve_identity_and_principal() {
        let initial = account();
        let deposited = initial.apply(&Action::Deposit(100)).unwrap();
        assert_eq!(deposited.owner, initial.owner);
        assert_eq!(deposited.asset, initial.asset);
        assert_eq!(deposited.balance, 100);
        assert_eq!(deposited.nonce, 1);
        let partial = deposited.apply(&Action::Withdraw(40)).unwrap();
        assert_eq!(partial.balance, 60);
        assert_eq!(partial.nonce, 2);
        let empty = partial.apply(&Action::Withdraw(60)).unwrap();
        assert_eq!(empty.balance, 0);
        assert_eq!(empty.require_close(), Ok(()));
        assert_eq!(deposited.require_close(), Err(Error::Transition));
        assert_eq!(empty.apply(&Action::Deposit(7)).unwrap().balance, 7);
    }

    fn account() -> State {
        State::new(
            [1; 32],
            PlainXudtAsset {
                owner: [2; 32],
                extended_args: true,
            },
        )
    }

    #[test]
    fn rejects_zero_amounts_overdraft_and_arithmetic_overflow() {
        let state = account();
        for action in [
            Action::Deposit(0),
            Action::Withdraw(0),
            Action::Withdraw(1),
            Action::Close,
        ] {
            assert_eq!(state.apply(&action), Err(Error::Transition));
        }
        let mut max = state.clone();
        max.balance = u128::MAX;
        assert_eq!(max.apply(&Action::Deposit(1)), Err(Error::Arithmetic));
        assert_eq!(max.apply(&Action::Withdraw(u128::MAX)).unwrap().balance, 0);
        max = state;
        max.nonce = u64::MAX;
        assert_eq!(max.apply(&Action::Deposit(1)), Err(Error::Arithmetic));
    }

    #[test]
    fn codecs_reject_alternate_magic_and_trailing_data() {
        let state = account();
        assert_eq!(state.encode().len(), 94);
        assert_eq!(State::decode(&state.encode()), Ok(state.clone()));
        let mut bytes = state.encode();
        bytes[0] = 0;
        assert_eq!(State::decode(&bytes), Err(Error::Encoding));
        let mut bytes = state.encode();
        bytes[69] = 2;
        assert_eq!(State::decode(&bytes), Err(Error::Encoding));
        for len in 0..state.encode().len() {
            assert_eq!(State::decode(&state.encode()[..len]), Err(Error::Encoding));
        }
        let mut bytes = state.encode();
        bytes.push(0);
        assert_eq!(State::decode(&bytes), Err(Error::Encoding));
        for action in [
            Action::Deposit(u128::MAX),
            Action::Withdraw(1),
            Action::Close,
        ] {
            let bytes = action.encode();
            assert_eq!(Action::decode(&bytes), Ok(action));
            for len in 0..bytes.len() {
                assert_eq!(Action::decode(&bytes[..len]), Err(Error::Encoding));
            }
            let mut extra = bytes;
            extra.push(0);
            assert_eq!(Action::decode(&extra), Err(Error::Encoding));
        }
        assert_eq!(Action::decode(b"VLA01\x03"), Err(Error::Encoding));
    }
}
