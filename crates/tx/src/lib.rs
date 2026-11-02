//! Host-side unsigned transaction assembly. No node, wallet or test VM required.
//! Business amounts, authorization, fees and code dependencies remain explicit.
pub mod context;
pub mod edit;
pub use ckb_types;

use ckb_contract_core::{token::encode_amount, BinaryCodec};
use ckb_types::{
    bytes::Bytes,
    core::{TransactionBuilder, TransactionView},
    packed::{Byte32, CellInput, CellOutput, OutPoint, Script, WitnessArgs},
    prelude::*,
};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    NoInputs,
    NoOutputs,
    InputIndex { index: usize, len: usize },
    OutputIndex { index: usize, len: usize },
    OutputDataMismatch,
    InvalidWitness,
    MissingStateInput,
    DuplicatePair,
    Layout(ckb_contract_core::Error),
    ReserveCount { expected: usize, actual: usize },
    ReserveIndex { index: usize, len: usize },
    InvalidCellSet,
    DuplicateInput,
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for BuildError {}

pub fn input(point: OutPoint) -> CellInput {
    CellInput::new_builder().previous_output(point).build()
}

fn validate_inputs(inputs: &[CellInput]) -> Result<(), BuildError> {
    if inputs.is_empty() {
        return Err(BuildError::NoInputs);
    }
    let mut points = HashSet::new();
    for cell in inputs {
        if !points.insert(cell.previous_output()) {
            return Err(BuildError::DuplicateInput);
        }
    }
    Ok(())
}

pub fn amount(value: u128) -> Bytes {
    Bytes::copy_from_slice(&encode_amount(value))
}

pub fn action_witness_bytes(action: &[u8]) -> Bytes {
    WitnessArgs::new_builder()
        .input_type(Some(Bytes::copy_from_slice(action)).pack())
        .build()
        .as_bytes()
}

pub fn action_witness<A: BinaryCodec>(action: &A) -> Bytes {
    action_witness_bytes(&action.encode())
}

pub fn action_witnesses<A: BinaryCodec>(
    action: &A,
    index: usize,
    count: usize,
) -> Result<Vec<Bytes>, BuildError> {
    if index >= count {
        return Err(BuildError::InputIndex { index, len: count });
    }
    let mut witnesses = vec![Bytes::new(); count];
    witnesses[index] = action_witness(action);
    Ok(witnesses)
}

/// Keep each output and its data together until final Molecule serialization.
#[derive(Clone)]
pub struct Cell {
    pub output: CellOutput,
    pub data: Bytes,
}

impl Cell {
    pub fn new(output: CellOutput, data: Bytes) -> Self {
        Self { output, data }
    }

    pub fn capacity(owner: &Script, capacity: u64) -> Self {
        Self::new(
            CellOutput::new_builder()
                .capacity(capacity)
                .lock(owner.clone())
                .build(),
            Bytes::new(),
        )
    }

    pub fn token(owner: &Script, token_type: &Script, capacity: u64, value: u128) -> Self {
        let output = Self::capacity(owner, capacity)
            .output
            .as_builder()
            .type_(Some(token_type.clone()).pack())
            .build();
        Self::new(output, amount(value))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellPair {
    pub state: OutPoint,
    pub reserve: OutPoint,
}

impl CellPair {
    /// Fixed State/Reserve output positions. The caller verifies their contents.
    pub fn at(tx_hash: Byte32) -> Self {
        Self {
            state: OutPoint::new_builder()
                .tx_hash(tx_hash.clone())
                .index(0u32)
                .build(),
            reserve: OutPoint::new_builder().tx_hash(tx_hash).index(1u32).build(),
        }
    }
}

impl From<(OutPoint, OutPoint)> for CellPair {
    fn from((state, reserve): (OutPoint, OutPoint)) -> Self {
        Self { state, reserve }
    }
}
impl From<CellPair> for (OutPoint, OutPoint) {
    fn from(pair: CellPair) -> Self {
        (pair.state, pair.reserve)
    }
}

#[derive(Clone)]
pub struct StateReserveLayout {
    pub state_type: Script,
    pub state_lock: Script,
    pub reserve_lock: Script,
    pub token_type: Script,
    pub state_capacity: u64,
    pub reserve_capacity: u64,
}

impl StateReserveLayout {
    pub fn state_output(&self) -> CellOutput {
        CellOutput::new_builder()
            .capacity(self.state_capacity)
            .lock(self.state_lock.clone())
            .type_(Some(self.state_type.clone()).pack())
            .build()
    }

    pub fn token(&self, owner: &Script, capacity: u64, value: u128) -> Cell {
        Cell::token(owner, &self.token_type, capacity, value)
    }

    pub fn reserve(&self, value: u128) -> Cell {
        self.token(&self.reserve_lock, self.reserve_capacity, value)
    }

    fn cells<S: BinaryCodec>(&self, state: &S, reserve: u128) -> Vec<Cell> {
        vec![
            Cell::new(self.state_output(), Bytes::from(state.encode())),
            self.reserve(reserve),
        ]
    }
}

/// Fixed output 0/1 layout, with automatic action witness placement when the
/// State input moves. Other inputs/outputs, raw since and amounts stay explicit.
pub struct StateReserveTx {
    inputs: Vec<CellInput>,
    outputs: Vec<Cell>,
    pair: Option<CellPair>,
    action: Option<Bytes>,
    state_index: usize,
    since: u64,
}

impl StateReserveTx {
    pub fn create<S: BinaryCodec>(layout: &StateReserveLayout, state: &S, reserve: u128) -> Self {
        Self {
            inputs: vec![],
            outputs: layout.cells(state, reserve),
            pair: None,
            action: None,
            state_index: 0,
            since: 0,
        }
    }

    pub fn update<S: BinaryCodec, A: BinaryCodec>(
        layout: &StateReserveLayout,
        pair: impl Into<CellPair>,
        state: &S,
        reserve: u128,
        action: &A,
    ) -> Self {
        let mut tx = Self::close(pair, action);
        tx.outputs = layout.cells(state, reserve);
        tx
    }

    pub fn close<A: BinaryCodec>(pair: impl Into<CellPair>, action: &A) -> Self {
        Self {
            inputs: vec![],
            outputs: vec![],
            pair: Some(pair.into()),
            action: Some(Bytes::from(action.encode())),
            state_index: 0,
            since: 0,
        }
    }

    pub fn input(mut self, input: CellInput) -> Self {
        self.inputs.push(input);
        self
    }

    pub fn output(mut self, cell: Cell) -> Self {
        self.outputs.push(cell);
        self
    }

    pub fn state_input_index(mut self, index: usize) -> Self {
        self.state_index = index;
        self
    }

    pub fn state_since(mut self, since: u64) -> Self {
        self.since = since;
        self
    }

    pub fn build(mut self) -> Result<TransactionView, BuildError> {
        if let Some(pair) = self.pair {
            if pair.state == pair.reserve {
                return Err(BuildError::DuplicatePair);
            }
            if self.state_index > self.inputs.len() {
                return Err(BuildError::InputIndex {
                    index: self.state_index,
                    len: self.inputs.len() + 1,
                });
            }
            self.inputs.insert(
                self.state_index,
                input(pair.state).as_builder().since(self.since).build(),
            );
            self.inputs
                .insert(self.state_index + 1, input(pair.reserve));
        } else if self.state_index != 0 || self.since != 0 {
            return Err(BuildError::MissingStateInput);
        }
        validate_inputs(&self.inputs)?;
        if self.outputs.is_empty() {
            return Err(BuildError::NoOutputs);
        }
        let (outputs, data): (Vec<_>, Vec<_>) = self
            .outputs
            .into_iter()
            .map(|cell| (cell.output, cell.data))
            .unzip();
        let tx = TransactionBuilder::default()
            .inputs(self.inputs)
            .outputs(outputs)
            .outputs_data(data.pack())
            .build();
        match self.action {
            Some(action) => edit::with_action_bytes(tx, self.state_index, &action),
            None => Ok(tx),
        }
    }
}
