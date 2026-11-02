//! Frozen wire vectors: change the version explicitly before changing these bytes.
use ckb_contract_core::{token::PlainXudtAsset, BinaryCodec};
use framework_vesting_script::{Action, Schedule, State};

fn bytes(hex: &str) -> Vec<u8> {
    assert_eq!(hex.len() % 2, 0);
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn state_and_actions_preserve_v1_wire_vectors() {
    let state_bytes = bytes(concat!(
        "5653543031",
        "0101010101010101010101010101010101010101010101010101010101010101",
        "0202020202020202020202020202020202020202020202020202020202020202",
        "0303030303030303030303030303030303030303030303030303030303030303",
        "01",
        "e8030000000000000000000000000000",
        "00000000000000000000000000000000",
        "e803000000000000b004000000000000d007000000000000",
    ));
    let state = State::new(
        [1; 32],
        [2; 32],
        PlainXudtAsset {
            owner: [3; 32],
            extended_args: true,
        },
        1000,
        Schedule {
            start: 1000,
            cliff: 1200,
            end: 2000,
        },
    )
    .unwrap();
    assert_eq!(State::decode(&state_bytes).unwrap(), state);
    assert_eq!(state.encode(), state_bytes);

    for (action, hex) in [
        (
            Action::Claim {
                timestamp: 2000,
                amount: 1000,
            },
            "565341303100d007000000000000e8030000000000000000000000000000",
        ),
        (Action::Close, "565341303101"),
    ] {
        let encoded = bytes(hex);
        assert_eq!(Action::decode(&encoded).unwrap(), action);
        assert_eq!(action.encode(), encoded);
    }
}
