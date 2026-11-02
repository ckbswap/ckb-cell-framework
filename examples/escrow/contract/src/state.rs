//! Fixed-amount, two-party escrow rules. This module computes states and settlements without syscalls.
//! lib.rs reads the transaction to verify authorization inputs, reserves and actual asset transfers.
//!
//! Funded --buyer Release--> Released, paying the seller.
//! Funded --seller Refund--> Refunded, paying the buyer.
//! Neither terminal state can settle again; buyer-authorized Close consumes the State and frees its CKB capacity.

use ckb_contract_core::{token::PlainXudtAsset, Error, Hash, Result, StateTransition, ZERO_HASH};

#[cfg(test)]
use ckb_contract_core::BinaryCodec;
use ckb_contract_derive::CellCodec;

#[derive(CellCodec, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The principal is still held in the Reserve.
    #[codec(tag = 0)]
    Funded,
    /// The seller has been paid; the Reserve token amount is zero.
    #[codec(tag = 1)]
    Released,
    /// The buyer has been refunded; the Reserve token amount is zero.
    #[codec(tag = 2)]
    Refunded,
}

/// Escrow terms and current status. Terms are immutable after creation; settlement changes only status.
#[derive(CellCodec, Clone, Debug, PartialEq, Eq)]
#[codec(prefix = b"ESC01")]
pub struct State {
    /// The buyer wallet's full Lock Hash, rather than a public key or code hash.
    pub buyer: Hash,
    /// The seller wallet's full Lock Hash; it must differ from the buyer's.
    pub seller: Hash,
    /// The plain xUDT owner parameter and its original args format.
    pub asset: PlainXudtAsset,
    /// The fixed principal in token base units, separate from CKB capacity.
    pub amount: u128,
    pub status: Status,
}

#[derive(CellCodec, Clone, Copy, Debug, PartialEq, Eq)]
#[codec(prefix = b"ESA01")]
pub enum Action {
    /// The buyer authorizes payment of the principal to the seller.
    #[codec(tag = 0)]
    Release,
    /// The seller authorizes refunding the principal to the buyer.
    #[codec(tag = 1)]
    Refund,
    /// The buyer closes a settled escrow through the 1->0 lifecycle.
    #[codec(tag = 2)]
    Close,
}

/// The expected settlement; this value alone does not prove authorization or payment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settlement {
    pub actor: Hash,
    pub recipient: Hash,
    pub amount: u128,
}

impl State {
    /// Construct the initial Funded State; on-chain validation separately proves that the principal was supplied.
    pub fn new(buyer: Hash, seller: Hash, asset: PlainXudtAsset, amount: u128) -> Result<Self> {
        let state = Self {
            buyer,
            seller,
            asset,
            amount,
            status: Status::Funded,
        };
        state.validate()?;
        Ok(state)
    }

    /// Require a nonzero principal and distinct, nonzero party identities.
    fn validate(&self) -> Result<()> {
        if self.amount == 0
            || self.buyer == self.seller
            || self.buyer == ZERO_HASH
            || self.seller == ZERO_HASH
        {
            return Err(Error::Transition);
        }
        Ok(())
    }

    /// Compute the required Reserve amount; the zero-balance Cell remains until close.
    pub fn reserve_amount(&self) -> u128 {
        if self.status == Status::Funded {
            self.amount
        } else {
            0
        }
    }

    /// Compute the actor and recipient for an Action, rejecting repeated settlement.
    /// Release requires the buyer, refund requires the seller; neither party can unilaterally pay themselves.
    pub fn settlement(&self, action: &Action) -> Result<Settlement> {
        self.validate()?;
        if self.status != Status::Funded {
            return Err(Error::Transition);
        }
        let (actor, recipient) = match action {
            Action::Release => (self.buyer, self.seller),
            Action::Refund => (self.seller, self.buyer),
            Action::Close => return Err(Error::Transition),
        };
        Ok(Settlement {
            actor,
            recipient,
            amount: self.amount,
        })
    }

    /// Check only terminal status; lib.rs separately verifies buyer authorization and the actual zero balance.
    pub fn require_close(&self) -> Result<()> {
        self.validate()?;
        if self.status == Status::Funded {
            return Err(Error::Transition);
        }
        Ok(())
    }
}

impl StateTransition for State {
    type Action = Action;

    fn apply(&self, action: &Action) -> Result<Self> {
        // Validate settlement eligibility, preserve the original terms and change only status; no assets move here.
        self.settlement(action)?;
        let mut after = self.clone();
        after.status = match action {
            Action::Release => Status::Released,
            Action::Refund => Status::Refunded,
            // Close produces no new State and is handled by the close branch in lib.rs.
            Action::Close => return Err(Error::Transition),
        };
        Ok(after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn funded() -> State {
        State::new(
            [1; 32],
            [2; 32],
            PlainXudtAsset {
                owner: [3; 32],
                extended_args: true,
            },
            100,
        )
        .unwrap()
    }

    #[test]
    fn settlement_uses_the_opposite_partys_authorization_and_preserves_terms() {
        let before = funded();
        for (action, status, actor, recipient) in [
            (
                Action::Release,
                Status::Released,
                before.buyer,
                before.seller,
            ),
            (
                Action::Refund,
                Status::Refunded,
                before.seller,
                before.buyer,
            ),
        ] {
            assert_eq!(
                before.settlement(&action),
                Ok(Settlement {
                    actor,
                    recipient,
                    amount: 100
                })
            );
            let after = before.apply(&action).unwrap();
            let mut expected = before.clone();
            expected.status = status;
            assert_eq!(after, expected);
            assert_eq!(after.reserve_amount(), 0);
            assert_eq!(after.require_close(), Ok(()));
            for repeat in [Action::Release, Action::Refund, Action::Close] {
                assert_eq!(after.apply(&repeat), Err(Error::Transition));
            }
        }
        assert_eq!(before.reserve_amount(), 100);
        assert_eq!(before.require_close(), Err(Error::Transition));
        assert_eq!(before.apply(&Action::Close), Err(Error::Transition));
    }

    #[test]
    fn rejects_invalid_terms_and_accepts_full_u128_amounts() {
        let state = funded();
        for (buyer, seller, amount) in [
            (state.buyer, state.seller, 0),
            (state.buyer, state.buyer, 1),
            (ZERO_HASH, state.seller, 1),
            (state.buyer, ZERO_HASH, 1),
        ] {
            assert_eq!(
                State::new(buyer, seller, state.asset, amount),
                Err(Error::Transition)
            );
        }
        let max = State::new(state.buyer, state.seller, state.asset, u128::MAX).unwrap();
        assert_eq!(max.settlement(&Action::Release).unwrap().amount, u128::MAX);
    }

    #[test]
    fn codecs_reject_truncation_trailing_bytes_invalid_tags_and_magic() {
        for extended_args in [false, true] {
            for status in [Status::Funded, Status::Released, Status::Refunded] {
                let mut state = funded();
                state.asset.extended_args = extended_args;
                state.status = status;
                let bytes = state.encode();
                assert_eq!(bytes.len(), 119);
                assert_eq!(State::decode(&bytes), Ok(state));
                for len in 0..bytes.len() {
                    assert_eq!(State::decode(&bytes[..len]), Err(Error::Encoding));
                }
                for (index, value) in [(0, 0), (101, 2), (118, 3)] {
                    let mut malformed = bytes.clone();
                    malformed[index] = value;
                    assert_eq!(State::decode(&malformed), Err(Error::Encoding));
                }
                let mut trailing = bytes;
                trailing.push(0);
                assert_eq!(State::decode(&trailing), Err(Error::Encoding));
            }
        }
        for action in [Action::Release, Action::Refund, Action::Close] {
            let bytes = action.encode();
            assert_eq!(Action::decode(&bytes), Ok(action));
            for len in 0..bytes.len() {
                assert_eq!(Action::decode(&bytes[..len]), Err(Error::Encoding));
            }
            let mut trailing = bytes;
            trailing.push(0);
            assert_eq!(Action::decode(&trailing), Err(Error::Encoding));
        }
        assert_eq!(Action::decode(b"ESA01\x03"), Err(Error::Encoding));
        assert_eq!(Action::decode(b"VLA01\x00"), Err(Error::Encoding));
    }
}
