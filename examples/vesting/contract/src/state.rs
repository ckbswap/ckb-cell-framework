//! Irrevocable, fully funded linear vesting with an optional cliff.
//! Pure calculations use Unix seconds; the adapter must bind claim timestamps
//! to consensus-checked input since and verify authorization and asset flows.

#[cfg(test)]
use ckb_contract_core::BinaryCodec;
use ckb_contract_core::{
    math::mul_div_floor, since, token::PlainXudtAsset, Error, Hash, Result, StateTransition,
    ZERO_HASH,
};
use ckb_contract_derive::CellCodec;

#[derive(CellCodec, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Schedule {
    pub start: u64,
    pub cliff: u64,
    pub end: u64,
}

impl Schedule {
    pub fn validate(&self) -> Result<()> {
        if self.start >= self.end || self.cliff < self.start || self.cliff > self.end {
            return Err(Error::Transition);
        }
        since::absolute_timestamp(self.end)?;
        Ok(())
    }
}

#[derive(CellCodec, Clone, Debug, PartialEq, Eq)]
#[codec(prefix = b"VST01")]
pub struct State {
    pub funder: Hash,
    pub beneficiary: Hash,
    pub asset: PlainXudtAsset,
    pub total: u128,
    pub claimed: u128,
    pub schedule: Schedule,
}

#[derive(CellCodec, Clone, Copy, Debug, PartialEq, Eq)]
#[codec(prefix = b"VSA01")]
pub enum Action {
    #[codec(tag = 0)]
    Claim { timestamp: u64, amount: u128 },
    #[codec(tag = 1)]
    Close,
}

impl State {
    pub fn new(
        funder: Hash,
        beneficiary: Hash,
        asset: PlainXudtAsset,
        total: u128,
        schedule: Schedule,
    ) -> Result<Self> {
        let state = Self {
            funder,
            beneficiary,
            asset,
            total,
            claimed: 0,
            schedule,
        };
        state.validate()?;
        Ok(state)
    }

    fn validate(&self) -> Result<()> {
        if self.funder == ZERO_HASH
            || self.beneficiary == ZERO_HASH
            || self.total == 0
            || self.claimed > self.total
        {
            return Err(Error::Transition);
        }
        self.schedule.validate()
    }

    pub fn remaining(&self) -> Result<u128> {
        self.validate()?;
        self.total
            .checked_sub(self.claimed)
            .ok_or(Error::Arithmetic)
    }

    /// Before cliff: zero. At cliff: the amount accrued since start becomes
    /// available. At/after end: all tokens, including the rounding remainder.
    pub fn vested_at(&self, timestamp: u64) -> Result<u128> {
        self.validate()?;
        since::absolute_timestamp(timestamp)?;
        if timestamp < self.schedule.cliff {
            return Ok(0);
        }
        if timestamp >= self.schedule.end {
            return Ok(self.total);
        }
        mul_div_floor(
            self.total,
            u128::from(timestamp - self.schedule.start),
            u128::from(self.schedule.end - self.schedule.start),
        )
    }

    pub fn claimable_at(&self, timestamp: u64) -> Result<u128> {
        Ok(self.vested_at(timestamp)?.saturating_sub(self.claimed))
    }

    /// The funder closes on-chain only after all principal has been paid out.
    pub fn require_close(&self) -> Result<()> {
        if self.remaining()? != 0 {
            return Err(Error::Transition);
        }
        Ok(())
    }
}

impl StateTransition for State {
    type Action = Action;

    fn apply(&self, action: &Action) -> Result<Self> {
        let Action::Claim { timestamp, amount } = *action else {
            return Err(Error::Transition);
        };
        if amount == 0 || amount > self.claimable_at(timestamp)? {
            return Err(Error::Transition);
        }
        let mut after = self.clone();
        after.claimed = self.claimed.checked_add(amount).ok_or(Error::Arithmetic)?;
        Ok(after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(total: u128) -> State {
        State::new(
            [1; 32],
            [2; 32],
            PlainXudtAsset {
                owner: [3; 32],
                extended_args: true,
            },
            total,
            Schedule {
                start: 1000,
                cliff: 1200,
                end: 2000,
            },
        )
        .unwrap()
    }

    #[test]
    fn cliff_linear_rounding_and_end_release() {
        let state = grant(1001);
        for (timestamp, vested) in [
            (0, 0),
            (999, 0),
            (1000, 0),
            (1199, 0),
            (1200, 200),
            (1500, 500),
            (1999, 999),
            (2000, 1001),
            (3000, 1001),
        ] {
            assert_eq!(state.vested_at(timestamp), Ok(vested));
        }
        let max = grant(u128::MAX);
        assert_eq!(max.vested_at(1500), Ok(u128::MAX / 2));
        assert_eq!(max.vested_at(2000), Ok(u128::MAX));
    }

    #[test]
    fn partial_delayed_and_repeated_claims_preserve_terms() {
        let initial = grant(1000);
        let mut state = initial.clone();
        for (timestamp, amount, claimed) in [
            (1200, 100, 100),
            (1200, 100, 200),
            (1500, 300, 500),
            (2000, 500, 1000),
        ] {
            state = state.apply(&Action::Claim { timestamp, amount }).unwrap();
            let mut expected = initial.clone();
            expected.claimed = claimed;
            assert_eq!(state, expected);
        }
        assert_eq!(state.require_close(), Ok(()));
        assert_eq!(initial.require_close(), Err(Error::Transition));
        assert_eq!(
            state.apply(&Action::Claim {
                timestamp: 3000,
                amount: 1
            }),
            Err(Error::Transition)
        );
        assert_eq!(state.apply(&Action::Close), Err(Error::Transition));
        for (timestamp, amount) in [(1199, 1), (1200, 201), (2000, 1001), (2000, 0)] {
            assert_eq!(
                initial.apply(&Action::Claim { timestamp, amount }),
                Err(Error::Transition)
            );
        }
    }

    #[test]
    fn validates_schedules_and_supports_no_cliff_and_end_only_unlock() {
        for schedule in [
            Schedule {
                start: 2,
                cliff: 2,
                end: 2,
            },
            Schedule {
                start: 3,
                cliff: 3,
                end: 2,
            },
            Schedule {
                start: 1,
                cliff: 0,
                end: 2,
            },
            Schedule {
                start: 1,
                cliff: 3,
                end: 2,
            },
            Schedule {
                start: 1,
                cliff: 1,
                end: since::MAX_TIMESTAMP + 1,
            },
        ] {
            assert!(schedule.validate().is_err());
        }
        let mut state = grant(1000);
        state.schedule.cliff = state.schedule.start;
        assert_eq!(state.vested_at(1001), Ok(1));
        state.schedule.cliff = state.schedule.end;
        assert_eq!(state.vested_at(1999), Ok(0));
        assert_eq!(state.vested_at(2000), Ok(1000));
        state.beneficiary = state.funder;
        assert_eq!(
            state
                .apply(&Action::Claim {
                    timestamp: 2000,
                    amount: 1000
                })
                .unwrap()
                .remaining(),
            Ok(0)
        );
        for field in 0..4 {
            let mut invalid = grant(1000);
            match field {
                0 => invalid.funder = ZERO_HASH,
                1 => invalid.beneficiary = ZERO_HASH,
                2 => invalid.total = 0,
                _ => invalid.claimed = 1001,
            }
            assert_eq!(invalid.remaining(), Err(Error::Transition));
        }
        assert_eq!(
            state.vested_at(since::MAX_TIMESTAMP + 1),
            Err(Error::Encoding)
        );
    }

    #[test]
    fn strict_codecs_preserve_both_asset_formats_and_reject_bad_actions() {
        for extended in [false, true] {
            let mut state = grant(1000);
            state.asset.extended_args = extended;
            let bytes = state.encode();
            assert_eq!(bytes.len(), 158);
            assert_eq!(State::decode(&bytes), Ok(state));
            for len in 0..bytes.len() {
                assert_eq!(State::decode(&bytes[..len]), Err(Error::Encoding));
            }
            for (index, value) in [(0, 0), (101, 2)] {
                let mut wrong = bytes.clone();
                wrong[index] = value;
                assert_eq!(State::decode(&wrong), Err(Error::Encoding));
            }
            let mut trailing = bytes;
            trailing.push(0);
            assert_eq!(State::decode(&trailing), Err(Error::Encoding));
        }
        for action in [
            Action::Claim {
                timestamp: 2000,
                amount: u128::MAX,
            },
            Action::Close,
        ] {
            let bytes = action.encode();
            assert_eq!(Action::decode(&bytes), Ok(action));
            for len in 0..bytes.len() {
                assert_eq!(Action::decode(&bytes[..len]), Err(Error::Encoding));
            }
            let mut trailing = bytes;
            trailing.push(0);
            assert_eq!(Action::decode(&trailing), Err(Error::Encoding));
        }
        assert_eq!(Action::decode(b"VSA01\x02"), Err(Error::Encoding));
        assert_eq!(Action::decode(b"ESA01\x01"), Err(Error::Encoding));
    }
}
