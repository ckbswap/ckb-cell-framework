use ckb_contract_core::{
    args::RoleArgs,
    layout::{ReserveLayout, StateLayout, TransactionLayout},
    since, BinaryCodec, Error,
};
use ckb_contract_tx::ckb_types::{
    bytes::Bytes,
    packed::{OutPoint, Script, WitnessArgs},
    prelude::*,
};
use ckb_contract_tx::{
    context::{CellSet, ContextTx, ResolvedLayout},
    input, BuildError, Cell,
};

const LAYOUT: TransactionLayout<'static> = TransactionLayout {
    state: StateLayout {
        role: 0,
        lock_role: 1,
        capacity: 400,
        output: 3,
    },
    reserves: &[
        ReserveLayout {
            role: 2,
            capacity: 200,
            output: 0,
        },
        ReserveLayout {
            role: 3,
            capacity: 300,
            output: 5,
        },
    ],
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

fn script(tag: u8) -> Script {
    Script::new_builder()
        .code_hash([tag; 32].pack())
        .hash_type(4u8)
        .build()
}
fn layout() -> ResolvedLayout {
    ResolvedLayout::new(&LAYOUT, &script(9), &[8; 32], &[script(4), script(5)]).unwrap()
}
fn cells() -> CellSet {
    CellSet::at(&LAYOUT, [7; 32].pack()).unwrap()
}
fn funding(index: u32) -> OutPoint {
    OutPoint::new_builder()
        .tx_hash([6; 32].pack())
        .index(index)
        .build()
}

#[test]
fn sparse_outputs_and_multiple_reserves_follow_shared_positions_and_actual_state_input() {
    let layout = layout();
    let points = cells();
    for index in 0..=2 {
        let timestamp = since::absolute_timestamp(2000).unwrap();
        let tx = ContextTx::update(&layout, points.clone(), &Data(9), &[123, 456], &Data(7))
            .unwrap()
            .input(input(funding(0)))
            .input(input(funding(1)))
            .state_input_index(index)
            .state_since(timestamp)
            .output(Cell::capacity(&script(6), 111))
            .output(Cell::new(
                Cell::capacity(&script(6), 222).output,
                Bytes::from_static(b"extra"),
            ))
            .output(Cell::capacity(&script(6), 333))
            .build()
            .unwrap();
        assert_eq!(
            tx.inputs().get(index).unwrap().previous_output(),
            points.state
        );
        assert_eq!(
            u64::from(tx.inputs().get(index).unwrap().since()),
            timestamp
        );
        for reserve in 0..2 {
            assert_eq!(
                tx.inputs()
                    .get(index + reserve + 1)
                    .unwrap()
                    .previous_output(),
                points.reserves[reserve]
            );
            assert_eq!(
                u64::from(tx.inputs().get(index + reserve + 1).unwrap().since()),
                0
            );
        }
        let witness =
            WitnessArgs::from_slice(&tx.witnesses().get(index).unwrap().raw_data()).unwrap();
        assert_eq!(
            witness.input_type().to_opt().unwrap().raw_data().as_ref(),
            &[7]
        );
        assert_eq!(tx.outputs().get(3).unwrap(), layout.state_output());
        assert_eq!(tx.outputs_data().get(3).unwrap().raw_data().as_ref(), &[9]);
        for (reserve, amount) in [123, 456].into_iter().enumerate() {
            let position = LAYOUT.reserves[reserve].output;
            let expected = layout.reserve(reserve, amount).unwrap();
            assert_eq!(tx.outputs().get(position).unwrap(), expected.output);
            assert_eq!(
                tx.outputs_data().get(position).unwrap().raw_data(),
                expected.data
            );
        }
        assert_eq!(u64::from(tx.outputs().get(1).unwrap().capacity()), 111);
        assert_eq!(
            tx.outputs_data().get(2).unwrap().raw_data().as_ref(),
            b"extra"
        );
        assert_eq!(u64::from(tx.outputs().get(4).unwrap().capacity()), 333);
    }
    let args = layout.state_type().args().raw_data();
    assert_eq!(RoleArgs::decode(&args).unwrap().id, [8; 32]);
    assert_eq!(layout.state_type().code_hash(), script(9).code_hash());
    assert_eq!(layout.state_type().hash_type(), script(9).hash_type());
}

#[test]
fn create_close_and_state_only_contexts_have_explicit_lifecycles() {
    let layout = layout();
    let tx = ContextTx::create(&layout, &Data(0), &[100, 200])
        .unwrap()
        .input(input(funding(0)))
        .output(Cell::capacity(&script(6), 100))
        .output(Cell::capacity(&script(6), 100))
        .output(Cell::capacity(&script(6), 100))
        .build()
        .unwrap();
    assert!(tx.witnesses().is_empty());
    assert_eq!(tx.outputs().len(), 6);
    let closed = ContextTx::close(&layout, cells(), &Data(1))
        .unwrap()
        .output(Cell::capacity(&script(6), 900))
        .build()
        .unwrap();
    assert_eq!(closed.inputs().len(), 3);
    assert_eq!(closed.outputs().len(), 1);

    let policy = TransactionLayout {
        state: StateLayout {
            output: 0,
            ..LAYOUT.state
        },
        reserves: &[],
    };
    let state_only = ResolvedLayout::new(&policy, &script(9), &[8; 32], &[]).unwrap();
    let tx = ContextTx::update(
        &state_only,
        CellSet::at(&policy, [7; 32].pack()).unwrap(),
        &Data(2),
        &[],
        &Data(1),
    )
    .unwrap()
    .build()
    .unwrap();
    assert_eq!(tx.inputs().len(), 1);
    assert_eq!(tx.outputs().len(), 1);
}

#[test]
fn rejects_wrong_reserve_counts_origins_positions_and_duplicate_inputs() {
    let layout = layout();
    assert!(matches!(
        ResolvedLayout::new(&LAYOUT, &script(9), &[0; 32], &[]),
        Err(BuildError::ReserveCount { .. })
    ));
    assert!(matches!(
        ContextTx::create(&layout, &Data(0), &[1]),
        Err(BuildError::ReserveCount { .. })
    ));
    assert!(matches!(
        layout.reserve(2, 1),
        Err(BuildError::ReserveIndex { .. })
    ));
    for mode in 0..4 {
        let mut bad = cells();
        match mode {
            0 => {
                bad.reserves[0] = bad.reserves[0]
                    .clone()
                    .as_builder()
                    .tx_hash([1; 32].pack())
                    .build()
            }
            1 => bad.reserves.swap(0, 1),
            2 => bad.state = bad.state.as_builder().index(2u32).build(),
            _ => bad.reserves[1] = bad.reserves[0].clone(),
        }
        assert!(matches!(
            ContextTx::close(&layout, bad, &Data(1)),
            Err(BuildError::InvalidCellSet)
        ));
    }
    let points = cells();
    let tx = ContextTx::close(&layout, points.clone(), &Data(1))
        .unwrap()
        .input(input(points.reserves[1].clone()))
        .output(Cell::capacity(&script(6), 1));
    assert!(matches!(tx.build(), Err(BuildError::DuplicateInput)));
}

#[test]
fn rejects_colliding_layouts_unfilled_output_gaps_and_extreme_indices_without_allocating_by_position(
) {
    let invalid = TransactionLayout {
        state: StateLayout {
            role: 1,
            ..LAYOUT.state
        },
        ..LAYOUT
    };
    assert!(matches!(
        ResolvedLayout::new(&invalid, &script(9), &[0; 32], &[script(4), script(5)]),
        Err(BuildError::Layout(Error::Encoding))
    ));
    let invalid = TransactionLayout {
        state: StateLayout {
            output: 0,
            ..LAYOUT.state
        },
        ..LAYOUT
    };
    assert_eq!(invalid.validate(), Err(Error::Structure));
    let layout = layout();
    assert!(matches!(
        ContextTx::create(&layout, &Data(0), &[1, 2])
            .unwrap()
            .input(input(funding(0)))
            .build(),
        Err(BuildError::OutputIndex { .. })
    ));
    let huge = TransactionLayout {
        state: StateLayout {
            output: u32::MAX as usize,
            ..LAYOUT.state
        },
        reserves: &[],
    };
    let layout = ResolvedLayout::new(&huge, &script(9), &[0; 32], &[]).unwrap();
    assert!(matches!(
        ContextTx::create(&layout, &Data(0), &[])
            .unwrap()
            .input(input(funding(0)))
            .build(),
        Err(BuildError::OutputIndex { .. })
    ));
}
