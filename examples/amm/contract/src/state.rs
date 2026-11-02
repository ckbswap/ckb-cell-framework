//! Constant-product arithmetic, independent of CKB transaction loading.
use ckb_contract_core::{
    math::{mul_div_ceil, mul_div_floor, sqrt_product, U256},
    token::PlainXudtAsset,
    Error, Hash, Result, StateTransition, ZERO_HASH,
};
use ckb_contract_derive::CellCodec;

pub const FEE_BPS: u128 = 30;
pub const BASIS_POINTS: u128 = 10_000;
pub const MINIMUM_LIQUIDITY: u128 = 1000;
/// Bounds the three-factor swap numerator below 2^238 in U256.
pub const MAX_RESERVE: u128 = (1u128 << 112) - 1;

#[derive(CellCodec, Clone, Debug, PartialEq, Eq)]
#[codec(prefix = b"AMM01")]
pub struct State {
    /// Initial funder only; this field grants no administrative rights.
    pub creator: Hash,
    pub asset_x: PlainXudtAsset,
    pub asset_y: PlainXudtAsset,
    pub reserve_x: u128,
    pub reserve_y: u128,
    /// Accounting shares: initial sqrt + LP minted - LP redeemed through the pool.
    /// Includes MINIMUM_LIQUIDITY virtual shares. Standalone xUDT burns abandon
    /// redemption rights and do not reduce this denominator.
    pub total_shares: u128,
    pub nonce: u64,
}

#[derive(CellCodec, Clone, Copy, Debug, PartialEq, Eq)]
#[codec(prefix = b"AMA01")]
pub enum Action {
    #[codec(tag = 0)]
    AddLiquidity {
        provider: Hash,
        shares: u128,
        max_x: u128,
        max_y: u128,
    },
    #[codec(tag = 1)]
    RemoveLiquidity {
        provider: Hash,
        shares: u128,
        min_x: u128,
        min_y: u128,
    },
    #[codec(tag = 2)]
    SwapXForY {
        trader: Hash,
        amount_in: u128,
        min_out: u128,
    },
    #[codec(tag = 3)]
    SwapYForX {
        trader: Hash,
        amount_in: u128,
        min_out: u128,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settlement {
    pub actor: Hash,
    pub spent_x: u128,
    pub received_x: u128,
    pub spent_y: u128,
    pub received_y: u128,
    pub minted: u128,
    pub burned: u128,
}

impl State {
    pub fn new(
        creator: Hash,
        asset_x: PlainXudtAsset,
        asset_y: PlainXudtAsset,
        reserve_x: u128,
        reserve_y: u128,
    ) -> Result<Self> {
        let state = Self {
            creator,
            asset_x,
            asset_y,
            reserve_x,
            reserve_y,
            total_shares: sqrt_product(reserve_x, reserve_y),
            nonce: 0,
        };
        state.validate()?;
        if state.total_shares <= MINIMUM_LIQUIDITY {
            return Err(Error::Transition);
        }
        Ok(state)
    }

    pub fn validate(&self) -> Result<()> {
        if self.creator == ZERO_HASH
            || self.asset_x == self.asset_y
            || self.reserve_x == 0
            || self.reserve_y == 0
            || self.reserve_x > MAX_RESERVE
            || self.reserve_y > MAX_RESERVE
            || self.total_shares < MINIMUM_LIQUIDITY
            || self.total_shares > MAX_RESERVE
        {
            return Err(Error::Transition);
        }
        Ok(())
    }

    pub fn creation_settlement(&self) -> Result<Settlement> {
        if *self
            != Self::new(
                self.creator,
                self.asset_x,
                self.asset_y,
                self.reserve_x,
                self.reserve_y,
            )?
        {
            return Err(Error::Genesis);
        }
        Ok(Settlement {
            actor: self.creator,
            spent_x: self.reserve_x,
            received_x: 0,
            spent_y: self.reserve_y,
            received_y: 0,
            minted: self.total_shares - MINIMUM_LIQUIDITY,
            burned: 0,
        })
    }

    pub fn settlement(&self, action: &Action) -> Result<Settlement> {
        self.validate()?;
        let mut flow = Settlement {
            actor: ZERO_HASH,
            spent_x: 0,
            received_x: 0,
            spent_y: 0,
            received_y: 0,
            minted: 0,
            burned: 0,
        };
        match *action {
            Action::AddLiquidity {
                provider,
                shares,
                max_x,
                max_y,
            } => {
                if shares == 0 || shares > MAX_RESERVE - self.total_shares {
                    return Err(Error::Transition);
                }
                flow.actor = provider;
                flow.spent_x = mul_div_ceil(shares, self.reserve_x, self.total_shares)?;
                flow.spent_y = mul_div_ceil(shares, self.reserve_y, self.total_shares)?;
                if flow.spent_x > max_x || flow.spent_y > max_y {
                    return Err(Error::Transition);
                }
                flow.minted = shares;
            }
            Action::RemoveLiquidity {
                provider,
                shares,
                min_x,
                min_y,
            } => {
                if shares == 0 || shares > self.total_shares - MINIMUM_LIQUIDITY {
                    return Err(Error::Transition);
                }
                flow.actor = provider;
                flow.received_x = mul_div_floor(shares, self.reserve_x, self.total_shares)?;
                flow.received_y = mul_div_floor(shares, self.reserve_y, self.total_shares)?;
                if flow.received_x == 0
                    || flow.received_y == 0
                    || flow.received_x < min_x
                    || flow.received_y < min_y
                {
                    return Err(Error::Transition);
                }
                flow.burned = shares;
            }
            Action::SwapXForY {
                trader,
                amount_in,
                min_out,
            } => {
                flow.actor = trader;
                flow.spent_x = amount_in;
                flow.received_y = quote_swap(amount_in, self.reserve_x, self.reserve_y)?;
                if flow.received_y < min_out {
                    return Err(Error::Transition);
                }
            }
            Action::SwapYForX {
                trader,
                amount_in,
                min_out,
            } => {
                flow.actor = trader;
                flow.spent_y = amount_in;
                flow.received_x = quote_swap(amount_in, self.reserve_y, self.reserve_x)?;
                if flow.received_x < min_out {
                    return Err(Error::Transition);
                }
            }
        }
        if flow.actor == ZERO_HASH {
            return Err(Error::Transition);
        }
        Ok(flow)
    }
}

/// Exact-input swap; the full input enters the reserve, including the LP fee.
pub fn quote_swap(amount_in: u128, reserve_in: u128, reserve_out: u128) -> Result<u128> {
    if amount_in == 0
        || reserve_in == 0
        || reserve_out == 0
        || reserve_in > MAX_RESERVE
        || reserve_out > MAX_RESERVE
        || amount_in > MAX_RESERVE - reserve_in
    {
        return Err(Error::Transition);
    }
    let effective = U256::from(amount_in) * U256::from(BASIS_POINTS - FEE_BPS);
    let output = (effective * U256::from(reserve_out)
        / (U256::from(reserve_in) * U256::from(BASIS_POINTS) + effective))
        .low_u128();
    if output == 0 {
        return Err(Error::Transition);
    }
    Ok(output)
}

impl StateTransition for State {
    type Action = Action;

    fn apply(&self, action: &Action) -> Result<Self> {
        let flow = self.settlement(action)?;
        let mut after = self.clone();
        after.reserve_x = self
            .reserve_x
            .checked_add(flow.spent_x)
            .and_then(|x| x.checked_sub(flow.received_x))
            .ok_or(Error::Arithmetic)?;
        after.reserve_y = self
            .reserve_y
            .checked_add(flow.spent_y)
            .and_then(|y| y.checked_sub(flow.received_y))
            .ok_or(Error::Arithmetic)?;
        after.total_shares = self.total_shares + flow.minted - flow.burned;
        after.nonce = self.nonce.checked_add(1).ok_or(Error::Arithmetic)?;
        after.validate()?;
        Ok(after)
    }
}
