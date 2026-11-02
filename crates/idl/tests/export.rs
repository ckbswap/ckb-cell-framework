use ckb_contract_core::{
    schema::{Constraint, ContextSchema, FieldKind, FieldSchema},
    wire::{CodecSchema, WireField, WireType, WireVariant},
    BinaryCodec,
};
use ckb_contract_idl::{export, Error, Instruction, Interface, Phase};
use framework_vesting_script::{Action, Schedule, State, LAYOUT};
use serde_json::{json, Value};

fn interface() -> Interface<'static> {
    Interface {
        name: "vesting",
        version: "0.1.0",
        state: State::SCHEMA,
        action: Action::SCHEMA,
        layout: LAYOUT,
        bindings: None,
        context: ContextSchema {
            version: 1,
            name: "VestingCells",
            reject_standard_mint: true,
            single_state: true,
            state_layout: Some(LAYOUT.state),
            constraints: &[],
            reserve_layouts: LAYOUT.reserves.to_vec(),
            fields: &[
                FieldSchema {
                    name: "grant",
                    kind: FieldKind::State,
                    constraints: &[Constraint {
                        name: "update_timestamp",
                        expression: "claim_timestamp",
                    }],
                },
                FieldSchema {
                    name: "actor",
                    kind: FieldKind::Authority,
                    constraints: &[],
                },
                FieldSchema {
                    name: "reserve",
                    kind: FieldKind::Reserve,
                    constraints: &[],
                },
            ],
        },
        instructions: &[
            Instruction {
                name: "create",
                phase: Phase::Create,
                action: None,
                authority: "funder",
                timestamp: None,
            },
            Instruction {
                name: "claim",
                phase: Phase::Update,
                action: Some("Claim"),
                authority: "beneficiary",
                timestamp: Some("timestamp"),
            },
            Instruction {
                name: "close",
                phase: Phase::Close,
                action: Some("Close"),
                authority: "funder",
                timestamp: None,
            },
        ],
    }
}

// Independent interpreter consumes only exported JSON and actual codec bytes.
fn decode(ty: &Value, data: &[u8], position: &mut usize) -> Value {
    match ty["kind"].as_str().unwrap() {
        "struct" => decode_fields(&ty["fields"], data, position),
        "enum" => {
            let start = *position;
            let tag = data[*position];
            *position += 1;
            let variant = ty["variants"]
                .as_array()
                .unwrap()
                .iter()
                .find(|variant| variant["tag"] == tag)
                .unwrap();
            let mut object = serde_json::Map::new();
            object.insert("variant".into(), variant["name"].clone());
            for field in variant["fields"].as_array().unwrap() {
                assert_eq!(
                    *position - start,
                    field["offset"].as_u64().unwrap() as usize
                );
                object.insert(
                    field["name"].as_str().unwrap().into(),
                    decode(&field["type"], data, position),
                );
            }
            Value::Object(object)
        }
        kind => {
            let len = ty["byte_length"].as_u64().unwrap() as usize;
            let bytes = &data[*position..*position + len];
            *position += len;
            match kind {
                "bytes" => json!(bytes
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()),
                "bool" => {
                    assert!(bytes[0] <= 1);
                    json!(bytes[0] == 1)
                }
                _ => {
                    let mut value = [0; 16];
                    value[..len].copy_from_slice(bytes);
                    json!(u128::from_le_bytes(value).to_string())
                }
            }
        }
    }
}
fn decode_fields(fields: &Value, data: &[u8], position: &mut usize) -> Value {
    let start = *position;
    let mut object = serde_json::Map::new();
    for field in fields.as_array().unwrap() {
        assert_eq!(
            *position - start,
            field["offset"].as_u64().unwrap() as usize
        );
        object.insert(
            field["name"].as_str().unwrap().into(),
            decode(&field["type"], data, position),
        );
    }
    Value::Object(object)
}
fn decode_top(schema: &Value, data: &[u8]) -> Value {
    let prefix = schema["body_offset"].as_u64().unwrap() as usize;
    assert_eq!(
        data[..prefix]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        schema["prefix_hex"]
    );
    let mut position = prefix;
    let result = decode(&schema["body"], data, &mut position);
    assert_eq!(position, data.len());
    result
}

#[test]
fn exported_json_decodes_existing_vesting_bytes_including_nested_fields_and_u128_max() {
    let source = export(&interface()).unwrap();
    assert_eq!(source, export(&interface()).unwrap());
    assert!(source.ends_with('\n'));
    let idl: Value = serde_json::from_str(&source).unwrap();
    assert_eq!(idl["schema_version"], 1);
    assert_eq!(idl["state"]["byte_length"], 158);
    assert_eq!(idl["layout"]["state"]["capacity"], "40000000000");
    assert_eq!(
        idl["instructions"][1]["since"]["input"],
        "actual_state_input"
    );
    assert_eq!(
        idl["context"]["fields"][0]["constraints"][0]["language"],
        "rust"
    );
    for extended_args in [false, true] {
        let state = State::new(
            [1; 32],
            [2; 32],
            ckb_contract_core::token::PlainXudtAsset {
                owner: [3; 32],
                extended_args,
            },
            u128::MAX,
            Schedule {
                start: 1000,
                cliff: 1200,
                end: 2000,
            },
        )
        .unwrap();
        let decoded = decode_top(&idl["state"], &state.encode());
        assert_eq!(decoded["total"], u128::MAX.to_string());
        assert_eq!(decoded["asset"]["extended_args"], extended_args);
        assert_eq!(decoded["asset"]["owner"], "03".repeat(32));
        assert_eq!(decoded["schedule"]["cliff"], "1200");
    }
    let action = Action::Claim {
        timestamp: 2000,
        amount: u128::MAX,
    };
    assert_eq!(
        decode_top(&idl["action"], &action.encode()),
        json!({"variant":"Claim", "timestamp":"2000", "amount":u128::MAX.to_string()})
    );
    assert_eq!(
        decode_top(&idl["action"], &Action::Close.encode()),
        json!({"variant":"Close"})
    );
}

#[test]
fn rejects_invalid_action_authority_time_and_lifecycle_references() {
    for bad in [
        Instruction {
            name: "bad",
            phase: Phase::Update,
            action: Some("Unknown"),
            authority: "funder",
            timestamp: None,
        },
        Instruction {
            name: "bad",
            phase: Phase::Update,
            action: Some("Claim"),
            authority: "total",
            timestamp: None,
        },
        Instruction {
            name: "bad",
            phase: Phase::Update,
            action: Some("Claim"),
            authority: "funder",
            timestamp: Some("amount"),
        },
        Instruction {
            name: "bad",
            phase: Phase::Create,
            action: Some("Claim"),
            authority: "funder",
            timestamp: None,
        },
        Instruction {
            name: "bad",
            phase: Phase::Close,
            action: None,
            authority: "funder",
            timestamp: None,
        },
    ] {
        let instructions = [bad];
        assert!(matches!(
            export(&Interface {
                instructions: &instructions,
                ..interface()
            }),
            Err(Error::Invalid(_))
        ));
    }
}

#[test]
fn rejects_duplicate_wire_tags_recursive_schemas_and_length_overflow() {
    let mut fixture = interface();
    fixture.action.body = WireType::Enum {
        name: "Bad",
        variants: &[
            WireVariant {
                name: "A",
                tag: 0,
                fields: &[],
            },
            WireVariant {
                name: "B",
                tag: 0,
                fields: &[],
            },
        ],
    };
    assert!(matches!(
        export(&fixture),
        Err(Error::Invalid("duplicate enum tag"))
    ));
    static CYCLE: [WireField; 1] = [WireField {
        name: "next",
        ty: WireType::Struct {
            name: "Cycle",
            fields: &CYCLE,
        },
    }];
    fixture = interface();
    fixture.state.body = WireType::Struct {
        name: "Cycle",
        fields: &CYCLE,
    };
    assert!(matches!(
        export(&fixture),
        Err(Error::Invalid("wire schema nesting exceeds 64 levels"))
    ));
    fixture.state.body = WireType::Struct {
        name: "Huge",
        fields: &[WireField {
            name: "huge",
            ty: WireType::Bytes(usize::MAX),
        }],
    };
    assert!(matches!(
        export(&fixture),
        Err(Error::Invalid("wire byte length overflow"))
    ));
}

#[test]
fn rejects_layout_values_that_drift_from_context() {
    for state in [
        ckb_contract_core::layout::StateLayout {
            role: 9,
            ..LAYOUT.state
        },
        ckb_contract_core::layout::StateLayout {
            lock_role: 9,
            ..LAYOUT.state
        },
        ckb_contract_core::layout::StateLayout {
            capacity: LAYOUT.state.capacity + 1,
            ..LAYOUT.state
        },
        ckb_contract_core::layout::StateLayout {
            output: 5,
            ..LAYOUT.state
        },
    ] {
        let mut fixture = interface();
        fixture.layout.state = state;
        assert!(
            matches!(export(&fixture), Err(Error::Invalid(_))),
            "accepted {state:?}"
        );
    }
    for reserve in [
        ckb_contract_core::layout::ReserveLayout {
            role: 9,
            ..LAYOUT.reserves[0]
        },
        ckb_contract_core::layout::ReserveLayout {
            capacity: LAYOUT.reserves[0].capacity + 1,
            ..LAYOUT.reserves[0]
        },
        ckb_contract_core::layout::ReserveLayout {
            output: 5,
            ..LAYOUT.reserves[0]
        },
    ] {
        let reserves = [reserve];
        let fixture = Interface {
            layout: ckb_contract_core::layout::TransactionLayout {
                reserves: &reserves,
                ..LAYOUT
            },
            ..interface()
        };
        assert!(
            matches!(export(&fixture), Err(Error::Invalid(_))),
            "accepted {reserve:?}"
        );
    }
}

#[test]
fn rejects_context_layout_drift_and_preserves_full_capacity_precision() {
    let mut fixture = interface();
    fixture.layout.reserves = &[];
    assert!(matches!(export(&fixture), Err(Error::Invalid(_))));
    fixture = interface();
    fixture.layout.state.capacity = u64::MAX;
    fixture.context.state_layout.as_mut().unwrap().capacity = u64::MAX;
    let idl: Value = serde_json::from_str(&export(&fixture).unwrap()).unwrap();
    assert_eq!(idl["layout"]["state"]["capacity"], u64::MAX.to_string());
}

#[test]
fn generic_context_export_has_no_implicit_state_role_layout_or_witness() {
    let context = ContextSchema {
        version: 1,
        name: "Transfer",
        reject_standard_mint: false,
        single_state: false,
        state_layout: None,
        reserve_layouts: vec![],
        constraints: &[Constraint {
            name: "constraint",
            expression: "before.data().value == after.data().value",
        }],
        fields: &[FieldSchema {
            name: "before",
            kind: FieldKind::Cell,
            constraints: &[Constraint {
                name: "source",
                expression: "Source::Input",
            }],
        }],
    };
    let value: Value =
        serde_json::from_str(&ckb_contract_idl::export_context(&context).unwrap()).unwrap();
    assert_eq!(value["fields"][0]["kind"], "cell");
    assert_eq!(value["constraints"][0]["language"], "rust");
    for key in ["state", "action", "layout", "witness", "script_args"] {
        assert!(value.get(key).is_none());
    }
}

#[test]
fn generic_state_protocol_interfaces_require_explicit_cell_bindings() {
    use ckb_contract_idl::ContextBindings;
    let mut fixture = interface();
    fixture.context.state_layout = None;
    fixture.context.reserve_layouts.clear();
    fixture.context.single_state = false;
    fixture.context.fields = &[
        FieldSchema {
            name: "after",
            kind: FieldKind::Cell,
            constraints: &[],
        },
        FieldSchema {
            name: "reserve_after",
            kind: FieldKind::Cell,
            constraints: &[],
        },
    ];
    assert!(export(&fixture).is_err());
    fixture.bindings = Some(ContextBindings {
        state_output: "after",
        reserve_outputs: &["reserve_after"],
    });
    let value: Value = serde_json::from_str(&export(&fixture).unwrap()).unwrap();
    assert_eq!(value["layout_verification"], "explicit_context_bindings");
    assert_eq!(value["layout"]["state"]["name"], "after");
    assert_eq!(value["context"]["fields"][0]["kind"], "cell");
    fixture.bindings = Some(ContextBindings {
        state_output: "missing",
        reserve_outputs: &["reserve_after"],
    });
    assert!(export(&fixture).is_err());
    fixture.bindings = Some(ContextBindings {
        state_output: "after",
        reserve_outputs: &["after"],
    });
    assert!(export(&fixture).is_err());
}
