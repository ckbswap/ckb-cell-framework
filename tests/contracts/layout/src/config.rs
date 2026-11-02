//! Structural VM fixture, not a production protocol. Deliberately uses sparse
//! outputs and two assets so neither side can assume State/Reserve positions 0/1.
use ckb_contract_core::{
    layout::{ReserveLayout, StateLayout, TransactionLayout},
    token::PlainXudtAsset,
    CKB,
};

pub const LAYOUT: TransactionLayout<'static> = TransactionLayout {
    state: StateLayout {
        role: 0,
        lock_role: 1,
        capacity: 400 * CKB,
        output: 3,
    },
    reserves: &[
        ReserveLayout {
            role: 2,
            capacity: 200 * CKB,
            output: 0,
        },
        ReserveLayout {
            role: 3,
            capacity: 200 * CKB,
            output: 5,
        },
    ],
};

pub const ASSETS: [PlainXudtAsset; 2] = [
    PlainXudtAsset {
        owner: [10; 32],
        extended_args: false,
    },
    PlainXudtAsset {
        owner: [11; 32],
        extended_args: true,
    },
];
