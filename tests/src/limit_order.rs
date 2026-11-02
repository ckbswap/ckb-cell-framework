use ckb_contract_core::{
    args, ckb_hash, token::PlainXudtAsset, BinaryCodec, Error, StateTransition, CKB,
};
use ckb_contract_testkit::{
    amount, input, replace_data, replace_output, witnesses, workspace_root, Funding, LayoutConfig,
    StateReserveFixture,
};
use ckb_contract_tx::{edit::with_action, Cell, StateReserveTx};
use ckb_testtool::ckb_types::{
    bytes::Bytes,
    core::{ScriptHashType, TransactionBuilder, TransactionView},
    packed::{OutPoint, Script, WitnessArgs},
    prelude::*,
};
use framework_limit_order_script::{Action, State};

use framework_limit_order_script::{
    LOCK_ROLE, RESERVE_CAPACITY, RESERVE_ROLE, STATE_CAPACITY, STATE_ROLE,
};

struct Fixture {
    base: StateReserveFixture,
    maker: Script,
    taker: Script,
    other: Script,
    buy_asset: PlainXudtAsset,
    buy_type: Script,
}

impl Fixture {
    fn new(sell_extended: bool, buy_extended: bool) -> Self {
        let base = StateReserveFixture::new(
            &workspace_root(env!("CARGO_MANIFEST_DIR")),
            "framework-limit-order-script",
            PlainXudtAsset {
                owner: [9; 32],
                extended_args: sell_extended,
            },
            LayoutConfig {
                roles: [STATE_ROLE, LOCK_ROLE, RESERVE_ROLE],
                state_capacity: STATE_CAPACITY,
                reserve_capacity: RESERVE_CAPACITY,
            },
            Funding::Token {
                capacity: 10_000 * CKB,
                amount: 1000,
            },
        );
        let buy_asset = PlainXudtAsset {
            owner: [8; 32],
            extended_args: buy_extended,
        };
        let buy_type = base
            .layout
            .token_type
            .clone()
            .as_builder()
            .args(buy_asset.args().pack())
            .build();
        Self {
            maker: base.wallets[0].clone(),
            taker: base.wallets[1].clone(),
            other: base.wallets[2].clone(),
            base,
            buy_asset,
            buy_type,
        }
    }

    fn initial(&self) -> State {
        State::new(
            ckb_hash(self.maker.as_slice()),
            self.base.asset,
            self.buy_asset,
            100,
            201,
        )
        .unwrap()
    }

    fn creation(&self, state: &State) -> TransactionView {
        StateReserveTx::create(&self.base.layout, state, state.sell_total)
            .input(input(self.base.funding.clone()))
            .output(self.base.layout.token(
                &self.maker,
                10_000 * CKB - STATE_CAPACITY - RESERVE_CAPACITY - CKB,
                1000 - state.sell_total,
            ))
            .build()
            .unwrap()
    }

    fn pair(&mut self, state: &State) -> (OutPoint, OutPoint) {
        self.base.pair(state, state.reserve_amount().unwrap())
    }

    fn action(&self, actor: &Script, amount: u128) -> Action {
        Action::Fill {
            taker: ckb_hash(actor.as_slice()),
            amount,
        }
    }

    // Outputs: State, Reserve, taker sale proceeds, maker quote payment,
    // taker quote change. All capacities and fee are explicit.
    fn fill(
        &mut self,
        before: &State,
        amount: u128,
        actor: Script,
        index: usize,
    ) -> TransactionView {
        let action = self.action(&actor, amount);
        let after = before.apply(&action).unwrap();
        let payment = before.quote(amount).unwrap();
        let pair = self.pair(before);
        let funding = self
            .base
            .cell(Cell::token(&actor, &self.buy_type, 2000 * CKB, 1000));
        StateReserveTx::update(
            &self.base.layout,
            pair,
            &after,
            after.reserve_amount().unwrap(),
            &action,
        )
        .input(input(funding))
        .state_input_index(index)
        .output(self.base.layout.token(&actor, 200 * CKB, amount))
        .output(Cell::token(&self.maker, &self.buy_type, 200 * CKB, payment))
        .output(Cell::token(
            &actor,
            &self.buy_type,
            1599 * CKB,
            1000 - payment,
        ))
        .build()
        .unwrap()
    }

    fn cancel(&mut self, before: &State, actor: Script, index: usize) -> TransactionView {
        let after = before.apply(&Action::Cancel).unwrap();
        let pair = self.pair(before);
        let funding = self.base.cell(Cell::capacity(&actor, 1000 * CKB));
        StateReserveTx::update(&self.base.layout, pair, &after, 0, &Action::Cancel)
            .input(input(funding))
            .state_input_index(index)
            .output(
                self.base
                    .layout
                    .token(&self.maker, 200 * CKB, before.remaining().unwrap()),
            )
            .output(Cell::capacity(&actor, 799 * CKB))
            .build()
            .unwrap()
    }

    fn close(&mut self, before: &State, actor: Script, index: usize) -> TransactionView {
        let pair = self.pair(before);
        let funding = self.base.cell(Cell::capacity(&actor, 1000 * CKB));
        StateReserveTx::close(pair, &Action::Close)
            .input(input(funding))
            .state_input_index(index)
            .output(Cell::capacity(
                &self.maker,
                STATE_CAPACITY + RESERVE_CAPACITY,
            ))
            .output(Cell::capacity(&actor, 999 * CKB))
            .build()
            .unwrap()
    }

    fn assert_valid(&mut self, tx: TransactionView) -> TransactionView {
        self.base.assert_valid(tx)
    }

    fn assert_error(&mut self, tx: TransactionView, error: Error) {
        self.base.assert_error(tx, error)
    }
}

#[test]
fn partial_fills_by_different_takers_and_close_use_actual_predecessors() {
    for sell_extended in [false, true] {
        for buy_extended in [false, true] {
            let mut f = Fixture::new(sell_extended, buy_extended);
            let mut state = f.initial();
            f.base.commit(f.creation(&state));
            let mut paid = 0;
            for (position, quantity) in [(0, 30), (1, 20), (0, 50)] {
                let actor = if position == 0 {
                    f.taker.clone()
                } else {
                    f.other.clone()
                };
                paid += state.quote(quantity).unwrap();
                let tx = f.fill(&state, quantity, actor.clone(), position);
                state = state.apply(&f.action(&actor, quantity)).unwrap();
                f.base.commit(tx);
                let pair = f.base.pair.as_ref().unwrap();
                assert!(f.base.context.get_cell(&pair.0).is_some());
            }
            assert_eq!(paid, 203); // Per-fill ceilings: 61 + 41 + 101.
            assert_eq!(state.filled, 100);
            let points = f.base.pair.clone().unwrap();
            let tx = f.close(&state, f.maker.clone(), 1);
            let tx = f.assert_valid(tx);
            ckb_contract_testkit::commit_outputs(&mut f.base.context, &tx);
            assert!(f.base.context.get_cell(&points.0).is_none());
            assert!(f.base.context.get_cell(&points.1).is_none());
        }
    }
}

#[test]
fn immediate_and_partial_cancellation_refund_only_unfilled_tokens() {
    for sell_extended in [false, true] {
        for buy_extended in [false, true] {
            for partial in [false, true] {
                let mut f = Fixture::new(sell_extended, buy_extended);
                let mut state = f.initial();
                f.base.commit(f.creation(&state));
                if partial {
                    let tx = f.fill(&state, 30, f.taker.clone(), 1);
                    state = state.apply(&f.action(&f.taker, 30)).unwrap();
                    f.base.commit(tx);
                }
                let refund = state.remaining().unwrap();
                let tx = f.cancel(&state, f.maker.clone(), 0);
                assert_eq!(tx.outputs_data().get(2).unwrap().raw_data(), amount(refund));
                f.base.commit(tx);
                state = state.apply(&Action::Cancel).unwrap();
                let tx = f.close(&state, f.maker.clone(), 0);
                f.assert_valid(tx);
            }
        }
    }
}

#[test]
fn full_fill_and_low_price_split_fills_never_receive_tokens_for_free() {
    for (sell, buy, pieces, expected) in [
        (100, 201, vec![100], 201),
        (3, 1, vec![1, 1, 1], 3),
        (3, 2, vec![1, 2], 3),
    ] {
        let mut f = Fixture::new(true, false);
        let mut state = State::new(
            ckb_hash(f.maker.as_slice()),
            f.base.asset,
            f.buy_asset,
            sell,
            buy,
        )
        .unwrap();
        f.base.commit(f.creation(&state));
        let mut paid = 0;
        for quantity in pieces {
            paid += state.quote(quantity).unwrap();
            let tx = f.fill(&state, quantity, f.taker.clone(), 1);
            state = state.apply(&f.action(&f.taker, quantity)).unwrap();
            f.base.commit(tx);
        }
        assert_eq!(paid, expected);
        let tx = f.close(&state, f.maker.clone(), 1);
        f.assert_valid(tx);
    }
}

#[test]
fn same_owner_with_different_raw_asset_args_is_a_distinct_pair() {
    let mut f = Fixture::new(false, true);
    f.buy_asset.owner = f.base.asset.owner;
    f.buy_type = f
        .buy_type
        .clone()
        .as_builder()
        .args(f.buy_asset.args().pack())
        .build();
    let before = f.initial();
    f.base.commit(f.creation(&before));
    let tx = f.fill(&before, 100, f.taker.clone(), 1);
    f.assert_valid(tx);
}

#[test]
fn rejects_invalid_genesis_terms_filled_cancelled_and_forged_id() {
    for field in 0..8 {
        let mut f = Fixture::new(true, false);
        let initial = f.initial();
        let tx = f.creation(&initial);
        let mut malformed = initial;
        match field {
            0 => malformed.maker = [0; 32],
            1 => malformed.buy_asset = malformed.sell_asset,
            2 => malformed.sell_total = 0,
            3 => malformed.buy_total = 0,
            4 => malformed.filled = 1,
            5 => malformed.cancelled = true,
            6 => malformed.filled = 101,
            _ => {
                for (script, role) in [
                    (&mut f.base.layout.state_type, STATE_ROLE),
                    (&mut f.base.layout.state_lock, LOCK_ROLE),
                    (&mut f.base.layout.reserve_lock, RESERVE_ROLE),
                ] {
                    *script = script
                        .clone()
                        .as_builder()
                        .args(args::encode(role, &[8; 32], &[]).pack())
                        .build();
                }
                let tx = f.creation(&malformed);
                f.assert_error(tx, Error::Genesis);
                continue;
            }
        }
        f.assert_error(
            replace_data(tx, 0, Bytes::from(malformed.encode())),
            Error::Genesis,
        );
    }
}

#[test]
fn rejects_missing_maker_authority_and_third_party_creation_funding() {
    for missing in [false, true] {
        let mut f = Fixture::new(true, true);
        let mut state = f.initial();
        if missing {
            state.maker = ckb_hash(f.other.as_slice());
        }
        let tx = f.creation(&state);
        let tx = if missing {
            tx
        } else {
            let donor = f.base.cell(f.base.layout.token(&f.other, 1000 * CKB, 100));
            replace_data(tx, 2, amount(1000))
                .as_advanced_builder()
                .input(input(donor))
                .output(f.base.layout.token(&f.other, 999 * CKB, 0).output)
                .output_data(amount(0).pack())
                .build()
        };
        f.assert_error(
            tx,
            if missing {
                Error::Authorization
            } else {
                Error::Token
            },
        );
    }
}

#[test]
fn rejects_wrong_taker_authority_self_trades_and_invalid_fill_amounts() {
    for mode in 0..5 {
        let mut f = Fixture::new(true, true);
        let before = f.initial();
        let tx = f.fill(&before, 30, f.taker.clone(), 1);
        let action = match mode {
            0 => Action::Fill {
                taker: ckb_hash(f.other.as_slice()),
                amount: 30,
            },
            1 => Action::Fill {
                taker: before.maker,
                amount: 30,
            },
            2 => Action::Fill {
                taker: [0; 32],
                amount: 30,
            },
            3 => f.action(&f.taker, 0),
            _ => f.action(&f.taker, 101),
        };
        f.assert_error(
            with_action(tx, 1, &action).unwrap(),
            if mode == 0 {
                Error::Authorization
            } else {
                Error::Transition
            },
        );
    }
}

#[test]
fn rejects_taker_cancelling_or_closing_and_close_before_terminal_state() {
    for mode in 0..3 {
        let mut f = Fixture::new(false, true);
        let before = f.initial();
        let tx = match mode {
            0 => f.cancel(&before, f.taker.clone(), 1),
            1 => f.close(&before.apply(&Action::Cancel).unwrap(), f.taker.clone(), 1),
            _ => f.close(&before, f.maker.clone(), 1),
        };
        f.assert_error(
            tx,
            if mode == 2 {
                Error::Transition
            } else {
                Error::Authorization
            },
        );
    }
}

#[test]
fn rejects_term_price_progress_and_cancel_flag_tampering() {
    for field in 0..9 {
        let mut f = Fixture::new(true, false);
        let before = f.initial();
        let tx = f.fill(&before, 30, f.taker.clone(), 1);
        let mut after = before.apply(&f.action(&f.taker, 30)).unwrap();
        match field {
            0 => after.maker = [7; 32],
            1 => after.sell_asset.owner = [7; 32],
            2 => after.sell_asset.extended_args = false,
            3 => after.buy_asset.owner = [7; 32],
            4 => after.buy_asset.extended_args = true,
            5 => after.sell_total += 1,
            6 => after.buy_total -= 1,
            7 => after.filled += 1,
            _ => after.cancelled = true,
        }
        f.assert_error(
            replace_data(tx, 0, Bytes::from(after.encode())),
            Error::Transition,
        );
    }
}

#[test]
fn rejects_repeated_fills_or_cancellation_of_terminal_orders() {
    for cancelled in [false, true] {
        for action in [
            Action::Fill {
                taker: [4; 32],
                amount: 1,
            },
            Action::Cancel,
        ] {
            let mut f = Fixture::new(true, true);
            let initial = f.initial();
            let before = if cancelled {
                initial.apply(&Action::Cancel).unwrap()
            } else {
                initial.apply(&f.action(&f.taker, 100)).unwrap()
            };
            let pair = f.pair(&before);
            let funding = f.base.cell(Cell::capacity(&f.taker, 1000 * CKB));
            let tx = StateReserveTx::update(&f.base.layout, pair, &before, 0, &action)
                .input(input(funding))
                .state_input_index(1)
                .output(Cell::capacity(&f.taker, 999 * CKB))
                .build()
                .unwrap();
            f.assert_error(tx, Error::Transition);
        }
    }
}

#[test]
fn rejects_missing_malformed_misplaced_and_trailing_witnesses() {
    for mode in 0..4 {
        let mut f = Fixture::new(true, true);
        let before = f.initial();
        let tx = f.fill(&before, 30, f.taker.clone(), 0);
        // Put a native fee input first so a misplaced action is not parsed by
        // xUDT as its own witness before the order's Type can reject it.
        let native = f.base.cell(Cell::capacity(&f.taker, 100 * CKB));
        let mut inputs = vec![input(native)];
        inputs.extend(tx.inputs());
        let returned = Cell::capacity(&f.taker, 100 * CKB);
        let tx = tx
            .as_advanced_builder()
            .set_inputs(inputs)
            .output(returned.output)
            .output_data(returned.data.pack())
            .build();
        let action = f.action(&f.taker, 30);
        let mut wrong = vec![Bytes::new(); 4];
        match mode {
            0 => {}
            1 => wrong[1] = Bytes::from(vec![0]),
            2 => wrong = witnesses(&action, 0, 4),
            _ => {
                let mut bytes = action.encode();
                bytes.push(0);
                wrong[1] = WitnessArgs::new_builder()
                    .input_type(Some(Bytes::from(bytes)).pack())
                    .build()
                    .as_bytes();
            }
        }
        let tx = tx
            .as_advanced_builder()
            .set_witnesses(wrong.into_iter().map(|value| value.pack()).collect())
            .build();
        f.assert_error(
            tx,
            if mode == 3 {
                Error::Encoding
            } else {
                Error::Structure
            },
        );
    }
}

#[test]
fn rejects_underpayment_redirection_and_zero_increment_rounding_attacks() {
    for mode in 0..4 {
        let mut f = Fixture::new(true, true);
        let before = f.initial();
        let tx = f.fill(&before, 30, f.taker.clone(), 1);
        let tx = match mode {
            0 => replace_data(replace_data(tx, 3, amount(60)), 4, amount(940)),
            1 => {
                let output = tx
                    .outputs()
                    .get(3)
                    .unwrap()
                    .as_builder()
                    .lock(f.other.clone())
                    .build();
                replace_output(tx, 3, output)
            }
            2 => {
                let output = tx
                    .outputs()
                    .get(2)
                    .unwrap()
                    .as_builder()
                    .lock(f.other.clone())
                    .build();
                replace_output(tx, 2, output)
            }
            _ => replace_data(replace_data(tx, 2, amount(29)), 1, amount(71)),
        };
        f.assert_error(tx, Error::Token);
    }
    // A cumulative ceiling delta would be zero after the first 1-for-1 fill.
    // The second taker must nevertheless pay one unit under per-fill pricing.
    let mut f = Fixture::new(true, false);
    let initial = State::new(
        ckb_hash(f.maker.as_slice()),
        f.base.asset,
        f.buy_asset,
        3,
        1,
    )
    .unwrap();
    let before = initial.apply(&f.action(&f.taker, 1)).unwrap();
    let tx = f.fill(&before, 1, f.taker.clone(), 1);
    let tx = replace_data(replace_data(tx, 3, amount(0)), 4, amount(1000));
    f.assert_error(tx, Error::Token);
}

#[test]
fn rejects_using_maker_inputs_as_payment_or_taker_inputs_as_delivery() {
    for maker_input in [false, true] {
        let mut f = Fixture::new(false, true);
        let before = f.initial();
        let tx = f.fill(&before, 30, f.taker.clone(), 1);
        let tx = if maker_input {
            let extra = f
                .base
                .cell(Cell::token(&f.maker, &f.buy_type, 200 * CKB, 61));
            replace_data(tx, 4, amount(1000))
                .as_advanced_builder()
                .input(input(extra))
                .output(Cell::capacity(&f.other, 200 * CKB).output)
                .output_data(Bytes::new().pack())
                .build()
        } else {
            let extra = f.base.cell(f.base.layout.token(&f.taker, 200 * CKB, 30));
            let diverted = f.base.layout.token(&f.other, 200 * CKB, 30);
            tx.as_advanced_builder()
                .input(input(extra))
                .output(diverted.output)
                .output_data(diverted.data.pack())
                .build()
        };
        f.assert_error(tx, Error::Token);
    }
}

#[test]
fn rejects_underpaid_redirected_cancel_refunds_and_extra_quote_transfers() {
    for mode in 0..3 {
        let mut f = Fixture::new(true, true);
        let before = f.initial();
        let tx = f.cancel(&before, f.maker.clone(), 1);
        let tx = match mode {
            0 => {
                let output = tx
                    .outputs()
                    .get(2)
                    .unwrap()
                    .as_builder()
                    .lock(f.other.clone())
                    .build();
                replace_output(tx, 2, output)
            }
            1 => {
                let extra = f.base.layout.token(&f.other, 200 * CKB, 1);
                let change = tx
                    .outputs()
                    .get(3)
                    .unwrap()
                    .as_builder()
                    .capacity(599 * CKB)
                    .build();
                replace_data(replace_output(tx, 3, change), 2, amount(99))
                    .as_advanced_builder()
                    .output(extra.output)
                    .output_data(extra.data.pack())
                    .build()
            }
            _ => {
                let donor = f
                    .base
                    .cell(Cell::token(&f.other, &f.buy_type, 200 * CKB, 1));
                let extra = Cell::token(&f.maker, &f.buy_type, 200 * CKB, 1);
                tx.as_advanced_builder()
                    .input(input(donor))
                    .output(extra.output)
                    .output_data(extra.data.pack())
                    .build()
            }
        };
        f.assert_error(tx, Error::Token);
    }
}

#[test]
fn rejects_wrong_reserve_amount_asset_encoding_and_capacity() {
    for mode in 0..6 {
        let mut f = Fixture::new(true, true);
        let before = f.initial();
        let tx = f.fill(&before, 30, f.taker.clone(), 0);
        let (tx, error) = match mode {
            0 => (replace_data(tx, 1, amount(69)), Error::Token),
            1 => {
                let output = tx
                    .outputs()
                    .get(1)
                    .unwrap()
                    .as_builder()
                    .type_(Some(f.buy_type.clone()).pack())
                    .build();
                // Conserve the substituted quote asset so xUDT does not reject
                // inflation before the order can reject the reserve identity.
                let tx = replace_data(replace_output(tx, 1, output), 4, amount(869));
                (tx, Error::Token)
            }
            2 => (replace_data(tx, 1, Bytes::from(vec![0; 17])), Error::Token),
            3 => {
                let output = tx
                    .outputs()
                    .get(1)
                    .unwrap()
                    .as_builder()
                    .capacity(RESERVE_CAPACITY - CKB)
                    .build();
                (replace_output(tx, 1, output), Error::Capacity)
            }
            4 => {
                let output = tx
                    .outputs()
                    .get(0)
                    .unwrap()
                    .as_builder()
                    .capacity(STATE_CAPACITY - CKB)
                    .build();
                (replace_output(tx, 0, output), Error::Capacity)
            }
            _ => {
                let point = tx.inputs().get(1).unwrap().previous_output();
                let (output, data) = f.base.context.get_cell(&point).unwrap();
                f.base.context.create_cell_with_out_point(
                    point,
                    output.as_builder().capacity(RESERVE_CAPACITY - CKB).build(),
                    data,
                );
                (tx, Error::Capacity)
            }
        };
        f.assert_error(tx, error);
    }
}

#[test]
fn rejects_fake_reserve_sources_wrong_indices_and_extra_reserves() {
    for mode in 0..4 {
        let mut f = Fixture::new(true, true);
        let before = f.initial();
        let tx = f.fill(&before, 30, f.taker.clone(), 1);
        let tx = if mode < 3 {
            let (mut state_point, mut reserve_point) = (
                tx.inputs().get(1).unwrap().previous_output(),
                tx.inputs().get(2).unwrap().previous_output(),
            );
            if mode == 0 {
                let reserve = f.base.layout.reserve(100);
                reserve_point = f.base.cell(reserve);
            } else if mode == 1 {
                let replacement = reserve_point.clone().as_builder().index(2u32).build();
                let reserve = f.base.layout.reserve(100);
                f.base.context.create_cell_with_out_point(
                    replacement.clone(),
                    reserve.output,
                    reserve.data,
                );
                reserve_point = replacement;
            } else {
                let replacement = state_point.clone().as_builder().index(3u32).build();
                let (output, data) = f.base.context.get_cell(&state_point).unwrap();
                f.base
                    .context
                    .create_cell_with_out_point(replacement.clone(), output, data);
                state_point = replacement;
            }
            let mut inputs: Vec<_> = tx.inputs().into_iter().collect();
            inputs[1] = input(state_point);
            inputs[2] = input(reserve_point);
            tx.as_advanced_builder().set_inputs(inputs).build()
        } else {
            let extra = f.base.layout.reserve(0);
            let change = tx
                .outputs()
                .get(4)
                .unwrap()
                .as_builder()
                .capacity(1399 * CKB)
                .build();
            replace_output(tx, 4, change)
                .as_advanced_builder()
                .output(extra.output)
                .output_data(extra.data.pack())
                .build()
        };
        f.assert_error(tx, Error::Structure);
    }
}

#[test]
fn rejects_duplicate_state_and_reordered_outputs() {
    for mode in 0..3 {
        let mut f = Fixture::new(true, true);
        let before = f.initial();
        let tx = f.fill(&before, 30, f.taker.clone(), 1);
        let mut outputs: Vec<_> = tx.outputs().into_iter().collect();
        let mut data: Vec<_> = tx.outputs_data().into_iter().collect();
        if mode == 0 {
            outputs.push(outputs[0].clone());
            data.push(data[0].clone());
            outputs[4] = outputs[4]
                .clone()
                .as_builder()
                .capacity(1599 * CKB - STATE_CAPACITY)
                .build();
        } else {
            let index = if mode == 1 { 0 } else { 1 };
            outputs.swap(index, 2);
            data.swap(index, 2);
        }
        f.assert_error(
            tx.as_advanced_builder()
                .set_outputs(outputs)
                .set_outputs_data(data)
                .build(),
            Error::Structure,
        );
    }
}

#[test]
fn rejects_capacity_theft_and_wrong_close_actions() {
    for mode in 0..3 {
        let mut f = Fixture::new(true, true);
        let before = f.initial().apply(&Action::Cancel).unwrap();
        let tx = f.close(&before, f.maker.clone(), 1);
        let tx = if mode == 2 {
            with_action(tx, 1, &Action::Cancel).unwrap()
        } else {
            let output = tx
                .outputs()
                .get(0)
                .unwrap()
                .as_builder()
                .lock(if mode == 0 {
                    f.other.clone()
                } else {
                    f.maker.clone()
                })
                .capacity(STATE_CAPACITY + RESERVE_CAPACITY - 100 * CKB)
                .build();
            let diverted = Cell::capacity(&f.other, 100 * CKB);
            replace_output(tx, 0, output)
                .as_advanced_builder()
                .output(diverted.output)
                .output_data(diverted.data.pack())
                .build()
        };
        f.assert_error(
            tx,
            if mode == 2 {
                Error::Transition
            } else {
                Error::Capacity
            },
        );
    }
}

#[test]
fn rejects_public_locks_and_type_hash_aliases_as_takers() {
    for role in [LOCK_ROLE, RESERVE_ROLE] {
        for alias in [false, true] {
            let mut f = Fixture::new(true, true);
            if alias {
                let id = args::RoleArgs::decode(&f.base.layout.state_type.args().raw_data())
                    .unwrap()
                    .id;
                let script = f
                    .base
                    .context
                    .build_script_with_hash_type(
                        &f.base.code,
                        ScriptHashType::Type,
                        Bytes::from(args::encode(STATE_ROLE, &id, &[])),
                    )
                    .unwrap();
                let role_script = |role| {
                    script
                        .clone()
                        .as_builder()
                        .args(args::encode(role, &id, &[]).pack())
                        .build()
                };
                f.base.layout.state_type = script.clone();
                f.base.layout.state_lock = role_script(LOCK_ROLE);
                f.base.layout.reserve_lock = role_script(RESERVE_ROLE);
            }
            let before = f.initial();
            let public = if role == LOCK_ROLE {
                f.base.layout.state_lock.clone()
            } else {
                f.base.layout.reserve_lock.clone()
            };
            let tx = f.fill(&before, 30, f.taker.clone(), 1);
            let tx = with_action(tx, 1, &f.action(&public, 30)).unwrap();
            f.assert_error(tx, Error::Authorization);
        }
    }
}

#[test]
fn rejects_public_locks_as_makers_and_unpaired_reserve_spends() {
    for reserve in [false, true] {
        for closing in [false, true] {
            let mut f = Fixture::new(true, false);
            let mut before = f.initial();
            before.maker = ckb_hash(if reserve {
                f.base.layout.reserve_lock.as_slice()
            } else {
                f.base.layout.state_lock.as_slice()
            });
            // Isolated malicious predecessors must not use an existing public
            // State/Reserve input as maker authorization.
            let tx = if closing {
                f.close(&before.apply(&Action::Cancel).unwrap(), f.other.clone(), 1)
            } else {
                f.cancel(&before, f.other.clone(), 1)
            };
            f.assert_error(tx, Error::Authorization);
        }
    }
    let mut f = Fixture::new(true, true);
    let before = f.initial();
    let (_, reserve) = f.pair(&before);
    let sale = f.base.layout.token(&f.other, RESERVE_CAPACITY, 100);
    let tx = TransactionBuilder::default()
        .input(input(reserve))
        .output(sale.output)
        .output_data(sale.data.pack())
        .build();
    f.assert_error(tx, Error::Structure);
}

#[test]
fn rejects_state_type_as_lock_and_foreign_token_minting_through_public_locks() {
    for mode in 0..5 {
        let mut f = Fixture::new(true, true);
        let before = f.initial();
        let tx = f.fill(&before, 30, f.taker.clone(), 1);
        let tx = if mode == 0 {
            let rogue = f
                .base
                .cell(Cell::capacity(&f.base.layout.state_type, 200 * CKB));
            tx.as_advanced_builder().input(input(rogue)).build()
        } else {
            let owner = ckb_hash(if mode <= 2 {
                f.base.layout.state_lock.as_slice()
            } else {
                f.base.layout.reserve_lock.as_slice()
            });
            let mut token_args = owner.to_vec();
            token_args.extend_from_slice(&[0; 4]);
            let token_type = if mode % 2 == 0 {
                f.base
                    .context
                    .build_script_with_hash_type(
                        &f.base.xudt_code,
                        ScriptHashType::Type,
                        Bytes::from(token_args),
                    )
                    .unwrap()
            } else {
                f.buy_type
                    .clone()
                    .as_builder()
                    .args(token_args.pack())
                    .build()
            };
            let minted = Cell::token(&f.other, &token_type, 200 * CKB, 1);
            let change = tx
                .outputs()
                .get(4)
                .unwrap()
                .as_builder()
                .capacity(1399 * CKB)
                .build();
            replace_output(tx, 4, change)
                .as_advanced_builder()
                .output(minted.output)
                .output_data(minted.data.pack())
                .build()
        };
        f.assert_error(
            tx,
            if mode == 0 {
                Error::Authorization
            } else {
                Error::Token
            },
        );
    }
}

#[test]
fn permits_unrelated_asset_transfers_without_relaxing_party_net_flows() {
    let mut f = Fixture::new(false, true);
    let before = f.initial();
    let tx = f.fill(&before, 30, f.taker.clone(), 1);
    let asset = f.buy_type.clone().as_builder().args([7u8; 32]).build();
    let extra = f.base.cell(Cell::token(&f.other, &asset, 200 * CKB, 99));
    let returned = Cell::token(&f.other, &asset, 200 * CKB, 99);
    let tx = tx
        .as_advanced_builder()
        .input(input(extra))
        .output(returned.output)
        .output_data(returned.data.pack())
        .build();
    f.assert_valid(tx);
}
