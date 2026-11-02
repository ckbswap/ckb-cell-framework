use ckb_contract_core::{
    args, ckb_hash, token::PlainXudtAsset, BinaryCodec, Error, StateTransition, CKB,
};
use ckb_contract_testkit::witnesses;
use ckb_contract_testkit::{
    amount, input, replace_data, replace_output, workspace_root, Funding, LayoutConfig,
    StateReserveFixture,
};
use ckb_contract_tx::{Cell, StateReserveTx};
use ckb_testtool::ckb_types::core::ScriptHashType;
use ckb_testtool::ckb_types::{
    bytes::Bytes,
    core::{TransactionBuilder, TransactionView},
    packed::{CellOutput, OutPoint, Script, WitnessArgs},
    prelude::*,
};
use framework_escrow_script::{Action, State, Status};

use framework_escrow_script::{
    LOCK_ROLE, RESERVE_CAPACITY, RESERVE_ROLE, STATE_CAPACITY, STATE_ROLE,
};

struct Fixture {
    base: StateReserveFixture,
    buyer: Script,
    seller: Script,
    other: Script,
}
impl Fixture {
    fn new(extended_args: bool) -> Self {
        let base = StateReserveFixture::new(
            &workspace_root(env!("CARGO_MANIFEST_DIR")),
            "framework-escrow-script",
            PlainXudtAsset {
                owner: [9; 32],
                extended_args,
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
        Self {
            buyer: base.wallets[0].clone(),
            seller: base.wallets[1].clone(),
            other: base.wallets[2].clone(),
            base,
        }
    }

    fn initial(&self) -> State {
        State::new(
            ckb_hash(self.buyer.as_slice()),
            ckb_hash(self.seller.as_slice()),
            self.base.asset,
            100,
        )
        .unwrap()
    }

    fn state_output(&self) -> CellOutput {
        self.base.layout.state_output()
    }

    fn reserve_output(&self) -> CellOutput {
        self.base.layout.reserve(0).output
    }

    fn token_output(&self, owner: &Script, capacity: u64) -> CellOutput {
        self.base.layout.token(owner, capacity, 0).output
    }

    fn creation(&self, state: &State) -> TransactionView {
        StateReserveTx::create(&self.base.layout, state, state.reserve_amount())
            .input(input(self.base.funding.clone()))
            .output(self.base.layout.token(
                &self.buyer,
                10_000 * CKB - STATE_CAPACITY - RESERVE_CAPACITY - CKB,
                900,
            ))
            .build()
            .unwrap()
    }

    fn account_pair(&mut self, before: &State) -> (OutPoint, OutPoint) {
        self.base.pair(before, before.reserve_amount())
    }

    fn update(
        &mut self,
        before: &State,
        after: &State,
        action: &Action,
        actor: Script,
        state_index: usize,
    ) -> TransactionView {
        let pair = self.account_pair(before);
        let funding = self
            .base
            .cell(self.base.layout.token(&actor, 1000 * CKB, 500));
        let recipient = if *action == Action::Refund {
            &self.buyer
        } else {
            &self.seller
        };
        StateReserveTx::update(
            &self.base.layout,
            pair,
            after,
            after.reserve_amount(),
            action,
        )
        .input(input(funding))
        .state_input_index(state_index)
        .output(self.base.layout.token(&actor, 799 * CKB, 500))
        .output(self.base.layout.token(recipient, 200 * CKB, before.amount))
        .build()
        .unwrap()
    }

    fn close(&mut self, before: &State, actor: Script, state_index: usize) -> TransactionView {
        let pair = self.account_pair(before);
        let funding = self.base.cell(Cell::capacity(&actor, 1000 * CKB));
        StateReserveTx::close(pair, &Action::Close)
            .input(input(funding))
            .state_input_index(state_index)
            .output(Cell::capacity(
                &self.buyer,
                STATE_CAPACITY + RESERVE_CAPACITY,
            ))
            .output(Cell::capacity(&actor, 999 * CKB))
            .build()
            .unwrap()
    }

    fn assert_valid(&mut self, tx: TransactionView) -> TransactionView {
        self.base.assert_valid(tx)
    }

    fn assert_error(&mut self, tx: TransactionView, expected: Error) {
        self.base.assert_error(tx, expected)
    }

    fn assert_state_error(&mut self, tx: TransactionView, expected: Error) {
        ckb_contract_testkit::assert_type_error(
            &mut self.base.context,
            tx,
            &self.base.layout.state_type,
            expected,
            self.base.max_cycles,
        );
    }

    fn commit(&mut self, tx: TransactionView) {
        self.base.commit(tx)
    }
}

#[test]
fn release_and_refund_lifecycles_for_both_xudt_formats() {
    for extended in [false, true] {
        for action in [Action::Release, Action::Refund] {
            let mut f = Fixture::new(extended);
            let before = f.initial();
            f.commit(f.creation(&before));
            let after = before.apply(&action).unwrap();
            let actor = if action == Action::Release {
                f.buyer.clone()
            } else {
                f.seller.clone()
            };
            let tx = f.update(&before, &after, &action, actor, 1);
            f.commit(tx);
            let tx = f.close(&after, f.buyer.clone(), 1);
            f.assert_valid(tx);
        }
    }
}

#[test]
fn accepts_state_at_zero_and_nonzero_input_indices() {
    for index in [0, 1] {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let after = before.apply(&Action::Release).unwrap();
        let tx = f.update(&before, &after, &Action::Release, f.buyer.clone(), index);
        f.assert_valid(tx);
        f.base.pair = None;
        let tx = f.close(&after, f.buyer.clone(), index);
        f.assert_valid(tx);
    }
}

#[test]
fn rejects_invalid_genesis_terms_status_and_creation_id() {
    for field in 0..7 {
        let mut f = Fixture::new(true);
        let mut initial = f.initial();
        match field {
            0 => initial.amount = 0,
            1 => initial.seller = initial.buyer,
            2 => initial.buyer = [0; 32],
            3 => initial.seller = [0; 32],
            4 => initial.status = Status::Released,
            5 => initial.status = Status::Refunded,
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
            }
        }
        let tx = f.creation(&initial);
        f.assert_error(tx, Error::Genesis);
    }
}

#[test]
fn rejects_creation_without_buyer_authorization_or_buyer_funding() {
    for missing_authority in [false, true] {
        let mut f = Fixture::new(true);
        let mut state = f.initial();
        if missing_authority {
            state.buyer = ckb_hash(f.other.as_slice());
        }
        let tx = f.creation(&state);
        let tx = if missing_authority {
            tx
        } else {
            let donor = f
                .base
                .context
                .create_cell(f.token_output(&f.other, 1000 * CKB), amount(100));
            replace_data(tx, 2, amount(1000))
                .as_advanced_builder()
                .input(input(donor))
                .output(f.token_output(&f.other, 999 * CKB))
                .output_data(amount(0).pack())
                .build()
        };
        f.assert_error(
            tx,
            if missing_authority {
                Error::Authorization
            } else {
                Error::Token
            },
        );
    }
}

#[test]
fn rejects_wrong_party_authorizing_release_refund_or_close() {
    for action in [Action::Release, Action::Refund, Action::Close] {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let tx = if action == Action::Close {
            f.close(
                &before.apply(&Action::Release).unwrap(),
                f.seller.clone(),
                0,
            )
        } else {
            let actor = if action == Action::Release {
                f.seller.clone()
            } else {
                f.buyer.clone()
            };
            f.update(&before, &before.apply(&action).unwrap(), &action, actor, 0)
        };
        f.assert_error(tx, Error::Authorization);
    }
}

#[test]
fn rejects_order_term_and_terminal_status_tampering() {
    for field in 0..6 {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let mut after = before.apply(&Action::Release).unwrap();
        let tx = f.update(&before, &after, &Action::Release, f.buyer.clone(), 0);
        match field {
            0 => after.buyer = [7; 32],
            1 => after.seller = [7; 32],
            2 => after.asset.owner = [7; 32],
            3 => after.asset.extended_args = false,
            4 => after.amount += 1,
            _ => after.status = Status::Refunded,
        }
        let tx = replace_data(tx, 0, Bytes::from(after.encode()));
        f.assert_error(tx, Error::Transition);
    }
}

#[test]
fn rejects_repeated_settlement_and_close_before_settlement() {
    let mut f = Fixture::new(true);
    let funded = f.initial();
    let tx = f.close(&funded, f.buyer.clone(), 0);
    f.assert_error(tx, Error::Transition);
    for status in [Status::Released, Status::Refunded] {
        for action in [Action::Release, Action::Refund, Action::Close] {
            let mut f = Fixture::new(true);
            let mut before = f.initial();
            before.status = status;
            let tx = f.update(&before, &before, &action, f.buyer.clone(), 0);
            let tx = replace_data(tx, 3, amount(0));
            f.assert_error(tx, Error::Transition);
        }
    }
}

#[test]
fn rejects_missing_malformed_or_misplaced_action_witness() {
    for mode in 0..4 {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let tx = f.update(
            &before,
            &before.apply(&Action::Release).unwrap(),
            &Action::Release,
            f.buyer.clone(),
            0,
        );
        // Keep a misplaced action away from xUDT's own witness parser.
        let native = f.base.cell(Cell::capacity(&f.buyer, 100 * CKB));
        let mut inputs = vec![input(native)];
        inputs.extend(tx.inputs());
        let returned = Cell::capacity(&f.buyer, 100 * CKB);
        let tx = tx
            .as_advanced_builder()
            .set_inputs(inputs)
            .output(returned.output)
            .output_data(returned.data.pack())
            .build();
        let mut wrong = vec![Bytes::new(); 4];
        match mode {
            0 => {}
            1 => wrong[1] = Bytes::from(vec![0]),
            2 => wrong = witnesses(&Action::Release, 0, 4),
            _ => {
                let mut bytes = Action::Release.encode();
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
fn rejects_redirected_or_underpaid_release_and_refund() {
    for action in [Action::Release, Action::Refund] {
        for redirect in [false, true] {
            let mut f = Fixture::new(true);
            let before = f.initial();
            let actor = if action == Action::Release {
                f.buyer.clone()
            } else {
                f.seller.clone()
            };
            let tx = f.update(&before, &before.apply(&action).unwrap(), &action, actor, 0);
            let tx = replace_data(tx, 3, amount(if redirect { 0 } else { 99 }));
            let tx = tx
                .as_advanced_builder()
                .output(f.token_output(&f.other, 200 * CKB))
                .output_data(amount(if redirect { 100 } else { 1 }).pack())
                .build();
            f.assert_error(tx, Error::Token);
        }
    }
}

#[test]
fn rejects_wrong_reserve_amount_type_encoding_or_capacity() {
    for field in 0..5 {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let tx = f.update(
            &before,
            &before.apply(&Action::Release).unwrap(),
            &Action::Release,
            f.buyer.clone(),
            0,
        );
        let (tx, error) = match field {
            0 => (replace_data(tx, 1, amount(1)), Error::Token),
            1 => {
                let mut data = amount(0).to_vec();
                data.push(0);
                (replace_data(tx, 1, Bytes::from(data)), Error::Token)
            }
            2 => (
                replace_output(
                    tx,
                    1,
                    f.reserve_output()
                        .as_builder()
                        .type_(None::<Script>.pack())
                        .build(),
                ),
                Error::Token,
            ),
            3 => (
                replace_output(
                    tx,
                    1,
                    f.reserve_output()
                        .as_builder()
                        .capacity(RESERVE_CAPACITY - CKB)
                        .build(),
                ),
                Error::Capacity,
            ),
            _ => (
                replace_output(
                    tx,
                    0,
                    f.state_output()
                        .as_builder()
                        .capacity(STATE_CAPACITY - CKB)
                        .build(),
                ),
                Error::Capacity,
            ),
        };
        f.assert_state_error(tx, error);
    }
}

#[test]
fn rejects_fake_reserve_origins_and_extra_reserve_cells() {
    for mode in 0..3 {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let tx = f.update(
            &before,
            &before.apply(&Action::Release).unwrap(),
            &Action::Release,
            f.buyer.clone(),
            0,
        );
        let tx = if mode < 2 {
            let point = if mode == 0 {
                f.base.context.create_cell(f.reserve_output(), amount(100))
            } else {
                let point = tx
                    .inputs()
                    .get(0)
                    .unwrap()
                    .previous_output()
                    .as_builder()
                    .index(2u32)
                    .build();
                f.base.context.create_cell_with_out_point(
                    point.clone(),
                    f.reserve_output(),
                    amount(100),
                );
                point
            };
            let mut inputs: Vec<_> = tx.inputs().into_iter().collect();
            inputs[1] = input(point);
            tx.as_advanced_builder().set_inputs(inputs).build()
        } else {
            tx.as_advanced_builder()
                .output(f.reserve_output())
                .output_data(amount(0).pack())
                .build()
        };
        f.assert_error(tx, Error::Structure);
    }
}

#[test]
fn rejects_duplicate_state_and_moved_state_or_reserve_outputs() {
    for mode in 0..3 {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let after = before.apply(&Action::Release).unwrap();
        let tx = f.update(&before, &after, &Action::Release, f.buyer.clone(), 0);
        let tx = if mode == 0 {
            tx.as_advanced_builder()
                .output(f.state_output())
                .output_data(Bytes::from(after.encode()).pack())
                .build()
        } else {
            let mut outputs: Vec<_> = tx.outputs().into_iter().collect();
            let mut data: Vec<_> = tx.outputs_data().into_iter().collect();
            outputs.swap(mode - 1, 2);
            data.swap(mode - 1, 2);
            tx.as_advanced_builder()
                .set_outputs(outputs)
                .set_outputs_data(data)
                .build()
        };
        f.assert_error(tx, Error::Structure);
    }
}

#[test]
fn rejects_public_order_locks_as_either_partys_authority() {
    for action in [Action::Release, Action::Refund] {
        for reserve in [false, true] {
            let mut f = Fixture::new(true);
            let mut before = f.initial();
            let public = ckb_hash(if reserve {
                f.base.layout.reserve_lock.as_slice()
            } else {
                f.base.layout.state_lock.as_slice()
            });
            if action == Action::Release {
                before.buyer = public;
            } else {
                before.seller = public;
            }
            let tx = f.update(
                &before,
                &before.apply(&action).unwrap(),
                &action,
                f.other.clone(),
                0,
            );
            f.assert_error(tx, Error::Authorization);
        }
    }
}

#[test]
fn rejects_type_hash_alias_of_public_lock_as_buyer() {
    let mut f = Fixture::new(true);
    let raw = f.base.layout.state_type.args().raw_data();
    let id = raw[1..33].try_into().unwrap();
    let alias_type = f
        .base
        .context
        .build_script_with_hash_type(
            &f.base.code,
            ScriptHashType::Type,
            Bytes::from(args::encode(STATE_ROLE, &id, &[])),
        )
        .unwrap();
    let alias_lock = alias_type
        .clone()
        .as_builder()
        .args(args::encode(LOCK_ROLE, &id, &[]).pack())
        .build();
    let alias_reserve_lock = alias_type
        .clone()
        .as_builder()
        .args(args::encode(RESERVE_ROLE, &id, &[]).pack())
        .build();
    // The alias order has a valid close, so its public Lock succeeds regardless
    // of script-group order. Only the original order's actor guard rejects it.
    let mut alias_order = f.initial();
    alias_order.buyer = ckb_hash(f.other.as_slice());
    alias_order.seller = ckb_hash(f.buyer.as_slice());
    alias_order.status = Status::Refunded;
    let alias_hash = ckb_hash(&alias_order.encode());
    let alias_state = OutPoint::new_builder()
        .tx_hash(alias_hash.pack())
        .index(0u32)
        .build();
    let alias_reserve = OutPoint::new_builder()
        .tx_hash(alias_hash.pack())
        .index(1u32)
        .build();
    let mut before = f.initial();
    before.buyer = ckb_hash(alias_lock.as_slice());
    f.base.context.create_cell_with_out_point(
        alias_state.clone(),
        CellOutput::new_builder()
            .capacity(STATE_CAPACITY)
            .lock(alias_lock)
            .type_(Some(alias_type).pack())
            .build(),
        Bytes::from(alias_order.encode()),
    );
    f.base.context.create_cell_with_out_point(
        alias_reserve.clone(),
        f.token_output(&alias_reserve_lock, RESERVE_CAPACITY),
        amount(0),
    );
    let sponsor = f.base.context.create_cell(
        CellOutput::new_builder()
            .capacity(1000 * CKB)
            .lock(f.buyer.clone())
            .build(),
        Bytes::new(),
    );
    let tx = f.update(
        &before,
        &before.apply(&Action::Release).unwrap(),
        &Action::Release,
        f.other.clone(),
        0,
    );
    let mut actions = witnesses(&Action::Release, 0, 6);
    actions[3] = witnesses(&Action::Close, 0, 1).remove(0);
    let tx = tx
        .as_advanced_builder()
        .input(input(alias_state))
        .input(input(alias_reserve))
        .input(input(sponsor))
        .output(
            CellOutput::new_builder()
                .capacity(751 * CKB)
                .lock(f.other.clone())
                .build(),
        )
        .output_data(Bytes::new().pack())
        .output(
            CellOutput::new_builder()
                .capacity(799 * CKB)
                .lock(f.buyer.clone())
                .build(),
        )
        .output_data(Bytes::new().pack())
        .set_witnesses(actions.into_iter().map(|value| value.pack()).collect())
        .build();
    f.assert_error(tx, Error::Authorization);
}

#[test]
fn rejects_capacity_theft_and_close_without_close_action() {
    for mode in 0..3 {
        let mut f = Fixture::new(true);
        let before = f.initial().apply(&Action::Refund).unwrap();
        let tx = f.close(&before, f.buyer.clone(), 0);
        let tx = if mode == 2 {
            tx.as_advanced_builder()
                .set_witnesses(
                    witnesses(&Action::Refund, 0, 3)
                        .into_iter()
                        .map(|value| value.pack())
                        .collect(),
                )
                .build()
        } else {
            let output = CellOutput::new_builder()
                .capacity(STATE_CAPACITY + RESERVE_CAPACITY - 100 * CKB)
                .lock(if mode == 0 {
                    f.other.clone()
                } else {
                    f.buyer.clone()
                })
                .build();
            let tx = replace_output(tx, 0, output);
            tx.as_advanced_builder()
                .output(
                    CellOutput::new_builder()
                        .capacity(100 * CKB)
                        .lock(f.other.clone())
                        .build(),
                )
                .output_data(Bytes::new().pack())
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
fn rejects_reserve_spend_without_state_and_type_as_input_lock() {
    let mut f = Fixture::new(true);
    let before = f.initial();
    let (_, reserve) = f.account_pair(&before);
    let tx = TransactionBuilder::default()
        .input(input(reserve))
        .output(f.token_output(&f.other, RESERVE_CAPACITY))
        .output_data(amount(100).pack())
        .build();
    f.assert_error(tx, Error::Structure);
    let tx = f.update(
        &before,
        &before.apply(&Action::Release).unwrap(),
        &Action::Release,
        f.buyer.clone(),
        0,
    );
    let rogue = f.base.context.create_cell(
        CellOutput::new_builder()
            .capacity(STATE_CAPACITY)
            .lock(f.base.layout.state_type.clone())
            .build(),
        Bytes::new(),
    );
    let tx = tx.as_advanced_builder().input(input(rogue)).build();
    f.assert_error(tx, Error::Authorization);
}

#[test]
fn rejects_foreign_xudt_mint_through_state_reserve_and_type_hash_alias() {
    for reserve in [false, true] {
        for alias in [false, true] {
            let mut f = Fixture::new(true);
            let before = f.initial();
            let tx = f.update(
                &before,
                &before.apply(&Action::Release).unwrap(),
                &Action::Release,
                f.buyer.clone(),
                0,
            );
            let owner = ckb_hash(if reserve {
                f.base.layout.reserve_lock.as_slice()
            } else {
                f.base.layout.state_lock.as_slice()
            });
            let mut token_args = owner.to_vec();
            token_args.extend_from_slice(&[0; 4]);
            let rogue = if alias {
                f.base
                    .context
                    .build_script_with_hash_type(
                        &f.base.xudt_code,
                        ScriptHashType::Type,
                        Bytes::from(token_args),
                    )
                    .unwrap()
            } else {
                f.base
                    .layout
                    .token_type
                    .clone()
                    .as_builder()
                    .args(token_args.pack())
                    .build()
            };
            let tx = tx
                .as_advanced_builder()
                .output(
                    CellOutput::new_builder()
                        .capacity(200 * CKB)
                        .lock(f.other.clone())
                        .type_(Some(rogue).pack())
                        .build(),
                )
                .output_data(amount(1).pack())
                .build();
            f.assert_error(tx, Error::Token);
        }
    }
}
