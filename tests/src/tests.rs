use ckb_contract_core::{entity_id, BinaryCodec, Error, StateTransition, CKB};
use ckb_contract_testkit::{
    self as kit,
    ckb_testtool::{
        ckb_types::{
            bytes::Bytes,
            core::{ScriptHashType, TransactionBuilder, TransactionView},
            packed::{CellInput, CellOutput, OutPoint, Script, WitnessArgs},
            prelude::*,
        },
        context::Context,
    },
};
use framework_counter_script::{Action, State, STATE_CAPACITY};

struct Fixture {
    context: Context,
    owner: Script,
    other: Script,
    type_script: Script,
    funding: OutPoint,
}

impl Fixture {
    fn new() -> Self {
        let root = kit::workspace_root(env!("CARGO_MANIFEST_DIR"));
        let mut context = Context::default();
        let code = context.deploy_cell(kit::load_binary(&root, "framework-counter-script"));
        let [owner, other, _] = kit::wallets(&mut context);
        let funding = context.create_cell(Self::output(&owner, None, 1000 * CKB), Bytes::new());
        let first = Self::input(funding.clone());
        // This example uses a raw ID, without a role byte.
        let id = entity_id(first.as_slice().try_into().unwrap(), 1);
        let type_script = context
            .build_script_with_hash_type(&code, ScriptHashType::Data2, Bytes::from(id.to_vec()))
            .unwrap();
        Self {
            context,
            owner,
            other,
            type_script,
            funding,
        }
    }

    fn input(point: OutPoint) -> CellInput {
        CellInput::new_builder().previous_output(point).build()
    }

    fn output(lock: &Script, type_script: Option<&Script>, capacity: u64) -> CellOutput {
        CellOutput::new_builder()
            .capacity(capacity)
            .lock(lock.clone())
            .type_(type_script.cloned().pack())
            .build()
    }

    fn state(&self) -> State {
        State::new(self.owner.calc_script_hash().unpack())
    }

    fn creation(&self) -> TransactionView {
        TransactionBuilder::default()
            .input(Self::input(self.funding.clone()))
            .output(Self::output(&self.owner, None, 699 * CKB))
            .output(Self::output(
                &self.owner,
                Some(&self.type_script),
                STATE_CAPACITY,
            ))
            .outputs_data([Bytes::new(), Bytes::from(self.state().encode())].pack())
            .build()
    }

    fn update(&mut self, state_index: usize) -> TransactionView {
        let before = self.state();
        let action = Action { increment: 7 };
        self.update_state(
            &before,
            &before.apply(&action).unwrap(),
            &action.encode(),
            state_index,
        )
    }

    fn update_state(
        &mut self,
        before: &State,
        after: &State,
        action: &[u8],
        state_index: usize,
    ) -> TransactionView {
        let state = self.context.create_cell(
            Self::output(&self.owner, Some(&self.type_script), STATE_CAPACITY),
            Bytes::from(before.encode()),
        );
        self.update_from(state, after, action, state_index)
    }

    fn update_from(
        &mut self,
        state: OutPoint,
        after: &State,
        action: &[u8],
        state_index: usize,
    ) -> TransactionView {
        let funding = self
            .context
            .create_cell(Self::output(&self.owner, None, 500 * CKB), Bytes::new());
        let mut inputs = vec![Self::input(funding)];
        inputs.insert(state_index, Self::input(state));
        let mut witnesses = vec![Bytes::new(); 2];
        witnesses[state_index] = WitnessArgs::new_builder()
            .input_type(Some(Bytes::from(action.to_vec())).pack())
            .build()
            .as_bytes();
        TransactionBuilder::default()
            .inputs(inputs)
            .output(Self::output(&self.owner, None, 499 * CKB))
            .output(Self::output(
                &self.owner,
                Some(&self.type_script),
                STATE_CAPACITY,
            ))
            .outputs_data([Bytes::new(), Bytes::from(after.encode())].pack())
            .witnesses(witnesses.pack())
            .build()
    }
}

#[test]
fn counter_creates_and_updates_without_roles_or_fixed_positions() {
    let mut f = Fixture::new();
    let creation = f.creation();
    kit::assert_valid(&mut f.context, creation, 30_000_000);
    for input in [0, 1] {
        let tx = f.update(input);
        kit::assert_valid(&mut f.context, tx.clone(), 30_000_000);
        // Output ordering is not part of this application's update policy.
        let mut outputs: Vec<_> = tx.outputs().into_iter().collect();
        let mut data: Vec<_> = tx.outputs_data().into_iter().collect();
        outputs.swap(0, 1);
        data.swap(0, 1);
        kit::assert_valid(
            &mut f.context,
            tx.as_advanced_builder()
                .set_outputs(outputs)
                .set_outputs_data(data)
                .build(),
            30_000_000,
        );
    }
}

#[test]
fn counter_rejects_wrong_identity_authority_lock_and_genesis() {
    let mut f = Fixture::new();
    let tx = f.creation();
    let output = tx.outputs().get(1).unwrap();
    let wrong_lock = output.clone().as_builder().lock(f.other.clone()).build();
    let changed = tx
        .clone()
        .as_advanced_builder()
        .set_outputs(vec![tx.outputs().get(0).unwrap(), wrong_lock])
        .build();
    kit::assert_error(&mut f.context, changed, Error::Structure, 30_000_000);
    let bad_id = f
        .type_script
        .clone()
        .as_builder()
        .args(Bytes::from(vec![9; 32]).pack())
        .build();
    let changed = tx
        .clone()
        .as_advanced_builder()
        .set_outputs(vec![
            tx.outputs().get(0).unwrap(),
            output
                .clone()
                .as_builder()
                .type_(Some(bad_id).pack())
                .build(),
        ])
        .build();
    kit::assert_error(&mut f.context, changed, Error::Genesis, 30_000_000);
    let mut invalid = f.state();
    invalid.value = 1;
    let changed = tx
        .as_advanced_builder()
        .set_outputs_data(vec![
            Bytes::new().pack(),
            Bytes::from(invalid.encode()).pack(),
        ])
        .build();
    kit::assert_error(&mut f.context, changed, Error::Genesis, 30_000_000);
}

#[test]
fn counter_rejects_capacity_output_tampering_duplicates_and_close() {
    let mut f = Fixture::new();
    let tx = f.update(1);
    let output = tx.outputs().get(1).unwrap();
    let changed = tx
        .clone()
        .as_advanced_builder()
        .set_outputs(vec![
            tx.outputs().get(0).unwrap(),
            output
                .clone()
                .as_builder()
                .capacity(STATE_CAPACITY + CKB)
                .build(),
        ])
        .build();
    kit::assert_error(&mut f.context, changed, Error::Capacity, 30_000_000);
    let changed = tx
        .clone()
        .as_advanced_builder()
        .set_outputs_data(vec![
            Bytes::new().pack(),
            Bytes::from(f.state().encode()).pack(),
        ])
        .build();
    kit::assert_error(&mut f.context, changed, Error::Transition, 30_000_000);
    let mut duplicate = f.state();
    duplicate.value = 99;
    let changed = tx
        .clone()
        .as_advanced_builder()
        .output(output.as_builder().capacity(STATE_CAPACITY - CKB).build())
        .output_data(Bytes::from(duplicate.encode()).pack())
        .build();
    // A duplicate with wrong capacity/data must still fail unique selection.
    kit::assert_error(&mut f.context, changed, Error::Structure, 30_000_000);
    let changed = tx
        .as_advanced_builder()
        .set_outputs(vec![Fixture::output(&f.owner, None, 799 * CKB)])
        .set_outputs_data(vec![Bytes::new().pack()])
        .build();
    kit::assert_error(&mut f.context, changed, Error::Structure, 30_000_000);
}

#[test]
fn counter_rejects_malformed_instance_ids() {
    for len in [0, 31, 33, 65] {
        let mut f = Fixture::new();
        let tx = f.creation();
        let wrong = f
            .type_script
            .clone()
            .as_builder()
            .args(Bytes::from(vec![1; len]).pack())
            .build();
        let output = tx
            .outputs()
            .get(1)
            .unwrap()
            .as_builder()
            .type_(Some(wrong).pack())
            .build();
        let tx = tx
            .as_advanced_builder()
            .set_outputs(vec![Fixture::output(&f.owner, None, 699 * CKB), output])
            .build();
        kit::assert_error(&mut f.context, tx, Error::Encoding, 30_000_000);
    }
}

#[test]
fn counter_requires_the_owner_to_authorize_creation() {
    let mut f = Fixture::new();
    let tx = f.creation();
    let state = State::new(f.other.calc_script_hash().unpack());
    let output = Fixture::output(&f.other, Some(&f.type_script), STATE_CAPACITY);
    let tx = tx
        .as_advanced_builder()
        .set_outputs(vec![Fixture::output(&f.owner, None, 699 * CKB), output])
        .set_outputs_data(vec![
            Bytes::new().pack(),
            Bytes::from(state.encode()).pack(),
        ])
        .build();
    kit::assert_error(&mut f.context, tx, Error::Authorization, 30_000_000);
}

#[test]
fn counter_creation_id_binds_the_first_input_and_actual_output_position() {
    for change_order in [false, true] {
        let mut f = Fixture::new();
        let extra = f
            .context
            .create_cell(Fixture::output(&f.owner, None, 100 * CKB), Bytes::new());
        let tx = f
            .creation()
            .as_advanced_builder()
            .input(Fixture::input(extra))
            .build();
        kit::assert_valid(&mut f.context, tx.clone(), 30_000_000);
        let mut inputs: Vec<_> = tx.inputs().into_iter().collect();
        if change_order {
            inputs.swap(0, 1);
        } else {
            inputs[0] = inputs[0].clone().as_builder().since(1u64).build();
        }
        kit::assert_error(
            &mut f.context,
            tx.as_advanced_builder().set_inputs(inputs).build(),
            Error::Genesis,
            30_000_000,
        );
    }
}

#[test]
fn counter_updates_committed_predecessors_after_outputs_move() {
    let mut f = Fixture::new();
    let creation = f.creation();
    let mut tx = kit::assert_valid(&mut f.context, creation, 30_000_000);
    kit::commit_outputs(&mut f.context, &tx);
    let mut position = 1;
    let mut state = f.state();
    for (state_index, increment) in [(1, 1), (0, 7), (1, 42)] {
        let previous = OutPoint::new_builder()
            .tx_hash(tx.hash())
            .index(position as u32)
            .build();
        let action = Action { increment };
        let after = state.apply(&action).unwrap();
        let update = f.update_from(previous.clone(), &after, &action.encode(), state_index);
        let mut outputs: Vec<_> = update.outputs().into_iter().collect();
        let mut data: Vec<_> = update.outputs_data().into_iter().collect();
        outputs.swap(0, 1);
        data.swap(0, 1);
        tx = kit::assert_valid(
            &mut f.context,
            update
                .as_advanced_builder()
                .set_outputs(outputs)
                .set_outputs_data(data)
                .build(),
            30_000_000,
        );
        position = 0;
        kit::commit_outputs(&mut f.context, &tx);
        assert!(f.context.get_cell(&previous).is_none());
        state = State::decode(&tx.outputs_data().get(position).unwrap().raw_data()).unwrap();
        assert_eq!(state, after);
    }
    assert_eq!((state.nonce, state.value), (3, 50));
}

#[test]
fn counter_checks_integer_boundaries_and_tampered_state_fields() {
    let mut f = Fixture::new();
    let before = f.state();
    let action = Action { increment: 1 };
    let expected = before.apply(&action).unwrap();
    for field in 0..3 {
        let mut forged = expected.clone();
        match field {
            0 => forged.owner = [9; 32],
            1 => forged.nonce += 1,
            _ => forged.value += 1,
        }
        let tx = f.update_state(&before, &forged, &action.encode(), 1);
        kit::assert_error(&mut f.context, tx, Error::Transition, 30_000_000);
    }
    for overflow_nonce in [false, true] {
        let before = State {
            owner: before.owner,
            nonce: if overflow_nonce { u64::MAX - 1 } else { 0 },
            value: if overflow_nonce { 0 } else { u64::MAX - 1 },
        };
        let limit = before.apply(&action).unwrap();
        let tx = f.update_state(&before, &limit, &action.encode(), 0);
        kit::assert_valid(&mut f.context, tx, 30_000_000);
        let tx = f.update_state(&limit, &limit, &action.encode(), 1);
        kit::assert_error(&mut f.context, tx, Error::Arithmetic, 30_000_000);
    }
}

#[test]
fn counter_rejects_malformed_data_and_wrong_action_witnesses() {
    let mut f = Fixture::new();
    for input_side in [false, true] {
        for malformed in 0..3 {
            let tx = f.update(1);
            let mut data = if input_side {
                f.state().encode()
            } else {
                tx.outputs_data().get(1).unwrap().raw_data().to_vec()
            };
            match malformed {
                0 => {
                    data.pop();
                }
                1 => data.push(0),
                _ => data[0] = 0,
            }
            let tx = if input_side {
                let point = tx.inputs().get(1).unwrap().previous_output();
                let (output, _) = f.context.get_cell(&point).unwrap();
                f.context
                    .create_cell_with_out_point(point, output, Bytes::from(data));
                tx
            } else {
                kit::replace_data(tx, 1, Bytes::from(data))
            };
            kit::assert_error(&mut f.context, tx, Error::Encoding, 30_000_000);
        }
    }
    for action in [vec![1; 7], vec![1; 9], vec![0; 8]] {
        let before = f.state();
        let tx = f.update_state(&before, &before, &action, 1);
        let error = if action.len() == 8 {
            Error::Transition
        } else {
            Error::Encoding
        };
        kit::assert_error(&mut f.context, tx, error, 30_000_000);
    }
    for empty in [Bytes::new(), WitnessArgs::default().as_bytes()] {
        let tx = f.update(1);
        let mut witnesses: Vec<_> = tx.witnesses().into_iter().collect();
        witnesses[0] = empty.pack();
        witnesses.swap(0, 1);
        let error = if empty.is_empty() {
            Error::Structure
        } else {
            Error::Encoding
        };
        kit::assert_error(
            &mut f.context,
            tx.as_advanced_builder().set_witnesses(witnesses).build(),
            error,
            30_000_000,
        );
    }
}

#[test]
fn counter_rejects_its_type_script_used_as_an_input_lock() {
    let mut f = Fixture::new();
    let extra = f.context.create_cell(
        Fixture::output(&f.type_script, None, 100 * CKB),
        Bytes::new(),
    );
    let tx = f
        .update(0)
        .as_advanced_builder()
        .input(Fixture::input(extra))
        .build();
    kit::assert_error(&mut f.context, tx, Error::Authorization, 30_000_000);
}

#[test]
fn counter_rejects_unrelated_official_token_minting_including_type_hash_aliases() {
    use ckb_contract_core::{ckb_hash, token::XUDT_CODE_HASH};
    for alias in [false, true] {
        let mut f = Fixture::new();
        let root = kit::workspace_root(env!("CARGO_MANIFEST_DIR"));
        let binary = std::fs::read(root.join("vendor/xudt")).unwrap();
        assert_eq!(ckb_hash(&binary), XUDT_CODE_HASH);
        let code = f.context.deploy_cell(Bytes::from(binary));
        let mut args = f.owner.calc_script_hash().raw_data().to_vec();
        args.extend_from_slice(&[0; 4]);
        let token = if alias {
            f.context
                .build_script_with_hash_type(&code, ScriptHashType::Type, Bytes::from(args))
                .unwrap()
        } else {
            Script::new_builder()
                .code_hash(XUDT_CODE_HASH.pack())
                .hash_type(ScriptHashType::Data1)
                .args(args.pack())
                .build()
        };
        let tx = f
            .update(0)
            .as_advanced_builder()
            .output(Fixture::output(&f.owner, Some(&token), 200 * CKB))
            .output_data(Bytes::from(1u128.to_le_bytes().to_vec()).pack())
            .build();
        kit::assert_error(&mut f.context, tx, Error::Token, 30_000_000);
    }
}
