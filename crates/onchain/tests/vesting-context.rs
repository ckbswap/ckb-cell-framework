use ckb_contract_core::schema::FieldKind;
use ckb_contract_onchain::context::Cells;
use framework_vesting_script::VestingCells;

#[test]
fn vesting_declares_optional_typed_cells_without_a_framework_state_policy() {
    let schema = VestingCells::schema();
    assert!(!schema.reject_standard_mint);
    assert!(!schema.single_state);
    assert_eq!(schema.state_layout, None);
    assert!(schema.reserve_layouts.is_empty());
    assert_eq!(
        schema.fields.iter().map(|f| f.name).collect::<Vec<_>>(),
        ["before", "after", "reserve_before", "reserve_after"]
    );
    assert!(schema.fields[..4]
        .iter()
        .all(|field| field.kind == FieldKind::Cell));
    assert!(schema.constraints.is_empty());
}

#[test]
fn vesting_schema_marks_optional_cells() {
    let schema = VestingCells::schema();
    assert!(schema.fields[..4].iter().all(|field| field
        .constraints
        .iter()
        .any(|constraint| constraint.name == "optional" && constraint.expression == "true")));
}
