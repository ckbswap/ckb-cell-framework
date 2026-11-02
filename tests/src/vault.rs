use ckb_contract_core::{
    args, ckb_hash, token::PlainXudtAsset, BinaryCodec, Error, StateTransition, CKB,
};
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
use framework_vault_script::{Action, State};

use framework_vault_script::{
    LOCK_ROLE, RESERVE_CAPACITY, RESERVE_ROLE, STATE_CAPACITY, STATE_ROLE,
};

struct Fixture {
    base: StateReserveFixture,
    owner: Script,
    other: Script,
}
impl Fixture {
    fn new(extended_args: bool) -> Self {
        let base = StateReserveFixture::new(
            &workspace_root(env!("CARGO_MANIFEST_DIR")),
            "framework-vault-script",
            PlainXudtAsset {
                owner: [9; 32],
                extended_args,
            },
            LayoutConfig {
                roles: [STATE_ROLE, LOCK_ROLE, RESERVE_ROLE],
                state_capacity: STATE_CAPACITY,
                reserve_capacity: RESERVE_CAPACITY,
            },
            Funding::Capacity(10_000 * CKB),
        );
        Self {
            owner: base.wallets[0].clone(),
            other: base.wallets[1].clone(),
            base,
        }
    }

    fn initial(&self) -> State {
        State::new(ckb_hash(self.owner.as_slice()), self.base.asset)
    }

    fn state_output(&self) -> CellOutput {
        self.base.layout.state_output()
    }

    fn reserve_output(&self) -> CellOutput {
        self.base.layout.reserve(0).output
    }

    fn wallet_token(&self, owner: &Script, capacity: u64) -> CellOutput {
        self.base.layout.token(owner, capacity, 0).output
    }

    fn creation(&self, state: &State) -> TransactionView {
        StateReserveTx::create(&self.base.layout, state, 0)
            .input(input(self.base.funding.clone()))
            .output(Cell::capacity(&self.owner, 9_499 * CKB))
            .build()
            .unwrap()
    }

    fn account_pair(&mut self, before: &State) -> (OutPoint, OutPoint) {
        self.base.pair(before, before.balance)
    }

    fn update(
        &mut self,
        before: &State,
        after: &State,
        action: &Action,
        authorized: bool,
        state_index: usize,
    ) -> TransactionView {
        let pair = self.account_pair(before);
        let wallet = if authorized {
            self.owner.clone()
        } else {
            self.other.clone()
        };
        let funding = self
            .base
            .cell(self.base.layout.token(&wallet, 1000 * CKB, 1000));
        let output_amount = match action {
            Action::Deposit(value) => 1000 - value,
            Action::Withdraw(value) => 1000 + value,
            Action::Close => 1000,
        };
        StateReserveTx::update(&self.base.layout, pair, after, after.balance, action)
            .input(input(funding))
            .state_input_index(state_index)
            .output(self.base.layout.token(&wallet, 999 * CKB, output_amount))
            .build()
            .unwrap()
    }

    fn close(&mut self, before: &State, authorized: bool) -> TransactionView {
        let pair = self.account_pair(before);
        let wallet = if authorized {
            self.owner.clone()
        } else {
            self.other.clone()
        };
        let funding = self.base.cell(Cell::capacity(&wallet, 1000 * CKB));
        StateReserveTx::close(pair, &Action::Close)
            .input(input(funding))
            .output(Cell::capacity(&wallet, 1499 * CKB))
            .build()
            .unwrap()
    }

    fn assert_valid(&mut self, tx: TransactionView) -> TransactionView {
        self.base.assert_valid(tx)
    }

    fn assert_error(&mut self, tx: TransactionView, expected: Error) {
        self.base.assert_error(tx, expected)
    }

    fn commit(&mut self, tx: TransactionView) {
        self.base.commit(tx)
    }
}

#[test]
fn complete_lifecycle_returns_exact_principal_for_both_xudt_formats() {
    for extended in [false, true] {
        let mut f = Fixture::new(extended);
        let mut state = f.initial();
        f.commit(f.creation(&state));
        for (action, balance) in [
            (Action::Deposit(100), 100),
            (Action::Deposit(50), 150),
            (Action::Withdraw(40), 110),
            (Action::Withdraw(110), 0),
            (Action::Deposit(7), 7),
            (Action::Withdraw(7), 0),
        ] {
            let after = state.apply(&action).unwrap();
            assert_eq!(after.balance, balance);
            let tx = f.update(&state, &after, &action, true, 1);
            f.commit(tx);
            state = after;
        }
        let tx = f.close(&state, true);
        f.assert_valid(tx);
    }
}

#[test]
fn accepts_state_at_input_zero_and_nonzero() {
    for state_index in [0, 1] {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let action = Action::Deposit(100);
        let tx = f.update(
            &before,
            &before.apply(&action).unwrap(),
            &action,
            true,
            state_index,
        );
        f.assert_valid(tx);
    }
}

#[test]
fn rejects_genesis_balance_nonce_and_forged_id() {
    for field in 0..3 {
        let mut f = Fixture::new(true);
        let mut state = f.initial();
        match field {
            0 => state.balance = 1,
            1 => state.nonce = 1,
            _ => {
                f.base.layout.state_type = f
                    .base
                    .layout
                    .state_type
                    .as_builder()
                    .args(args::encode(STATE_ROLE, &[8; 32], &[]).pack())
                    .build();
                f.base.layout.state_lock = f
                    .base
                    .layout
                    .state_lock
                    .as_builder()
                    .args(args::encode(LOCK_ROLE, &[8; 32], &[]).pack())
                    .build();
                f.base.layout.reserve_lock = f
                    .base
                    .layout
                    .reserve_lock
                    .as_builder()
                    .args(args::encode(RESERVE_ROLE, &[8; 32], &[]).pack())
                    .build();
            }
        }
        let tx = f.creation(&state);
        f.assert_error(tx, Error::Genesis);
    }
}

#[test]
fn rejects_genesis_without_owner_authorization() {
    let mut f = Fixture::new(true);
    let mut state = f.initial();
    state.owner = ckb_hash(f.other.as_slice());
    let tx = f.creation(&state);
    f.assert_error(tx, Error::Authorization);
}

#[test]
fn rejects_foreign_wallet_and_public_account_locks_as_owner() {
    for mode in 0..3 {
        let mut f = Fixture::new(true);
        let mut before = f.initial();
        before.balance = 100;
        if mode != 0 {
            before.owner = ckb_hash(if mode == 1 {
                f.base.layout.state_lock.as_slice()
            } else {
                f.base.layout.reserve_lock.as_slice()
            });
        }
        let action = Action::Withdraw(20);
        let tx = f.update(&before, &before.apply(&action).unwrap(), &action, false, 0);
        f.assert_error(tx, Error::Authorization);
    }
}

#[test]
fn rejects_tampered_balance_nonce_owner_and_asset() {
    for field in 0..5 {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let action = Action::Deposit(100);
        let mut after = before.apply(&action).unwrap();
        let tx = f.update(&before, &after, &action, true, 0);
        match field {
            0 => after.balance += 1,
            1 => after.nonce += 1,
            2 => after.owner = [7; 32],
            3 => after.asset.owner = [7; 32],
            _ => after.asset.extended_args = false,
        }
        let tx = replace_data(tx, 0, Bytes::from(after.encode()));
        f.assert_error(tx, Error::Transition);
    }
}

#[test]
fn rejects_zero_deposits_zero_withdrawals_and_overdrafts() {
    for action in [Action::Deposit(0), Action::Withdraw(0), Action::Withdraw(1)] {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let tx = f.update(&before, &before, &action, true, 0);
        let tx = replace_data(tx, 2, amount(1000));
        f.assert_error(tx, Error::Transition);
    }
}

#[test]
fn rejects_missing_malformed_and_misplaced_action_witness() {
    for mode in 0..4 {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let action = Action::Deposit(100);
        let tx = f.update(&before, &before.apply(&action).unwrap(), &action, true, 0);
        // A misplaced action belongs on a native input, not an xUDT input
        // whose own witness parser could fail before this contract runs.
        let native = f.base.cell(Cell::capacity(&f.owner, 100 * CKB));
        let mut inputs = vec![input(native)];
        inputs.extend(tx.inputs());
        let returned = Cell::capacity(&f.owner, 100 * CKB);
        let tx = tx
            .as_advanced_builder()
            .set_inputs(inputs)
            .output(returned.output)
            .output_data(returned.data.pack())
            .build();
        let mut witnesses: Vec<_> = ckb_contract_testkit::witnesses(&action, 1, 4)
            .into_iter()
            .map(|value| value.pack())
            .collect();
        match mode {
            0 => witnesses[1] = Bytes::new().pack(),
            1 => {
                let mut bytes = action.encode();
                bytes.push(0);
                witnesses[1] = WitnessArgs::new_builder()
                    .input_type(Some(Bytes::from(bytes)).pack())
                    .build()
                    .as_bytes()
                    .pack();
            }
            2 => witnesses.swap(0, 1),
            _ => {
                witnesses[1] = WitnessArgs::new_builder()
                    .input_type(Some(Bytes::from_static(b"VLA01\x03")).pack())
                    .build()
                    .as_bytes()
                    .pack()
            }
        }
        f.assert_error(
            tx.as_advanced_builder().set_witnesses(witnesses).build(),
            if mode == 0 || mode == 2 {
                Error::Structure
            } else {
                Error::Encoding
            },
        );
    }
}

#[test]
fn rejects_underfunded_or_third_party_funded_deposits() {
    for mode in 0..2 {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let action = Action::Deposit(100);
        let tx = f.update(&before, &before.apply(&action).unwrap(), &action, true, 0);
        let source = f
            .base
            .context
            .create_cell(f.wallet_token(&f.other, 200 * CKB), amount(100));
        let tx = tx.as_advanced_builder().input(input(source)).build();
        // Total supply is conserved in both attacks; the owner's net debit is wrong.
        let tx = replace_data(tx, 2, amount(if mode == 0 { 1000 } else { 999 }));
        let tx = tx
            .as_advanced_builder()
            .output(f.wallet_token(&f.other, 200 * CKB))
            .output_data(amount(if mode == 0 { 0 } else { 1 }).pack())
            .build();
        f.assert_error(tx, Error::Token);
    }
}

#[test]
fn rejects_withdrawal_redirect_and_underpayment() {
    for redirect in [false, true] {
        let mut f = Fixture::new(true);
        let mut before = f.initial();
        before.balance = 100;
        let action = Action::Withdraw(40);
        let tx = f.update(&before, &before.apply(&action).unwrap(), &action, true, 0);
        let tx = replace_data(tx, 2, amount(if redirect { 1000 } else { 1039 }));
        let tx = tx
            .as_advanced_builder()
            .output(f.wallet_token(&f.other, 200 * CKB))
            .output_data(amount(if redirect { 40 } else { 1 }).pack())
            .build();
        f.assert_error(tx, Error::Token);
    }
}

#[test]
fn rejects_wrong_reserve_amount_type_data_and_capacity() {
    for field in 0..5 {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let action = Action::Deposit(100);
        let tx = f.update(&before, &before.apply(&action).unwrap(), &action, true, 0);
        let (tx, error) = match field {
            0 => (replace_data(tx, 1, amount(99)), Error::Token),
            1 => {
                let mut bytes = amount(100).to_vec();
                bytes.push(0);
                (replace_data(tx, 1, Bytes::from(bytes)), Error::Token)
            }
            2 => {
                let output = f
                    .reserve_output()
                    .as_builder()
                    .type_(None::<Script>.pack())
                    .build();
                (replace_output(tx, 1, output), Error::Token)
            }
            3 => {
                let output = f
                    .reserve_output()
                    .as_builder()
                    .capacity(RESERVE_CAPACITY - CKB)
                    .build();
                (replace_output(tx, 1, output), Error::Capacity)
            }
            _ => {
                let output = f
                    .state_output()
                    .as_builder()
                    .capacity(STATE_CAPACITY - CKB)
                    .build();
                (replace_output(tx, 0, output), Error::Capacity)
            }
        };
        f.assert_error(tx, error);
    }
}

#[test]
fn rejects_foreign_reserve_source_and_extra_reserve_cells() {
    for mode in 0..3 {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let action = Action::Deposit(100);
        let tx = f.update(&before, &before.apply(&action).unwrap(), &action, true, 0);
        let tx = if mode < 2 {
            let reserve = if mode == 0 {
                f.base.context.create_cell(f.reserve_output(), amount(0))
            } else {
                let state_point = tx.inputs().get(0).unwrap().previous_output();
                let point = state_point.as_builder().index(2u32).build();
                f.base.context.create_cell_with_out_point(
                    point.clone(),
                    f.reserve_output(),
                    amount(0),
                );
                point
            };
            let mut inputs: Vec<_> = tx.inputs().into_iter().collect();
            inputs[1] = input(reserve);
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
fn rejects_duplicate_state_and_moved_outputs() {
    for mode in 0..3 {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let action = Action::Deposit(100);
        let after = before.apply(&action).unwrap();
        let tx = f.update(&before, &after, &action, true, 0);
        let tx = if mode == 0 {
            tx.as_advanced_builder()
                .output(f.state_output())
                .output_data(Bytes::from(after.encode()).pack())
                .build()
        } else {
            let mut outputs: Vec<_> = tx.outputs().into_iter().collect();
            let mut data: Vec<_> = tx.outputs_data().into_iter().collect();
            let indices = if mode == 1 { (0, 2) } else { (1, 2) };
            outputs.swap(indices.0, indices.1);
            data.swap(indices.0, indices.1);
            tx.as_advanced_builder()
                .set_outputs(outputs)
                .set_outputs_data(data)
                .build()
        };
        f.assert_error(tx, Error::Structure);
    }
}

#[test]
fn rejects_destroying_nonempty_account_and_unauthorized_close() {
    for nonempty in [false, true] {
        let mut f = Fixture::new(true);
        let mut before = f.initial();
        if nonempty {
            before.balance = 1;
        }
        let tx = f.close(&before, nonempty);
        f.assert_error(
            tx,
            if nonempty {
                Error::Transition
            } else {
                Error::Authorization
            },
        );
    }
}

#[test]
fn rejects_close_without_close_action_or_with_capacity_theft() {
    for mode in 0..2 {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let tx = f.close(&before, true);
        let (tx, error) = if mode == 0 {
            let witness = WitnessArgs::new_builder()
                .input_type(Some(Bytes::from(Action::Withdraw(1).encode())).pack())
                .build()
                .as_bytes();
            (
                tx.as_advanced_builder()
                    .set_witnesses(vec![
                        witness.pack(),
                        Bytes::new().pack(),
                        Bytes::new().pack(),
                    ])
                    .build(),
                Error::Transition,
            )
        } else {
            let tx = replace_output(
                tx,
                0,
                CellOutput::new_builder()
                    .capacity(1399 * CKB)
                    .lock(f.owner.clone())
                    .build(),
            );
            (
                tx.as_advanced_builder()
                    .output(
                        CellOutput::new_builder()
                            .capacity(100 * CKB)
                            .lock(f.other.clone())
                            .build(),
                    )
                    .output_data(Bytes::new().pack())
                    .build(),
                Error::Capacity,
            )
        };
        f.assert_error(tx, error);
    }
}

#[test]
fn rejects_closing_state_with_wrong_capacity() {
    let mut f = Fixture::new(true);
    let before = f.initial();
    let tx = f.close(&before, true);
    let state_point = tx.inputs().get(0).unwrap().previous_output();
    // A synthetic malformed predecessor exercises the explicit close adapter.
    f.base.context.create_cell_with_out_point(
        state_point,
        f.state_output()
            .as_builder()
            .capacity(STATE_CAPACITY - CKB)
            .build(),
        Bytes::from(before.encode()),
    );
    f.assert_error(tx, Error::Capacity);
}

#[test]
fn rejects_spending_reserve_without_corresponding_state() {
    let mut f = Fixture::new(true);
    let before = f.initial();
    let (_, reserve) = f.account_pair(&before);
    let tx = TransactionBuilder::default()
        .input(input(reserve))
        .output(
            CellOutput::new_builder()
                .capacity(RESERVE_CAPACITY - CKB)
                .lock(f.other.clone())
                .build(),
        )
        .output_data(Bytes::new().pack())
        .build();
    f.assert_error(tx, Error::Structure);
}

#[test]
fn rejects_using_state_type_as_public_input_lock() {
    let mut f = Fixture::new(true);
    let before = f.initial();
    let action = Action::Deposit(100);
    let tx = f.update(&before, &before.apply(&action).unwrap(), &action, true, 0);
    let rogue = f.base.context.create_cell(
        CellOutput::new_builder()
            .capacity(300 * CKB)
            .lock(f.base.layout.state_type.clone())
            .build(),
        Bytes::new(),
    );
    let tx = tx.as_advanced_builder().input(input(rogue)).build();
    f.assert_error(tx, Error::Authorization);
}

#[test]
fn rejects_unrelated_xudt_mint_via_public_state_or_reserve_lock() {
    for reserve_owner in [false, true] {
        let mut f = Fixture::new(true);
        let before = f.initial();
        let action = Action::Deposit(100);
        let tx = f.update(&before, &before.apply(&action).unwrap(), &action, true, 0);
        let owner = ckb_hash(if reserve_owner {
            f.base.layout.reserve_lock.as_slice()
        } else {
            f.base.layout.state_lock.as_slice()
        });
        let mut args = owner.to_vec();
        args.extend_from_slice(&[0; 4]);
        let rogue_type = f
            .base
            .layout
            .token_type
            .clone()
            .as_builder()
            .args(args.pack())
            .build();
        let rogue_output = CellOutput::new_builder()
            .capacity(200 * CKB)
            .lock(f.other.clone())
            .type_(Some(rogue_type).pack())
            .build();
        let tx = tx
            .as_advanced_builder()
            .output(rogue_output)
            .output_data(amount(1).pack())
            .build();
        f.assert_error(tx, Error::Token);
    }
}
