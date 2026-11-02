//! Official consensus maturity checks, separate from script-only VM verification.
use ckb_testtool::{
    ckb_chain_spec::consensus::ConsensusBuilder,
    ckb_traits::{HeaderFields, HeaderFieldsProvider},
    ckb_types::{
        core::{
            cell::{CellMetaBuilder, ResolvedTransaction},
            EpochNumberWithFraction, HeaderBuilder, TransactionView,
        },
        packed::Byte32,
    },
    ckb_verification::{TimeRelativeTransactionVerifier, TxVerifyEnv},
    context::Context,
};
use std::sync::Arc;

struct History<'a>(&'a Context);
impl HeaderFieldsProvider for History<'_> {
    fn get_header_fields(&self, hash: &Byte32) -> Option<HeaderFields> {
        let header = self.0.headers.get(hash)?;
        Some(HeaderFields {
            hash: header.hash(),
            number: header.number(),
            epoch: header.epoch(),
            timestamp: header.timestamp(),
            parent_hash: header.parent_hash(),
        })
    }
}

/// Synthetic linked headers have the requested exact median. Header validity
/// and PoW are outside this helper; the official since/maturity verifier runs.
pub fn verify(
    context: &mut Context,
    tx: &TransactionView,
    median_ms: u64,
) -> Result<(), ckb_testtool::ckb_error::Error> {
    let consensus = ConsensusBuilder::default().build();
    let count = consensus.median_time_block_count() as u64;
    assert!(
        median_ms >= count / 2,
        "median must allow a nonnegative header history"
    );
    let mut parent = Byte32::default();
    for number in 0..count {
        let header = HeaderBuilder::default()
            .number(number)
            .epoch(EpochNumberWithFraction::new(0, number, count + 1))
            .parent_hash(parent)
            .timestamp(median_ms - count / 2 + number)
            .build();
        parent = header.hash();
        context.insert_header(header);
    }
    let committing = HeaderBuilder::default()
        .number(count)
        .epoch(EpochNumberWithFraction::new(0, count, count + 1))
        .parent_hash(parent)
        .build();
    let resolved_inputs = tx
        .inputs()
        .into_iter()
        .map(|input| {
            let point = input.previous_output();
            let (cell, data) = context.get_cell(&point).expect("test input not found");
            let mut meta = CellMetaBuilder::from_cell_output(cell, data).out_point(point.clone());
            if let Some(info) = context.transaction_infos.get(&point) {
                meta = meta.transaction_info(info.clone());
            }
            meta.build()
        })
        .collect();
    let resolved = ResolvedTransaction {
        transaction: tx.clone(),
        resolved_inputs,
        resolved_cell_deps: vec![],
        resolved_dep_groups: vec![],
    };
    TimeRelativeTransactionVerifier::new(
        Arc::new(resolved),
        Arc::new(consensus),
        History(context),
        Arc::new(TxVerifyEnv::new_commit(&committing)),
    )
    .verify()
}
