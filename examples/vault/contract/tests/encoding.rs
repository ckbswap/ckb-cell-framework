// Fixed protocol vectors, independent of CellCodec and its generated schema.
use ckb_contract_core::{token::PlainXudtAsset, wire::CodecSchema, BinaryCodec};

use framework_vault_script::{Action, State};

fn bytes(hex: &str) -> Vec<u8> {
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(core::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn derived_vault_codecs_match_protocol_vectors() {
    for extended_args in [false, true] {
        let state = State {
            owner: [0x11; 32],
            asset: PlainXudtAsset {
                owner: [0x22; 32],
                extended_args,
            },
            nonce: 0x0102_0304_0506_0708,
            balance: 0x1112_1314_1516_1718_191a_1b1c_1d1e_1f20,
        };
        let mut expected = bytes("564c54303111111111111111111111111111111111111111111111111111111111111111112222222222222222222222222222222222222222222222222222222222222222000807060504030201201f1e1d1c1b1a191817161514131211");
        expected[69] = u8::from(extended_args);
        assert_eq!(state.encode(), expected);
        assert_eq!(State::decode(&expected), Ok(state));
    }
    for (action, expected) in [
        (
            Action::Deposit(u128::MAX),
            bytes("564c41303100ffffffffffffffffffffffffffffffff"),
        ),
        (
            Action::Withdraw(u128::MAX),
            bytes("564c41303101ffffffffffffffffffffffffffffffff"),
        ),
        (Action::Close, b"VLA01\x02".to_vec()),
    ] {
        assert_eq!(action.encode(), expected);
        assert_eq!(Action::decode(&expected), Ok(action));
    }
    assert_eq!(State::SCHEMA.prefix, b"VLT01");
    assert_eq!(Action::SCHEMA.prefix, b"VLA01");
}
