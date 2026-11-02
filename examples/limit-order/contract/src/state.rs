//! Fixed-price plain xUDT limit orders. The adapter verifies authorization,
//! reserve provenance and the actual net transfers of both assets.

use ckb_contract_core::{
    math::mul_div_ceil, token::PlainXudtAsset, Error, Hash, Result, StateTransition, ZERO_HASH,
};

#[cfg(test)]
use ckb_contract_core::BinaryCodec;
use ckb_contract_derive::CellCodec;

#[derive(CellCodec, Clone, Debug, PartialEq, Eq)]
#[codec(prefix = b"LOR01")]
pub struct State {
    pub maker: Hash,
    pub sell_asset: PlainXudtAsset,
    pub buy_asset: PlainXudtAsset,
    pub sell_total: u128,
    pub buy_total: u128,
    pub filled: u128,
    pub cancelled: bool,
}

#[derive(CellCodec, Clone, Copy, Debug, PartialEq, Eq)]
#[codec(prefix = b"LOA01")]
pub enum Action {
    #[codec(tag = 0)]
    Fill { taker: Hash, amount: u128 },
    #[codec(tag = 1)]
    Cancel,
    #[codec(tag = 2)]
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settlement {
    pub actor: Hash,
    pub sell_recipient: Hash,
    pub sell_amount: u128,
    pub buy_amount: u128,
}

impl State {
    pub fn new(
        maker: Hash,
        sell_asset: PlainXudtAsset,
        buy_asset: PlainXudtAsset,
        sell_total: u128,
        buy_total: u128,
    ) -> Result<Self> {
        let state = Self {
            maker,
            sell_asset,
            buy_asset,
            sell_total,
            buy_total,
            filled: 0,
            cancelled: false,
        };
        state.validate()?;
        Ok(state)
    }

    fn validate(&self) -> Result<()> {
        if self.maker == ZERO_HASH
            || self.sell_asset == self.buy_asset
            || self.sell_total == 0
            || self.buy_total == 0
            || self.filled > self.sell_total
            || (self.cancelled && self.filled == self.sell_total)
        {
            return Err(Error::Transition);
        }
        Ok(())
    }

    pub fn remaining(&self) -> Result<u128> {
        self.validate()?;
        Ok(self.sell_total - self.filled)
    }

    pub fn reserve_amount(&self) -> Result<u128> {
        let remaining = self.remaining()?;
        Ok(if self.cancelled { 0 } else { remaining })
    }

    /// Buy-asset units owed for this fill. Rounding applies independently to
    /// each fill: splitting cannot reduce payment, and positive fills cost >= 1.
    pub fn quote(&self, amount: u128) -> Result<u128> {
        if self.cancelled || amount == 0 || amount > self.remaining()? {
            return Err(Error::Transition);
        }
        mul_div_ceil(amount, self.buy_total, self.sell_total)
    }

    pub fn settlement(&self, action: &Action) -> Result<Settlement> {
        self.validate()?;
        match action {
            Action::Fill { taker, amount } => {
                if *taker == ZERO_HASH || *taker == self.maker {
                    return Err(Error::Transition);
                }
                Ok(Settlement {
                    actor: *taker,
                    sell_recipient: *taker,
                    sell_amount: *amount,
                    buy_amount: self.quote(*amount)?,
                })
            }
            Action::Cancel if !self.cancelled && self.filled < self.sell_total => Ok(Settlement {
                actor: self.maker,
                sell_recipient: self.maker,
                sell_amount: self.remaining()?,
                buy_amount: 0,
            }),
            _ => Err(Error::Transition),
        }
    }

    pub fn require_close(&self) -> Result<()> {
        self.validate()?;
        if !self.cancelled && self.filled != self.sell_total {
            return Err(Error::Transition);
        }
        Ok(())
    }
}

impl StateTransition for State {
    type Action = Action;

    fn apply(&self, action: &Action) -> Result<Self> {
        self.settlement(action)?;
        let mut after = self.clone();
        match action {
            Action::Fill { amount, .. } => after.filled += amount,
            Action::Cancel => after.cancelled = true,
            Action::Close => return Err(Error::Transition),
        }
        Ok(after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(sell_total: u128, buy_total: u128) -> State {
        State::new(
            [1; 32],
            PlainXudtAsset {
                owner: [2; 32],
                extended_args: false,
            },
            PlainXudtAsset {
                owner: [3; 32],
                extended_args: true,
            },
            sell_total,
            buy_total,
        )
        .unwrap()
    }

    #[test]
    fn partial_fills_cancel_and_close_preserve_original_terms() {
        let initial = order(100, 201);
        let action = Action::Fill {
            taker: [4; 32],
            amount: 30,
        };
        assert_eq!(
            initial.settlement(&action),
            Ok(Settlement {
                actor: [4; 32],
                sell_recipient: [4; 32],
                sell_amount: 30,
                buy_amount: 61,
            })
        );
        let partial = initial.apply(&action).unwrap();
        let mut expected = initial.clone();
        expected.filled = 30;
        assert_eq!(partial, expected);
        assert_eq!(partial.reserve_amount(), Ok(70));
        assert_eq!(partial.require_close(), Err(Error::Transition));
        assert_eq!(partial.settlement(&Action::Cancel).unwrap().sell_amount, 70);
        let cancelled = partial.apply(&Action::Cancel).unwrap();
        assert_eq!(cancelled.filled, 30);
        assert_eq!(cancelled.reserve_amount(), Ok(0));
        assert_eq!(cancelled.require_close(), Ok(()));
        let filled = partial
            .apply(&Action::Fill {
                taker: [5; 32],
                amount: 70,
            })
            .unwrap();
        assert_eq!(filled.reserve_amount(), Ok(0));
        assert_eq!(filled.require_close(), Ok(()));
        for terminal in [cancelled, filled] {
            for action in [action, Action::Cancel, Action::Close] {
                assert_eq!(terminal.apply(&action), Err(Error::Transition));
            }
        }
    }

    #[test]
    fn rounding_never_discounts_split_fills_or_allows_free_fills() {
        // Exhaustive small ratios and every two-way partition of a valid fill.
        for sell in 1..=24 {
            for buy in 1..=24 {
                let initial = order(sell, buy);
                for amount in 1..=sell {
                    let whole = initial.quote(amount).unwrap();
                    assert!(whole > 0);
                    assert!(whole * sell >= amount * buy);
                    assert!((whole - 1) * sell < amount * buy);
                    for first in 1..amount {
                        let partial = initial
                            .apply(&Action::Fill {
                                taker: [4; 32],
                                amount: first,
                            })
                            .unwrap();
                        assert!(
                            initial.quote(first).unwrap() + partial.quote(amount - first).unwrap()
                                >= whole
                        );
                    }
                }
            }
        }
        let low_price = order(3, 1);
        let after = low_price
            .apply(&Action::Fill {
                taker: [4; 32],
                amount: 1,
            })
            .unwrap();
        assert_eq!(after.quote(1), Ok(1));
        assert_eq!(order(u128::MAX, u128::MAX).quote(u128::MAX), Ok(u128::MAX));
        assert_eq!(order(u128::MAX, 1).quote(1), Ok(1));
        assert_eq!(order(1, u128::MAX).quote(1), Ok(u128::MAX));
    }

    #[test]
    fn invalid_terms_actors_amounts_and_terminal_states_are_rejected() {
        let initial = order(100, 200);
        for mutation in 0..7 {
            let mut invalid = initial.clone();
            match mutation {
                0 => invalid.maker = ZERO_HASH,
                1 => invalid.buy_asset = invalid.sell_asset,
                2 => invalid.sell_total = 0,
                3 => invalid.buy_total = 0,
                4 => invalid.filled = 101,
                5 => {
                    invalid.filled = 100;
                    invalid.cancelled = true;
                }
                _ => {
                    invalid.cancelled = true;
                }
            }
            assert_eq!(invalid.quote(1), Err(Error::Transition));
        }
        for (taker, amount) in [
            (ZERO_HASH, 1),
            (initial.maker, 1),
            ([4; 32], 0),
            ([4; 32], 101),
        ] {
            assert_eq!(
                initial.apply(&Action::Fill { taker, amount }),
                Err(Error::Transition)
            );
        }
        // Same owner with distinct raw args denotes two distinct asset Types.
        let mut raw_identity = initial.clone();
        raw_identity.buy_asset = raw_identity.sell_asset;
        raw_identity.buy_asset.extended_args = !raw_identity.sell_asset.extended_args;
        assert!(State::new(
            raw_identity.maker,
            raw_identity.sell_asset,
            raw_identity.buy_asset,
            100,
            200
        )
        .is_ok());
    }

    #[test]
    fn strict_codecs_preserve_both_assets_and_reject_invalid_tags_and_lengths() {
        for sell_extended in [false, true] {
            for buy_extended in [false, true] {
                let mut state = order(100, 201);
                state.sell_asset.extended_args = sell_extended;
                state.buy_asset.extended_args = buy_extended;
                let bytes = state.encode();
                assert_eq!(bytes.len(), 152);
                assert_eq!(State::decode(&bytes), Ok(state));
                for len in 0..bytes.len() {
                    assert_eq!(State::decode(&bytes[..len]), Err(Error::Encoding));
                }
                for index in [0, 69, 102, 151] {
                    let mut invalid = bytes.clone();
                    invalid[index] = 2;
                    assert_eq!(State::decode(&invalid), Err(Error::Encoding));
                }
                let mut trailing = bytes;
                trailing.push(0);
                assert_eq!(State::decode(&trailing), Err(Error::Encoding));
            }
        }
        for action in [
            Action::Fill {
                taker: [4; 32],
                amount: 30,
            },
            Action::Cancel,
            Action::Close,
        ] {
            let bytes = action.encode();
            assert_eq!(
                bytes.len(),
                if matches!(action, Action::Fill { .. }) {
                    54
                } else {
                    6
                }
            );
            assert_eq!(Action::decode(&bytes), Ok(action));
            for len in 0..bytes.len() {
                assert_eq!(Action::decode(&bytes[..len]), Err(Error::Encoding));
            }
            let mut invalid = bytes.clone();
            invalid[0] = 0;
            assert_eq!(Action::decode(&invalid), Err(Error::Encoding));
            invalid = bytes.clone();
            invalid[5] = 3;
            assert_eq!(Action::decode(&invalid), Err(Error::Encoding));
            let mut trailing = bytes;
            trailing.push(0);
            assert_eq!(Action::decode(&trailing), Err(Error::Encoding));
        }
    }
}
