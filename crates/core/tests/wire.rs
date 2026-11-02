use ckb_contract_core::{
    wire::{CodecSchema, WireType},
    BinaryCodec, Error,
};
use ckb_contract_derive::CellCodec;

#[derive(CellCodec, Debug, PartialEq)]
struct Nested {
    flag: bool,
    bytes: [u8; 2],
}

#[derive(CellCodec, Debug, PartialEq)]
#[codec(prefix = b"TEST")]
struct Record {
    small: u8,
    count: u32,
    time: u64,
    amount: u128,
    nested: Nested,
}

#[derive(CellCodec, Debug, PartialEq)]
#[codec(prefix = b"ACT")]
enum Action {
    #[codec(tag = 7)]
    Set { value: Record },
    #[codec(tag = 2)]
    Close,
    #[codec(tag = 3)]
    Write { writer: u8, reader: u8 },
}

#[test]
fn derived_wire_encoding_is_little_endian_nested_and_strict() {
    let record = Record {
        small: 1,
        count: 0x04030201,
        time: 5,
        amount: u128::MAX,
        nested: Nested {
            flag: true,
            bytes: [8, 9],
        },
    };
    let mut expected = b"TEST\x01\x01\x02\x03\x04\x05\0\0\0\0\0\0\0".to_vec();
    expected.extend_from_slice(&[255; 16]);
    expected.extend_from_slice(&[1, 8, 9]);
    assert_eq!(record.encode(), expected);
    assert_eq!(Record::decode(&expected).unwrap(), record);
    for len in 0..expected.len() {
        assert_eq!(Record::decode(&expected[..len]), Err(Error::Encoding));
    }
    let mut invalid = expected.clone();
    invalid[33] = 2;
    assert_eq!(Record::decode(&invalid), Err(Error::Encoding));
    invalid = expected.clone();
    invalid.push(0);
    assert_eq!(Record::decode(&invalid), Err(Error::Encoding));
    assert_eq!(Record::SCHEMA.prefix, b"TEST");
    let WireType::Struct { fields, .. } = Record::SCHEMA.body else {
        panic!("missing struct schema")
    };
    assert_eq!(fields[3].name, "amount");
    assert_eq!(fields[3].ty, WireType::U128);

    let action = Action::Set { value: record };
    // Nested records inline their body; only the Action's top-level prefix appears.
    let mut action_bytes = b"ACT\x07".to_vec();
    action_bytes.extend_from_slice(&expected[4..]);
    assert_eq!(action.encode(), action_bytes);
    assert_eq!(Action::decode(&action_bytes).unwrap(), action);
    assert_eq!(Action::Close.encode(), b"ACT\x02");
    assert_eq!(Action::decode(b"ACT\x00"), Err(Error::Encoding));
    let writer_fields = Action::Write {
        writer: 4,
        reader: 5,
    };
    assert_eq!(writer_fields.encode(), b"ACT\x03\x04\x05");
    assert_eq!(Action::decode(b"ACT\x03\x04\x05").unwrap(), writer_fields);
}

#[derive(CellCodec, Debug, PartialEq)]
#[codec(prefix = b"NEST")]
struct Payload {
    writer: bool,
    reader: [u8; 2],
}

#[derive(CellCodec, Debug, PartialEq)]
#[codec(prefix = b"TUP")]
enum TupleAction {
    #[codec(tag = 7)]
    Move(u64, Payload),
    #[codec(tag = 8)]
    EmptyTuple(),
    #[codec(tag = 9)]
    EmptyNamed {},
    #[codec(tag = 10)]
    Close,
}

#[test]
fn tuple_variants_inline_nested_values_and_reject_invalid_encodings() {
    let action = TupleAction::Move(
        0x0102_0304_0506_0708,
        Payload {
            writer: false,
            reader: [0x11, 0x22],
        },
    );
    // The nested Payload prefix is excluded; its body follows the u64.
    let expected = b"TUP\x07\x08\x07\x06\x05\x04\x03\x02\x01\x00\x11\x22";
    assert_eq!(action.encode(), expected);
    assert_eq!(TupleAction::decode(expected), Ok(action));
    for len in 0..expected.len() {
        assert_eq!(TupleAction::decode(&expected[..len]), Err(Error::Encoding));
    }
    for (index, byte) in [(3, 255), (12, 2)] {
        let mut invalid = expected.to_vec();
        invalid[index] = byte;
        assert_eq!(TupleAction::decode(&invalid), Err(Error::Encoding));
    }
    let mut trailing = expected.to_vec();
    trailing.push(0);
    assert_eq!(TupleAction::decode(&trailing), Err(Error::Encoding));
    for (action, bytes) in [
        (TupleAction::EmptyTuple(), b"TUP\x08"),
        (TupleAction::EmptyNamed {}, b"TUP\x09"),
        (TupleAction::Close, b"TUP\x0a"),
    ] {
        assert_eq!(action.encode(), bytes);
        assert_eq!(TupleAction::decode(bytes), Ok(action));
    }
    let WireType::Enum { variants, .. } = TupleAction::SCHEMA.body else {
        panic!("missing enum schema")
    };
    assert_eq!(variants[0].fields[0].name, "0");
    assert_eq!(variants[0].fields[0].ty, WireType::U64);
    assert_eq!(variants[0].fields[1].name, "1");
    assert!(matches!(
        variants[0].fields[1].ty,
        WireType::Struct {
            name: "Payload",
            ..
        }
    ));
}
