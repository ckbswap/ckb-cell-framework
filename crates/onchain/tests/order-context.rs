use ckb_contract_core::schema::FieldKind;
use ckb_contract_onchain::context::Cells;
use framework_limit_order_script::cells::OrderCells;

#[test]
fn order_declares_generic_cells_and_explicit_protocol_checks() {
    let schema = OrderCells::schema();
    assert!(!schema.reject_standard_mint);
    assert!(!schema.single_state);
    assert_eq!(schema.state_layout, None);
    assert!(schema.reserve_layouts.is_empty());
    assert_eq!(schema.fields.len(), 4);
    for field in &schema.fields[..4] {
        assert_eq!(field.kind, FieldKind::Cell);
        assert!(field.constraints.iter().any(|c| c.name == "source"));
        assert!(field.constraints.iter().any(|c| c.name == "capacity"));
    }
    assert!(schema.constraints.is_empty());
    assert!(schema
        .fields
        .iter()
        .flat_map(|field| field.constraints)
        .all(
            |constraint| !constraint.expression.contains("verify_protocol")
                && !constraint.expression.contains("require_protocol")
        ));
}
