//! Checked transaction edits; useful for client assembly and malicious vectors.
use crate::BuildError;
use ckb_contract_core::BinaryCodec;
use ckb_types::{
    bytes::Bytes,
    core::TransactionView,
    packed::{CellOutput, WitnessArgs},
    prelude::*,
};

pub fn with_action<A: BinaryCodec>(
    tx: TransactionView,
    index: usize,
    action: &A,
) -> Result<TransactionView, BuildError> {
    with_action_bytes(tx, index, &action.encode())
}

/// Preserve existing WitnessArgs.lock/output_type and extra witnesses.
/// Apply before signing, because changing input_type changes signed data.
pub fn with_action_bytes(
    tx: TransactionView,
    index: usize,
    action: &[u8],
) -> Result<TransactionView, BuildError> {
    if index >= tx.inputs().len() {
        return Err(BuildError::InputIndex {
            index,
            len: tx.inputs().len(),
        });
    }
    let mut witnesses: Vec<Bytes> = tx.witnesses().into_iter().map(|w| w.raw_data()).collect();
    witnesses.resize(witnesses.len().max(tx.inputs().len()), Bytes::new());
    let args = if witnesses[index].is_empty() {
        WitnessArgs::default()
    } else {
        WitnessArgs::from_slice(&witnesses[index]).map_err(|_| BuildError::InvalidWitness)?
    };
    witnesses[index] = args
        .as_builder()
        .input_type(Some(Bytes::copy_from_slice(action)).pack())
        .build()
        .as_bytes();
    Ok(tx
        .as_advanced_builder()
        .set_witnesses(witnesses.into_iter().map(|w| w.pack()).collect())
        .build())
}

pub fn replace_since(
    tx: TransactionView,
    index: usize,
    since: u64,
) -> Result<TransactionView, BuildError> {
    let mut inputs: Vec<_> = tx.inputs().into_iter().collect();
    let len = inputs.len();
    let slot = inputs
        .get_mut(index)
        .ok_or(BuildError::InputIndex { index, len })?;
    *slot = slot.clone().as_builder().since(since).build();
    Ok(tx.as_advanced_builder().set_inputs(inputs).build())
}

pub fn replace_data(
    tx: TransactionView,
    index: usize,
    value: Bytes,
) -> Result<TransactionView, BuildError> {
    if tx.outputs().len() != tx.outputs_data().len() {
        return Err(BuildError::OutputDataMismatch);
    }
    let mut data: Vec<_> = tx.outputs_data().into_iter().collect();
    let len = data.len();
    *data
        .get_mut(index)
        .ok_or(BuildError::OutputIndex { index, len })? = value.pack();
    Ok(tx.as_advanced_builder().set_outputs_data(data).build())
}

pub fn replace_output(
    tx: TransactionView,
    index: usize,
    value: CellOutput,
) -> Result<TransactionView, BuildError> {
    if tx.outputs().len() != tx.outputs_data().len() {
        return Err(BuildError::OutputDataMismatch);
    }
    let mut outputs: Vec<_> = tx.outputs().into_iter().collect();
    let len = outputs.len();
    *outputs
        .get_mut(index)
        .ok_or(BuildError::OutputIndex { index, len })? = value;
    Ok(tx.as_advanced_builder().set_outputs(outputs).build())
}
