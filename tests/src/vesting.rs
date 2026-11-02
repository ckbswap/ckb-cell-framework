use ckb_contract_core::since;
use ckb_contract_core::{
    args, ckb_hash, token::PlainXudtAsset, BinaryCodec, Error, StateTransition, CKB,
};
use ckb_contract_testkit::{action_witness, replace_since};
use ckb_contract_testkit::{
    amount, input, replace_data, replace_output, workspace_root, Funding, LayoutConfig,
    StateReserveFixture,
};
use ckb_contract_tx::{Cell, StateReserveTx};
use ckb_testtool::ckb_types::{
    bytes::Bytes,
    core::{TransactionBuilder, TransactionView},
    packed::{CellOutput, OutPoint, Script, WitnessArgs},
    prelude::*,
};
use framework_vesting_script::{Action, Schedule, State};

use framework_vesting_script::{
    LOCK_ROLE, RESERVE_CAPACITY, RESERVE_ROLE, STATE_CAPACITY, STATE_ROLE,
};

struct Fixture {
    base: StateReserveFixture,
    funder: Script,
    beneficiary: Script,
    other: Script,
}
const START: u64 = 1_700_000_000;
const CLIFF: u64 = START + 200;
const END: u64 = START + 1000;

fn claim(timestamp: u64, amount: u128) -> Action {
    Action::Claim { timestamp, amount }
}
impl Fixture {
    fn new(extended_args: bool) -> Self {
        let base = StateReserveFixture::new(
            &workspace_root(env!("CARGO_MANIFEST_DIR")),
            "framework-vesting-script",
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
                amount: 2000,
            },
        );
        Self {
            funder: base.wallets[0].clone(),
            beneficiary: base.wallets[1].clone(),
            other: base.wallets[2].clone(),
            base,
        }
    }

    fn initial(&self) -> State {
        State::new(
            ckb_hash(self.funder.as_slice()),
            ckb_hash(self.beneficiary.as_slice()),
            self.base.asset,
            1000,
            Schedule {
                start: START,
                cliff: CLIFF,
                end: END,
            },
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
        StateReserveTx::create(&self.base.layout, state, state.total)
            .input(input(self.base.funding.clone()))
            .output(self.base.layout.token(
                &self.funder,
                10_000 * CKB - STATE_CAPACITY - RESERVE_CAPACITY - CKB,
                2000 - state.total,
            ))
            .build()
            .unwrap()
    }

    fn pair(&mut self, before: &State) -> (OutPoint, OutPoint) {
        self.base.pair(before, before.total - before.claimed)
    }

    fn update(
        &mut self,
        before: &State,
        after: &State,
        action: &Action,
        actor: Script,
        state_index: usize,
    ) -> TransactionView {
        let pair = self.pair(before);
        let timestamp = match action {
            Action::Claim { timestamp, .. } => *timestamp,
            Action::Close => END,
        };
        let funding = self
            .base
            .cell(self.base.layout.token(&actor, 1000 * CKB, 500));
        StateReserveTx::update(
            &self.base.layout,
            pair,
            after,
            after.total - after.claimed,
            action,
        )
        .input(input(funding))
        .state_input_index(state_index)
        .state_since(since::absolute_timestamp(timestamp).unwrap())
        .output(self.base.layout.token(&actor, 799 * CKB, 500))
        .output(self.base.layout.token(
            &self.beneficiary,
            200 * CKB,
            after.claimed - before.claimed,
        ))
        .build()
        .unwrap()
    }

    fn close(&mut self, before: &State, actor: Script, state_index: usize) -> TransactionView {
        let pair = self.pair(before);
        let funding = self.base.cell(Cell::capacity(&actor, 1000 * CKB));
        StateReserveTx::close(pair, &Action::Close)
            .input(input(funding))
            .state_input_index(state_index)
            .output(Cell::capacity(
                &self.funder,
                STATE_CAPACITY + RESERVE_CAPACITY,
            ))
            .output(Cell::capacity(&actor, 999 * CKB))
            .build()
            .unwrap()
    }

    fn verify_time(
        &mut self,
        tx: &TransactionView,
        median_ms: u64,
    ) -> Result<(), ckb_testtool::ckb_error::Error> {
        self.base.verify_time(tx, median_ms)
    }

    fn assert_vm_valid(&mut self, tx: TransactionView) -> TransactionView {
        self.base.assert_valid(tx)
    }

    fn assert_valid_at(&mut self, tx: TransactionView, seconds: u64) -> TransactionView {
        self.base.assert_valid_at(tx, seconds)
    }

    fn assert_error(&mut self, tx: TransactionView, error: Error) {
        self.base.assert_error(tx, error)
    }

    fn commit(&mut self, tx: TransactionView, seconds: u64) {
        self.base.commit_at(tx, seconds)
    }
}

#[test]
fn complete_linear_and_partial_claim_lifecycles_for_both_xudt_formats() {
    for extended in [false, true] {
        let mut f = Fixture::new(extended);
        let mut state = f.initial();
        f.commit(f.creation(&state), START - 1);
        for (timestamp, paid) in [(CLIFF, 100), (CLIFF, 100), (START + 500, 300), (END, 500)] {
            let action = claim(timestamp, paid);
            let after = state.apply(&action).unwrap();
            let tx = f.update(&state, &after, &action, f.beneficiary.clone(), 1);
            f.commit(tx, timestamp);
            state = after;
        }
        let tx = f.close(&state, f.funder.clone(), 1);
        f.assert_valid_at(tx, END);
    }
}

#[test]
fn consensus_rejects_future_claim_even_when_the_vm_accepts_it() {
    let mut f = Fixture::new(true);
    let before = f.initial();
    let action = claim(END, 1000);
    let tx = f.update(
        &before,
        &before.apply(&action).unwrap(),
        &action,
        f.beneficiary.clone(),
        1,
    );
    let tx = f.assert_vm_valid(tx);
    let error = f
        .verify_time(&tx, END * 1000 - 1)
        .expect_err("future since matured early");
    assert!(format!("{error:?}").contains("Immature"));
    f.verify_time(&tx, END * 1000).unwrap();
    f.verify_time(&tx, END * 1000 + 1).unwrap();
}

#[test]
fn rejects_missing_relative_reserved_wrong_metric_or_mismatched_since() {
    for index in [0, 1] {
        for bad_since in [
            0,
            END,
            0x2000_0000_0000_0000 | END,
            0xc000_0000_0000_0000 | END,
            0x4100_0000_0000_0000 | END,
            since::absolute_timestamp(END - 1).unwrap(),
            since::absolute_timestamp(END + 1).unwrap(),
        ] {
            let mut f = Fixture::new(true);
            let before = f.initial();
            let action = claim(END, 1000);
            let tx = f.update(
                &before,
                &before.apply(&action).unwrap(),
                &action,
                f.beneficiary.clone(),
                index,
            );
            let tx = replace_since(tx, index, bad_since);
            f.assert_error(tx, Error::Transition);
        }
    }
}

#[test]
fn time_proof_on_another_input_cannot_authorize_the_state() {
    let mut f = Fixture::new(true);
    let before = f.initial();
    let action = claim(END, 1000);
    let tx = f.update(
        &before,
        &before.apply(&action).unwrap(),
        &action,
        f.beneficiary.clone(),
        1,
    );
    let tx = replace_since(tx, 1, 0);
    let tx = replace_since(tx, 0, since::absolute_timestamp(END).unwrap());
    f.assert_error(tx, Error::Transition);
}

#[test]
fn accepts_claims_at_both_state_indices_and_self_funded_vesting() {
    for index in [0, 1] {
        for same_owner in [false, true] {
            let mut f = Fixture::new(true);
            if same_owner {
                f.beneficiary = f.funder.clone();
            }
            let before = f.initial();
            let action = claim(END, 1000);
            f.assert_valid_at(f.creation(&before), START);
            let tx = f.update(
                &before,
                &before.apply(&action).unwrap(),
                &action,
                f.beneficiary.clone(),
                index,
            );
            f.assert_valid_at(tx, END);
        }
    }
}

#[test]
fn rejects_precliff_overclaim_zero_and_repeated_claims() {
    for (timestamp, paid, already) in [
        (CLIFF - 1, 1, 0),
        (CLIFF, 201, 0),
        (END, 0, 0),
        (END, 1001, 0),
        (CLIFF, 1, 200),
        (END, 1, 1000),
    ] {
        let mut f = Fixture::new(true);
        let mut before = f.initial();
        before.claimed = already;
        let mut after = before.clone();
        after.claimed = (already + paid).min(before.total);
        let tx = f.update(
            &before,
            &after,
            &claim(timestamp, paid),
            f.beneficiary.clone(),
            0,
        );
        f.assert_error(tx, Error::Transition);
    }
}

#[test]
fn rejects_genesis_claimed_amount_bad_schedule_zero_identity_and_forged_id() {
    for field in 0..8 {
        let mut f = Fixture::new(true);
        let mut state = f.initial();
        match field {
            0 => state.claimed = 1,
            1 => state.total = 0,
            2 => state.schedule.end = state.schedule.start,
            3 => state.schedule.cliff = state.schedule.start - 1,
            4 => state.schedule.cliff = state.schedule.end + 1,
            5 => state.funder = [0; 32],
            6 => state.beneficiary = [0; 32],
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
        let tx = f.creation(&state);
        f.assert_error(tx, Error::Genesis);
    }
}

#[test]
fn rejects_creation_without_funder_authorization_or_net_funding() {
    for missing_authority in [false, true] {
        let mut f = Fixture::new(true);
        let mut state = f.initial();
        if missing_authority {
            state.funder = ckb_hash(f.other.as_slice());
        }
        let tx = f.creation(&state);
        let tx = if missing_authority {
            tx
        } else {
            let donor = f
                .base
                .context
                .create_cell(f.token_output(&f.other, 1000 * CKB), amount(1000));
            replace_data(tx, 2, amount(2000))
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
fn rejects_funder_claiming_beneficiary_tokens_and_beneficiary_closing() {
    let mut f = Fixture::new(true);
    let before = f.initial();
    let action = claim(END, 1000);
    let tx = f.update(
        &before,
        &before.apply(&action).unwrap(),
        &action,
        f.funder.clone(),
        0,
    );
    f.assert_error(tx, Error::Authorization);
    let mut f = Fixture::new(true);
    let before = f.initial().apply(&action).unwrap();
    let tx = f.close(&before, f.beneficiary.clone(), 0);
    f.assert_error(tx, Error::Authorization);
}

#[test]
fn rejects_changing_terms_and_claimed_balance() {
    for field in 0..9 {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let action = claim(CLIFF, 100);
        let mut after = before.apply(&action).unwrap();
        let tx = f.update(&before, &after, &action, f.beneficiary.clone(), 0);
        match field {
            0 => after.funder = [7; 32],
            1 => after.beneficiary = [7; 32],
            2 => after.asset.owner = [7; 32],
            3 => after.asset.extended_args = false,
            4 => after.total += 1,
            5 => after.claimed += 1,
            6 => after.schedule.start += 1,
            7 => after.schedule.cliff += 1,
            _ => after.schedule.end += 1,
        }
        let tx = replace_data(tx, 0, Bytes::from(after.encode()));
        f.assert_error(tx, Error::Transition);
    }
}

#[test]
fn rejects_redirecting_or_underpaying_claims() {
    for redirect in [false, true] {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let action = claim(CLIFF, 100);
        let tx = f.update(
            &before,
            &before.apply(&action).unwrap(),
            &action,
            f.beneficiary.clone(),
            0,
        );
        let tx = replace_data(tx, 3, amount(if redirect { 0 } else { 99 }));
        let tx = tx
            .as_advanced_builder()
            .output(f.token_output(&f.other, 200 * CKB))
            .output_data(amount(if redirect { 100 } else { 1 }).pack())
            .build();
        f.assert_error(tx, Error::Token);
    }
}

#[test]
fn rejects_missing_malformed_misplaced_and_trailing_action_witness() {
    for mode in 0..4 {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let action = claim(CLIFF, 100);
        let mut tx = f.update(
            &before,
            &before.apply(&action).unwrap(),
            &action,
            f.beneficiary.clone(),
            1,
        );
        let mut wrong = vec![Bytes::new(); 3];
        match mode {
            0 => {}
            1 => wrong[1] = Bytes::from(vec![0]),
            2 => {
                // Put the misplaced Action on an untyped input. An Action in
                // the xUDT input's input_type also violates xUDT's witness format,
                // making the observed error depend on script group order.
                let wallet_cell = Cell::capacity(&f.beneficiary, 100 * CKB);
                let point = f.base.cell(wallet_cell.clone());
                tx = tx
                    .as_advanced_builder()
                    .input(input(point))
                    .output(wallet_cell.output)
                    .output_data(wallet_cell.data.pack())
                    .build();
                wrong.push(action_witness(&action));
            }
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
            .set_witnesses(wrong.into_iter().map(|v| v.pack()).collect())
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
fn rejects_wrong_reserve_amount_asset_encoding_and_capacities() {
    for field in 0..5 {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let action = claim(CLIFF, 100);
        let tx = f.update(
            &before,
            &before.apply(&action).unwrap(),
            &action,
            f.beneficiary.clone(),
            0,
        );
        let (tx, error) = match field {
            0 => (replace_data(tx, 1, amount(899)), Error::Token),
            1 => {
                let mut bytes = amount(900).to_vec();
                bytes.push(0);
                (replace_data(tx, 1, Bytes::from(bytes)), Error::Token)
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
        f.assert_error(tx, error);
    }
}

#[test]
fn rejects_fake_reserve_sources_extra_cells_and_moved_outputs() {
    for mode in 0..6 {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let action = claim(CLIFF, 100);
        let after = before.apply(&action).unwrap();
        let tx = f.update(&before, &after, &action, f.beneficiary.clone(), 0);
        let tx = match mode {
            0 | 1 => {
                let point = if mode == 0 {
                    f.base.context.create_cell(f.reserve_output(), amount(1000))
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
                        amount(1000),
                    );
                    point
                };
                let mut inputs: Vec<_> = tx.inputs().into_iter().collect();
                inputs[1] = input(point);
                tx.as_advanced_builder().set_inputs(inputs).build()
            }
            2 => tx
                .as_advanced_builder()
                .output(f.reserve_output())
                .output_data(amount(0).pack())
                .build(),
            3 => tx
                .as_advanced_builder()
                .output(f.state_output())
                .output_data(Bytes::from(after.encode()).pack())
                .build(),
            _ => {
                let mut outputs: Vec<_> = tx.outputs().into_iter().collect();
                let mut data: Vec<_> = tx.outputs_data().into_iter().collect();
                outputs.swap(mode - 4, 2);
                data.swap(mode - 4, 2);
                tx.as_advanced_builder()
                    .set_outputs(outputs)
                    .set_outputs_data(data)
                    .build()
            }
        };
        f.assert_error(tx, Error::Structure);
    }
}

#[test]
fn rejects_close_with_unclaimed_principal_wrong_action_or_capacity_theft() {
    for mode in 0..3 {
        let mut f = Fixture::new(true);
        let mut before = f.initial();
        if mode > 0 {
            before.claimed = before.total;
        }
        let tx = f.close(&before, f.funder.clone(), 0);
        let tx = if mode == 1 {
            tx.as_advanced_builder()
                .set_witnesses(vec![
                    action_witness(&claim(END, 1)).pack(),
                    Bytes::new().pack(),
                    Bytes::new().pack(),
                ])
                .build()
        } else if mode == 2 {
            let output = CellOutput::new_builder()
                .capacity(STATE_CAPACITY + RESERVE_CAPACITY - 100 * CKB)
                .lock(f.funder.clone())
                .build();
            replace_output(tx, 0, output)
                .as_advanced_builder()
                .output(
                    CellOutput::new_builder()
                        .capacity(100 * CKB)
                        .lock(f.other.clone())
                        .build(),
                )
                .output_data(Bytes::new().pack())
                .build()
        } else {
            tx
        };
        f.assert_error(
            tx,
            if mode == 2 {
                Error::Capacity
            } else {
                Error::Transition
            },
        );
    }
}

#[test]
fn rejects_public_locks_as_beneficiary_and_state_type_as_input_lock() {
    for reserve in [false, true] {
        let mut f = Fixture::new(true);
        let mut before = f.initial();
        before.beneficiary = ckb_hash(if reserve {
            f.base.layout.reserve_lock.as_slice()
        } else {
            f.base.layout.state_lock.as_slice()
        });
        let action = claim(CLIFF, 100);
        let tx = f.update(
            &before,
            &before.apply(&action).unwrap(),
            &action,
            f.other.clone(),
            0,
        );
        f.assert_error(tx, Error::Authorization);
    }
    let mut f = Fixture::new(true);
    let before = f.initial();
    let action = claim(CLIFF, 100);
    let tx = f.update(
        &before,
        &before.apply(&action).unwrap(),
        &action,
        f.beneficiary.clone(),
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
fn rejects_reserve_spend_without_state_and_unrelated_xudt_mint() {
    let mut f = Fixture::new(true);
    let before = f.initial();
    let (_, reserve) = f.pair(&before);
    let tx = TransactionBuilder::default()
        .input(input(reserve))
        .output(f.token_output(&f.other, RESERVE_CAPACITY))
        .output_data(amount(1000).pack())
        .build();
    f.assert_error(tx, Error::Structure);
    for reserve in [false, true] {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let action = claim(CLIFF, 100);
        let tx = f.update(
            &before,
            &before.apply(&action).unwrap(),
            &action,
            f.beneficiary.clone(),
            0,
        );
        let mut args = ckb_hash(if reserve {
            f.base.layout.reserve_lock.as_slice()
        } else {
            f.base.layout.state_lock.as_slice()
        })
        .to_vec();
        args.extend_from_slice(&[0; 4]);
        let rogue = f
            .base
            .layout
            .token_type
            .clone()
            .as_builder()
            .args(args.pack())
            .build();
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
