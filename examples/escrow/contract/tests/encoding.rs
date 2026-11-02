// Fixed protocol vectors, independent of CellCodec and its generated schema.
use ckb_contract_core::{token::PlainXudtAsset, wire::CodecSchema, BinaryCodec};

use framework_escrow_script::{Action, State, Status};

fn bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(core::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn derived_escrow_codecs_match_protocol_vectors() {
    for extended_args in [false, true] {
        for (tag, status) in [
            (0, Status::Funded),
            (1, Status::Released),
            (2, Status::Refunded),
        ] {
            let state = State {
                buyer: [0x11; 32],
                seller: [0x22; 32],
                asset: PlainXudtAsset {
                    owner: [0x33; 32],
                    extended_args,
                },
                amount: 0x1112_1314_1516_1718_191a_1b1c_1d1e_1f20,
                status,
            };
            let mut expected = bytes("455343303111111111111111111111111111111111111111111111111111111111111111112222222222222222222222222222222222222222222222222222222222222222333333333333333333333333333333333333333333333333333333333333333300201f1e1d1c1b1a19181716151413121100");
            expected[101] = u8::from(extended_args);
            expected[118] = tag;
            assert_eq!(state.encode(), expected);
            assert_eq!(State::decode(&expected), Ok(state));
            assert_eq!(status.encode(), [tag]);
        }
    }
    for (action, expected) in [
        (Action::Release, b"ESA01\x00"),
        (Action::Refund, b"ESA01\x01"),
        (Action::Close, b"ESA01\x02"),
    ] {
        assert_eq!(action.encode(), expected);
        assert_eq!(Action::decode(expected), Ok(action));
    }
    assert_eq!(State::SCHEMA.prefix, b"ESC01");
    assert_eq!(Action::SCHEMA.prefix, b"ESA01");
}
