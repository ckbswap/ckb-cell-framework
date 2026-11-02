//! Assembly from the same layout values used by on-chain Cells declarations.
use crate::{edit, input, BuildError, Cell, CellPair};
use ckb_contract_core::{
    args,
    layout::{ReserveLayout, StateLayout, TransactionLayout},
    BinaryCodec, Hash,
};
use ckb_types::{
    bytes::Bytes,
    core::{TransactionBuilder, TransactionView},
    packed::{Byte32, CellInput, CellOutput, OutPoint, Script},
    prelude::*,
};

#[derive(Clone)]
pub struct ResolvedReserve {
    pub layout: ReserveLayout,
    pub lock: Script,
    pub token_type: Script,
}

/// Resolves code identity and full asset Types without network queries. The
/// caller supplies trusted deployment information, amounts and economic policy.
#[derive(Clone)]
pub struct ResolvedLayout {
    state: StateLayout,
    state_type: Script,
    state_lock: Script,
    reserves: Vec<ResolvedReserve>,
}

impl ResolvedLayout {
    pub fn new(
        layout: &TransactionLayout<'_>,
        code: &Script,
        id: &Hash,
        assets: &[Script],
    ) -> Result<Self, BuildError> {
        layout.validate().map_err(BuildError::Layout)?;
        require_count(layout.reserves.len(), assets.len())?;
        let role = |role| {
            code.clone()
                .as_builder()
                .args(args::encode(role, id, &[]).pack())
                .build()
        };
        Ok(Self {
            state: layout.state,
            state_type: role(layout.state.role),
            state_lock: role(layout.state.lock_role),
            reserves: layout
                .reserves
                .iter()
                .zip(assets)
                .map(|(layout, asset)| ResolvedReserve {
                    layout: *layout,
                    lock: role(layout.role),
                    token_type: asset.clone(),
                })
                .collect(),
        })
    }

    pub fn state(&self) -> StateLayout {
        self.state
    }
    pub fn state_type(&self) -> &Script {
        &self.state_type
    }
    pub fn state_lock(&self) -> &Script {
        &self.state_lock
    }
    pub fn reserves(&self) -> &[ResolvedReserve] {
        &self.reserves
    }

    pub fn state_output(&self) -> CellOutput {
        CellOutput::new_builder()
            .capacity(self.state.capacity)
            .lock(self.state_lock.clone())
            .type_(Some(self.state_type.clone()).pack())
            .build()
    }

    pub fn token(
        &self,
        reserve: usize,
        owner: &Script,
        capacity: u64,
        amount: u128,
    ) -> Result<Cell, BuildError> {
        let asset = self.reserves.get(reserve).ok_or(BuildError::ReserveIndex {
            index: reserve,
            len: self.reserves.len(),
        })?;
        Ok(Cell::token(owner, &asset.token_type, capacity, amount))
    }

    pub fn reserve(&self, index: usize, amount: u128) -> Result<Cell, BuildError> {
        let reserve = self.reserves.get(index).ok_or(BuildError::ReserveIndex {
            index,
            len: self.reserves.len(),
        })?;
        self.token(index, &reserve.lock, reserve.layout.capacity, amount)
    }

    fn outputs<S: BinaryCodec>(
        &self,
        state: &S,
        amounts: &[u128],
    ) -> Result<Vec<(usize, Cell)>, BuildError> {
        require_count(self.reserves.len(), amounts.len())?;
        let mut cells = vec![(
            self.state.output,
            Cell::new(self.state_output(), Bytes::from(state.encode())),
        )];
        for (index, (reserve, amount)) in self.reserves.iter().zip(amounts).enumerate() {
            cells.push((reserve.layout.output, self.reserve(index, *amount)?));
        }
        Ok(cells)
    }

    fn validate_inputs(&self, cells: &CellSet) -> Result<(), BuildError> {
        require_count(self.reserves.len(), cells.reserves.len())?;
        if u32::from(cells.state.index()) as usize != self.state.output {
            return Err(BuildError::InvalidCellSet);
        }
        for (reserve, point) in self.reserves.iter().zip(&cells.reserves) {
            if point.tx_hash() != cells.state.tx_hash()
                || u32::from(point.index()) as usize != reserve.layout.output
            {
                return Err(BuildError::InvalidCellSet);
            }
        }
        Ok(())
    }
}

/// OutPoints in declaration order. Positions and shared predecessor identity
/// are checked during update/close, but liveness and contents require a chain source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellSet {
    pub state: OutPoint,
    pub reserves: Vec<OutPoint>,
}

impl CellSet {
    pub fn at(layout: &TransactionLayout<'_>, tx_hash: Byte32) -> Result<Self, BuildError> {
        layout.validate().map_err(BuildError::Layout)?;
        let point = |index: usize| {
            OutPoint::new_builder()
                .tx_hash(tx_hash.clone())
                .index(index as u32)
                .build()
        };
        Ok(Self {
            state: point(layout.state.output),
            reserves: layout
                .reserves
                .iter()
                .map(|reserve| point(reserve.output))
                .collect(),
        })
    }
}

impl From<CellPair> for CellSet {
    fn from(pair: CellPair) -> Self {
        Self {
            state: pair.state,
            reserves: vec![pair.reserve],
        }
    }
}

/// One State with zero or more reserves, at arbitrary declared output positions.
/// Additional outputs fill unused positions in insertion order. Inputs form a
/// movable State/reserves block; the Action and since follow the State input.
pub struct ContextTx {
    inputs: Vec<CellInput>,
    fixed_outputs: Vec<(usize, Cell)>,
    outputs: Vec<Cell>,
    cells: Option<CellSet>,
    action: Option<Bytes>,
    state_index: usize,
    since: u64,
}

impl ContextTx {
    pub fn create<S: BinaryCodec>(
        layout: &ResolvedLayout,
        state: &S,
        amounts: &[u128],
    ) -> Result<Self, BuildError> {
        Ok(Self {
            inputs: vec![],
            fixed_outputs: layout.outputs(state, amounts)?,
            outputs: vec![],
            cells: None,
            action: None,
            state_index: 0,
            since: 0,
        })
    }

    pub fn update<S: BinaryCodec, A: BinaryCodec>(
        layout: &ResolvedLayout,
        cells: CellSet,
        state: &S,
        amounts: &[u128],
        action: &A,
    ) -> Result<Self, BuildError> {
        let mut tx = Self::close(layout, cells, action)?;
        tx.fixed_outputs = layout.outputs(state, amounts)?;
        Ok(tx)
    }

    pub fn close<A: BinaryCodec>(
        layout: &ResolvedLayout,
        cells: CellSet,
        action: &A,
    ) -> Result<Self, BuildError> {
        layout.validate_inputs(&cells)?;
        Ok(Self {
            inputs: vec![],
            fixed_outputs: vec![],
            outputs: vec![],
            cells: Some(cells),
            action: Some(Bytes::from(action.encode())),
            state_index: 0,
            since: 0,
        })
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
        if let Some(cells) = self.cells {
            if self.state_index > self.inputs.len() {
                return Err(BuildError::InputIndex {
                    index: self.state_index,
                    len: self.inputs.len() + 1,
                });
            }
            let block = std::iter::once(input(cells.state).as_builder().since(self.since).build())
                .chain(cells.reserves.into_iter().map(input));
            self.inputs
                .splice(self.state_index..self.state_index, block);
        } else if self.state_index != 0 || self.since != 0 {
            return Err(BuildError::MissingStateInput);
        }
        crate::validate_inputs(&self.inputs)?;
        // Bound allocation by the actual supplied Cell count, never the largest
        // declared position (which may be u32::MAX).
        let len = self.fixed_outputs.len() + self.outputs.len();
        if len == 0 {
            return Err(BuildError::NoOutputs);
        }
        for (index, _) in &self.fixed_outputs {
            if *index >= len {
                return Err(BuildError::OutputIndex { index: *index, len });
            }
        }
        self.fixed_outputs.sort_unstable_by_key(|(index, _)| *index);
        let mut fixed = self.fixed_outputs.into_iter().peekable();
        let mut extra = self.outputs.into_iter();
        let mut outputs = Vec::with_capacity(len);
        let mut data = Vec::with_capacity(len);
        for index in 0..len {
            let cell = if fixed.peek().is_some_and(|(position, _)| *position == index) {
                fixed.next().map(|(_, cell)| cell)
            } else {
                extra.next()
            }
            .ok_or(BuildError::OutputIndex { index, len })?;
            outputs.push(cell.output);
            data.push(cell.data);
        }
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

fn require_count(expected: usize, actual: usize) -> Result<(), BuildError> {
    if expected != actual {
        return Err(BuildError::ReserveCount { expected, actual });
    }
    Ok(())
}
