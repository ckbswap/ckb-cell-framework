// Fixed protocol vectors, independent of CellCodec and its generated schema.
use ckb_contract_core::{token::PlainXudtAsset, wire::CodecSchema, BinaryCodec};

use framework_limit_order_script::{Action, State};

fn bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(core::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn derived_order_codecs_match_protocol_vectors() {
    for sell_extended in [false, true] {
        for buy_extended in [false, true] {
            for cancelled in [false, true] {
                let state = State {
                    maker: [0x11; 32],
                    sell_asset: PlainXudtAsset {
                        owner: [0x22; 32],
                        extended_args: sell_extended,
                    },
                    buy_asset: PlainXudtAsset {
                        owner: [0x33; 32],
                        extended_args: buy_extended,
                    },
                    sell_total: 1000,
                    buy_total: 2500,
                    filled: 321,
                    cancelled,
                };
                let mut expected = bytes("4c4f5230311111111111111111111111111111111111111111111111111111111111111111222222222222222222222222222222222222222222222222222222222222222200333333333333333333333333333333333333333333333333333333333333333300e8030000000000000000000000000000c40900000000000000000000000000004101000000000000000000000000000000");
                expected[69] = u8::from(sell_extended);
                expected[102] = u8::from(buy_extended);
                expected[151] = u8::from(cancelled);
                assert_eq!(state.encode(), expected);
                assert_eq!(State::decode(&expected), Ok(state));
            }
        }
    }
    for (action, expected) in [
        (Action::Fill { taker: [0x44; 32], amount: 777 }, bytes("4c4f41303100444444444444444444444444444444444444444444444444444444444444444409030000000000000000000000000000")),
        (Action::Cancel, b"LOA01\x01".to_vec()), (Action::Close, b"LOA01\x02".to_vec()),
    ] {
        assert_eq!(action.encode(), expected);
        assert_eq!(Action::decode(&expected), Ok(action));
    }
    assert_eq!(State::SCHEMA.prefix, b"LOR01");
    assert_eq!(Action::SCHEMA.prefix, b"LOA01");
}
