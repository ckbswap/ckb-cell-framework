use ckb_contract_core::since;
use ckb_contract_testkit::ckb_testtool::{
    ckb_types::{bytes::Bytes, core::TransactionBuilder, packed::Script, prelude::*},
    context::Context,
};
use ckb_contract_testkit::{commit_outputs, time, workspace_root};
use ckb_contract_tx::{input, Cell, CellPair};

#[test]
fn commit_removes_spent_inputs_and_preserves_actual_output_outpoints() {
    let mut context = Context::default();
    let owner = Script::default();
    let funding = context.create_cell(Cell::capacity(&owner, 1000).output, Bytes::new());
    let state = Cell::new(
        Cell::capacity(&owner, 400).output,
        Bytes::from_static(b"state"),
    );
    let reserve = Cell::new(
        Cell::capacity(&owner, 200).output,
        Bytes::from_static(b"reserve"),
    );
    let tx = TransactionBuilder::default()
        .input(input(funding.clone()))
        .outputs([state.output.clone(), reserve.output.clone()])
        .outputs_data([state.data.clone(), reserve.data.clone()].pack())
        .build();
    commit_outputs(&mut context, &tx);
    let pair = CellPair::at(tx.hash());
    assert!(context.get_cell(&funding).is_none());
    assert!(context
        .cells_by_data_hash
        .values()
        .all(|point| context.cells.contains_key(point)));
    assert!(context
        .cells_by_type_hash
        .values()
        .all(|point| context.cells.contains_key(point)));
    assert_eq!(
        context.get_cell(&pair.state).unwrap(),
        (state.output, state.data)
    );
    assert_eq!(
        context.get_cell(&pair.reserve).unwrap(),
        (reserve.output, reserve.data)
    );
}

#[test]
fn workspace_lookup_handles_a_scaffolded_nested_test_crate() {
    let root = workspace_root(env!("CARGO_MANIFEST_DIR"));
    assert_eq!(
        workspace_root(root.join("contracts/new-example/tests")),
        root
    );
}

#[test]
fn official_consensus_time_check_enforces_the_exact_median_boundary() {
    let mut context = Context::default();
    let point = context.create_cell(
        Cell::capacity(&Script::default(), 1000).output,
        Bytes::new(),
    );
    let tx = TransactionBuilder::default()
        .input(
            input(point)
                .as_builder()
                .since(since::absolute_timestamp(1000).unwrap())
                .build(),
        )
        .build();
    assert!(time::verify(&mut context, &tx, 999999).is_err());
    time::verify(&mut context, &tx, 1000000).unwrap();
}
