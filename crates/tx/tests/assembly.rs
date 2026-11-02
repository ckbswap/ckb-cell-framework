use ckb_contract_core::{since, BinaryCodec, Error};
use ckb_contract_tx::{
    action_witness,
    ckb_types::{
        bytes::Bytes,
        core::TransactionBuilder,
        packed::{OutPoint, Script, WitnessArgs},
        prelude::*,
    },
    edit, input, BuildError, Cell, CellPair, StateReserveLayout, StateReserveTx,
};

struct Data(u8);
impl BinaryCodec for Data {
    type Error = Error;

    fn encode(&self) -> Vec<u8> {
        vec![self.0]
    }

    fn decode(data: &[u8]) -> Result<Self, Error> {
        match data {
            [value] => Ok(Self(*value)),
            _ => Err(Error::Encoding),
        }
    }
}

fn point(index: u32) -> OutPoint {
    OutPoint::new_builder().index(index).build()
}

fn script(role: u8) -> Script {
    Script::new_builder().args([role].pack()).build()
}

fn layout() -> StateReserveLayout {
    StateReserveLayout {
        state_type: script(0),
        state_lock: script(1),
        reserve_lock: script(2),
        token_type: script(3),
        state_capacity: 400,
        reserve_capacity: 200,
    }
}

#[test]
fn moving_state_keeps_pair_since_action_and_output_data_bound() {
    let layout = layout();
    let pair = CellPair {
        state: point(10),
        reserve: point(11),
    };
    let since = since::absolute_timestamp(1700000000).unwrap();
    for index in 0..=2 {
        let tx = StateReserveTx::update(&layout, pair.clone(), &Data(9), 123, &Data(7))
            .input(input(point(20)))
            .input(input(point(21)))
            .state_input_index(index)
            .state_since(since)
            .output(Cell::token(&script(4), &layout.token_type, 300, 456))
            .output(Cell::capacity(&script(5), 100))
            .build()
            .unwrap();
        let inputs = tx.inputs();
        assert_eq!(inputs.get(index).unwrap().previous_output(), pair.state);
        assert_eq!(
            inputs.get(index + 1).unwrap().previous_output(),
            pair.reserve
        );
        assert_eq!(u64::from(inputs.get(index).unwrap().since()), since);
        assert_eq!(u64::from(inputs.get(index + 1).unwrap().since()), 0);
        assert_eq!(
            tx.witnesses().get(index).unwrap().raw_data(),
            action_witness(&Data(7))
        );
        for other in 0..inputs.len() {
            if other != index {
                assert!(tx.witnesses().get(other).unwrap().raw_data().is_empty());
            }
        }
        assert_eq!(tx.outputs().len(), tx.outputs_data().len());
        assert_eq!(tx.outputs().get(0).unwrap(), layout.state_output());
        assert_eq!(tx.outputs_data().get(0).unwrap().raw_data().as_ref(), &[9]);
        assert_eq!(
            tx.outputs_data().get(1).unwrap().raw_data().as_ref(),
            &123u128.to_le_bytes()
        );
        assert_eq!(
            tx.outputs_data().get(2).unwrap().raw_data().as_ref(),
            &456u128.to_le_bytes()
        );
        assert!(tx.outputs_data().get(3).unwrap().raw_data().is_empty());
    }
}

#[test]
fn create_and_close_have_distinct_lifecycles_and_reject_bad_structure() {
    let layout = layout();
    let pair = (point(0), point(1));
    assert!(matches!(
        StateReserveTx::create(&layout, &Data(0), 0).build(),
        Err(BuildError::NoInputs)
    ));
    assert!(matches!(
        StateReserveTx::create(&layout, &Data(0), 0)
            .input(input(point(2)))
            .state_since(1)
            .build(),
        Err(BuildError::MissingStateInput)
    ));
    assert!(matches!(
        StateReserveTx::close(pair.clone(), &Data(1)).build(),
        Err(BuildError::NoOutputs)
    ));
    assert!(matches!(
        StateReserveTx::close(pair.clone(), &Data(1))
            .state_input_index(1)
            .output(Cell::capacity(&script(9), 600))
            .build(),
        Err(BuildError::InputIndex { .. })
    ));
    assert!(matches!(
        StateReserveTx::close((point(0), point(0)), &Data(1))
            .output(Cell::capacity(&script(9), 600))
            .build(),
        Err(BuildError::DuplicatePair)
    ));
    let created = StateReserveTx::create(&layout, &Data(0), 100)
        .input(input(point(2)))
        .build()
        .unwrap();
    assert_eq!(created.inputs().len(), 1);
    assert!(created.witnesses().is_empty());
    let closed = StateReserveTx::close(pair, &Data(1))
        .output(Cell::capacity(&script(9), 600))
        .build()
        .unwrap();
    assert_eq!(closed.outputs().len(), 1);
    assert_eq!(closed.inputs().len(), 2);
}

#[test]
fn fixed_layout_rejects_duplicate_funding_state_and_reserve_inputs() {
    let layout = layout();
    let pair = CellPair {
        state: point(0),
        reserve: point(1),
    };
    assert!(matches!(
        StateReserveTx::create(&layout, &Data(0), 0)
            .input(input(point(2)))
            .input(input(point(2)).as_builder().since(1u64).build())
            .build(),
        Err(BuildError::DuplicateInput)
    ));
    for duplicate in [pair.state.clone(), pair.reserve.clone(), point(2)] {
        for close in [false, true] {
            let builder = if close {
                StateReserveTx::close(pair.clone(), &Data(1))
                    .output(Cell::capacity(&script(9), 600))
            } else {
                StateReserveTx::update(&layout, pair.clone(), &Data(1), 0, &Data(1))
            };
            assert!(matches!(
                builder
                    .input(input(point(2)))
                    .input(input(duplicate.clone()).as_builder().since(1u64).build())
                    .state_input_index(1)
                    .build(),
                Err(BuildError::DuplicateInput)
            ));
        }
    }
}

#[test]
fn setting_action_preserves_wallet_fields_and_extra_witnesses() {
    let existing = WitnessArgs::new_builder()
        .lock(Some(Bytes::from_static(b"signature placeholder")).pack())
        .input_type(Some(Bytes::from_static(b"old action")).pack())
        .output_type(Some(Bytes::from_static(b"output proof")).pack())
        .build();
    let tx = TransactionBuilder::default()
        .inputs([input(point(0)), input(point(1))])
        .witnesses(
            [
                Bytes::from_static(b"untouched"),
                existing.as_bytes(),
                Bytes::from_static(b"extra proof"),
            ]
            .pack(),
        )
        .build();
    let tx = edit::with_action(tx, 1, &Data(7)).unwrap();
    let args = WitnessArgs::from_slice(&tx.witnesses().get(1).unwrap().raw_data()).unwrap();
    assert_eq!(args.lock(), existing.lock());
    assert_eq!(args.output_type(), existing.output_type());
    assert_eq!(
        args.input_type().to_opt().unwrap().raw_data().as_ref(),
        &[7]
    );
    assert_eq!(
        tx.witnesses().get(0).unwrap().raw_data().as_ref(),
        b"untouched"
    );
    assert_eq!(
        tx.witnesses().get(2).unwrap().raw_data().as_ref(),
        b"extra proof"
    );
    assert_eq!(tx.witnesses().len(), 3);
}

#[test]
fn checked_edits_reject_bad_indices_malformed_witness_and_misaligned_data() {
    let tx = TransactionBuilder::default()
        .input(input(point(0)))
        .witness(Bytes::from_static(b"not WitnessArgs").pack())
        .build();
    assert!(matches!(
        edit::with_action(tx.clone(), 0, &Data(0)),
        Err(BuildError::InvalidWitness)
    ));
    assert!(matches!(
        edit::with_action(tx.clone(), 1, &Data(0)),
        Err(BuildError::InputIndex { .. })
    ));
    assert!(matches!(
        edit::replace_since(tx.clone(), 1, 0),
        Err(BuildError::InputIndex { .. })
    ));
    assert!(matches!(
        edit::replace_data(tx.clone(), 0, Bytes::new()),
        Err(BuildError::OutputIndex { .. })
    ));
    let misaligned = tx
        .as_advanced_builder()
        .output(Cell::capacity(&script(9), 100).output)
        .build();
    assert!(matches!(
        edit::replace_data(misaligned.clone(), 0, Bytes::new()),
        Err(BuildError::OutputDataMismatch)
    ));
    assert!(matches!(
        edit::replace_output(misaligned, 0, Cell::capacity(&script(9), 100).output),
        Err(BuildError::OutputDataMismatch)
    ));
}
