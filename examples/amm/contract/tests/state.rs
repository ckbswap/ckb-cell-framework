use ckb_contract_core::{
    math::U256, token::PlainXudtAsset, BinaryCodec, Error, StateTransition, ZERO_HASH,
};
use framework_amm_script::{quote_swap, Action, State, MAX_RESERVE, MINIMUM_LIQUIDITY};

fn pool() -> State {
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
        1_000_000,
        4_000_000,
    )
    .unwrap()
}

#[test]
fn initialization_uses_geometric_mean_and_permanently_virtual_minimum_shares() {
    let before = pool();
    assert_eq!(before.total_shares, 2_000_000);
    assert_eq!(before.creation_settlement().unwrap().minted, 1_999_000);
    let after = before
        .apply(&Action::RemoveLiquidity {
            provider: [1; 32],
            shares: 1_999_000,
            min_x: 999_500,
            min_y: 3_998_000,
        })
        .unwrap();
    assert_eq!(
        (after.reserve_x, after.reserve_y, after.total_shares),
        (500, 2000, 1000)
    );
    assert_eq!(
        after.apply(&Action::RemoveLiquidity {
            provider: [1; 32],
            shares: 1,
            min_x: 0,
            min_y: 0,
        }),
        Err(Error::Transition)
    );
    // A drained circulating supply is still an existing pool, not a new genesis.
    assert!(after
        .apply(&Action::AddLiquidity {
            provider: [4; 32],
            shares: 2,
            max_x: 1,
            max_y: 4,
        })
        .is_ok());
    for size in [0, 1, MINIMUM_LIQUIDITY] {
        assert_eq!(
            State::new(before.creator, before.asset_x, before.asset_y, size, size),
            Err(Error::Transition)
        );
    }
}

#[test]
fn swap_fee_rounding_and_both_directions_follow_the_constant_product_quote() {
    let before = pool();
    assert_eq!(
        quote_swap(10_000, before.reserve_x, before.reserve_y),
        Ok(39_486)
    );
    let after = before
        .apply(&Action::SwapXForY {
            trader: [4; 32],
            amount_in: 10_000,
            min_out: 39_486,
        })
        .unwrap();
    assert_eq!((after.reserve_x, after.reserve_y), (1_010_000, 3_960_514));
    assert_eq!(after.total_shares, before.total_shares);
    assert_eq!(after.nonce, 1);
    assert!(
        U256::from(after.reserve_x) * U256::from(after.reserve_y)
            > U256::from(before.reserve_x) * U256::from(before.reserve_y)
    );
    let reverse = after
        .apply(&Action::SwapYForX {
            trader: [5; 32],
            amount_in: 40_000,
            min_out: 1,
        })
        .unwrap();
    assert!(reverse.reserve_x < after.reserve_x && reverse.reserve_y > after.reserve_y);
    assert_eq!(reverse.nonce, 2);
    assert_eq!(reverse.creator, before.creator);
    assert_eq!(
        (reverse.asset_x, reverse.asset_y),
        (before.asset_x, before.asset_y)
    );
}

#[test]
fn liquidity_rounding_charges_ceil_and_pays_floor() {
    let mut before = pool();
    before.reserve_x += 1;
    before.reserve_y += 1;
    let add = Action::AddLiquidity {
        provider: [4; 32],
        shares: 2,
        max_x: 2,
        max_y: 5,
    };
    let flow = before.settlement(&add).unwrap();
    assert_eq!((flow.spent_x, flow.spent_y, flow.minted), (2, 5, 2));
    let after = before.apply(&add).unwrap();
    let remove = Action::RemoveLiquidity {
        provider: [4; 32],
        shares: 2,
        min_x: 1,
        min_y: 4,
    };
    let flow = after.settlement(&remove).unwrap();
    assert_eq!((flow.received_x, flow.received_y, flow.burned), (1, 4, 2));
    let final_state = after.apply(&remove).unwrap();
    assert!(final_state.reserve_x >= before.reserve_x && final_state.reserve_y >= before.reserve_y);
}

#[test]
fn slippage_zero_amounts_and_invalid_actors_are_rejected() {
    let before = pool();
    for action in [
        Action::SwapXForY {
            trader: [4; 32],
            amount_in: 10_000,
            min_out: 39_487,
        },
        Action::SwapYForX {
            trader: [4; 32],
            amount_in: 10_000,
            min_out: 10_000,
        },
        Action::SwapXForY {
            trader: [4; 32],
            amount_in: 0,
            min_out: 0,
        },
        Action::SwapYForX {
            trader: [4; 32],
            amount_in: 1,
            min_out: 0,
        },
        Action::SwapXForY {
            trader: ZERO_HASH,
            amount_in: 10_000,
            min_out: 0,
        },
        Action::AddLiquidity {
            provider: [4; 32],
            shares: 2,
            max_x: 0,
            max_y: 4,
        },
        Action::AddLiquidity {
            provider: [4; 32],
            shares: 2,
            max_x: 1,
            max_y: 3,
        },
        Action::AddLiquidity {
            provider: ZERO_HASH,
            shares: 2,
            max_x: 1,
            max_y: 4,
        },
        Action::AddLiquidity {
            provider: [4; 32],
            shares: 0,
            max_x: 0,
            max_y: 0,
        },
        Action::RemoveLiquidity {
            provider: [4; 32],
            shares: 2,
            min_x: 2,
            min_y: 4,
        },
        Action::RemoveLiquidity {
            provider: [4; 32],
            shares: 2,
            min_x: 1,
            min_y: 5,
        },
        Action::RemoveLiquidity {
            provider: ZERO_HASH,
            shares: 2,
            min_x: 0,
            min_y: 0,
        },
        Action::RemoveLiquidity {
            provider: [4; 32],
            shares: 0,
            min_x: 0,
            min_y: 0,
        },
        Action::RemoveLiquidity {
            provider: [4; 32],
            shares: before.total_shares,
            min_x: 0,
            min_y: 0,
        },
    ] {
        assert_eq!(before.apply(&action), Err(Error::Transition), "{action:?}");
    }
}

#[test]
fn reserve_share_and_nonce_boundaries_fail_without_overflow() {
    let mut before = pool();
    before.reserve_x = MAX_RESERVE - 1;
    before.reserve_y = MAX_RESERVE;
    let action = Action::SwapXForY {
        trader: [4; 32],
        amount_in: 1,
        min_out: 0,
    };
    // At a 1:1 price a one-unit swap rounds to zero.
    assert_eq!(before.apply(&action), Err(Error::Transition));
    assert!(quote_swap(MAX_RESERVE - 1, 1, MAX_RESERVE).is_ok());
    for (input, x, y) in [
        (u128::MAX, 1, 1),
        (2, MAX_RESERVE - 1, MAX_RESERVE),
        (1, MAX_RESERVE + 1, 1),
        (1, 1, MAX_RESERVE + 1),
    ] {
        assert_eq!(quote_swap(input, x, y), Err(Error::Transition));
    }
    before = pool();
    before.total_shares = MAX_RESERVE;
    assert_eq!(
        before.apply(&Action::AddLiquidity {
            provider: [4; 32],
            shares: 1,
            max_x: u128::MAX,
            max_y: u128::MAX
        }),
        Err(Error::Transition)
    );
    before = pool();
    before.nonce = u64::MAX;
    assert_eq!(
        before.apply(&Action::SwapXForY {
            trader: [4; 32],
            amount_in: 10_000,
            min_out: 1
        }),
        Err(Error::Arithmetic)
    );
    before = pool();
    before.reserve_x = MAX_RESERVE;
    assert_eq!(
        before.apply(&Action::AddLiquidity {
            provider: [4; 32],
            shares: 2,
            max_x: u128::MAX,
            max_y: u128::MAX
        }),
        Err(Error::Transition)
    );
}

#[test]
fn quotes_preserve_product_across_varied_prices_and_inputs() {
    for x in [1, 1000, 1_000_000, MAX_RESERVE / 2] {
        for y in [1, 2000, 4_000_000, MAX_RESERVE] {
            for input in [1, 100, 10_000, MAX_RESERVE / 3] {
                if let Ok(out) = quote_swap(input, x, y) {
                    assert!(out > 0 && out < y);
                    assert!(
                        U256::from(x + input) * U256::from(y - out)
                            >= U256::from(x) * U256::from(y)
                    );
                    // Fee-adjusted invariant, including integer-rounding loss.
                    assert!(
                        (U256::from(x) * U256::from(10_000u128)
                            + U256::from(input) * U256::from(9970u128))
                            * U256::from(y - out)
                            >= U256::from(x) * U256::from(y) * U256::from(10_000u128)
                    );
                }
            }
        }
    }
}

#[test]
fn codecs_are_strict_and_cover_every_action_and_full_asset_identity() {
    let state = pool();
    assert_eq!(State::decode(&state.encode()), Ok(state.clone()));
    assert_eq!(&state.encode()[..5], b"AMM01");
    for action in [
        Action::AddLiquidity {
            provider: [4; 32],
            shares: 123,
            max_x: 12,
            max_y: 34,
        },
        Action::RemoveLiquidity {
            provider: [4; 32],
            shares: 123,
            min_x: 12,
            min_y: 34,
        },
        Action::SwapXForY {
            trader: [4; 32],
            amount_in: 123,
            min_out: 45,
        },
        Action::SwapYForX {
            trader: [4; 32],
            amount_in: 123,
            min_out: 45,
        },
    ] {
        let bytes = action.encode();
        assert_eq!(Action::decode(&bytes), Ok(action));
        for length in 0..bytes.len() {
            assert_eq!(Action::decode(&bytes[..length]), Err(Error::Encoding));
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert_eq!(Action::decode(&trailing), Err(Error::Encoding));
        let mut unknown = bytes;
        unknown[5] = 4;
        assert_eq!(Action::decode(&unknown), Err(Error::Encoding));
    }
    for length in 0..state.encode().len() {
        assert_eq!(
            State::decode(&state.encode()[..length]),
            Err(Error::Encoding)
        );
    }
    let mut bytes = state.encode();
    bytes.push(0);
    assert_eq!(State::decode(&bytes), Err(Error::Encoding));
    let mut bytes = state.encode();
    bytes[69] = 2; // asset_x.extended_args
    assert_eq!(State::decode(&bytes), Err(Error::Encoding));
    // Raw 32-byte args and 36-byte args with zero flags are separate assets.
    let mut other = state.asset_x;
    other.extended_args = true;
    assert!(State::new(state.creator, state.asset_x, other, 2000, 2000).is_ok());
    assert_eq!(
        State::new(state.creator, state.asset_x, state.asset_x, 2000, 2000),
        Err(Error::Transition)
    );
}
