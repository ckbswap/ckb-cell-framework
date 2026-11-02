//! Host-side VM fixtures. Wallets use ALWAYS_SUCCESS, not real signatures.
pub mod time;
pub use ckb_contract_tx as tx;
pub use ckb_testtool;
pub use tx::{action_witness, amount, input};

use ckb_contract_core::{
    args, ckb_hash, entity_id,
    token::{PlainXudtAsset, XUDT_CODE_HASH},
    BinaryCodec, Error,
};
use ckb_testtool::{
    builtin::ALWAYS_SUCCESS,
    ckb_script::{ScriptError, TransactionScriptError},
    ckb_types::{
        bytes::Bytes,
        core::{ScriptHashType, TransactionView},
        packed::{CellOutput, OutPoint, Script},
        prelude::*,
    },
    context::Context,
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tx::{Cell, CellPair, StateReserveLayout};

/// Resolve the calling test crate's workspace, including scaffolded nested crates.
pub fn workspace_root(manifest_dir: impl AsRef<Path>) -> PathBuf {
    for directory in manifest_dir.as_ref().ancestors() {
        if fs::read_to_string(directory.join("Cargo.toml"))
            .is_ok_and(|text| text.lines().any(|line| line.trim() == "[workspace]"))
        {
            return directory.to_path_buf();
        }
    }
    panic!(
        "workspace root not found from {}",
        manifest_dir.as_ref().display()
    );
}

pub fn load_binary(root: &Path, package: &str) -> Bytes {
    Bytes::from(
        fs::read(root.join("build").join(package))
            .unwrap_or_else(|error| panic!("run bash scripts/build.sh {package}: {error}")),
    )
}

pub fn wallets(context: &mut Context) -> [Script; 3] {
    let code = context.deploy_cell(ALWAYS_SUCCESS.clone());
    [1, 2, 3].map(|tag| {
        context
            .build_script_with_hash_type(&code, ScriptHashType::Data2, Bytes::from(vec![tag]))
            .unwrap()
    })
}

pub fn deploy_xudt(
    context: &mut Context,
    root: &Path,
    asset: PlainXudtAsset,
) -> (OutPoint, Script) {
    let data = fs::read(root.join("vendor/xudt")).expect("run python3 scripts/fetch-udt.py");
    assert_eq!(
        ckb_hash(&data),
        XUDT_CODE_HASH,
        "official xUDT hash mismatch"
    );
    let code = context.deploy_cell(Bytes::from(data));
    let script = Script::new_builder()
        .code_hash(XUDT_CODE_HASH.pack())
        .hash_type(ScriptHashType::Data1)
        .args(asset.args().pack())
        .build();
    (code, script)
}

pub fn assert_valid(
    context: &mut Context,
    tx: TransactionView,
    max_cycles: u64,
) -> TransactionView {
    let tx = context.complete_tx(tx);
    context
        .verify_tx(&tx, max_cycles)
        .expect("valid transaction rejected");
    tx
}

/// Inspect the typed VM error: a cycle limit or a different failure cannot pass
/// merely because its formatted text happens to contain the expected number.
pub fn assert_error(context: &mut Context, tx: TransactionView, expected: Error, max_cycles: u64) {
    let tx = context.complete_tx(tx);
    let error = context
        .verify_tx(&tx, max_cycles)
        .expect_err("malicious transaction accepted");
    let cause = error
        .root_cause()
        .downcast_ref::<TransactionScriptError>()
        .unwrap_or_else(|| panic!("expected script rejection {expected:?}: {error:?}"));
    match cause.script_error() {
        ScriptError::ValidationFailure(_, code) => assert_eq!(*code, expected as i8, "{error:?}"),
        _ => panic!("expected script rejection {expected:?}: {error:?}"),
    }
}

/// Require full-transaction rejection and the exact error from a selected Type
/// group. Other scripts (such as xUDT) may reject first during full verification.
/// Fixtures using this helper must use direct Code deps, not dep groups.
pub fn assert_type_error(
    context: &mut Context,
    tx: TransactionView,
    type_script: &Script,
    expected: Error,
    max_cycles: u64,
) {
    use ckb_testtool::{
        ckb_chain_spec::consensus::ConsensusBuilder,
        ckb_script::{ScriptGroupType, TransactionScriptsVerifier, TxVerifyEnv},
        ckb_types::core::{
            cell::{CellMetaBuilder, ResolvedTransaction},
            hardfork::{HardForks, CKB2021, CKB2023},
            DepType, HeaderBuilder,
        },
    };
    use std::sync::Arc;

    let tx = context.complete_tx(tx);
    assert!(
        context.verify_tx(&tx, max_cycles).is_err(),
        "invalid transaction accepted"
    );
    let resolve = |point: OutPoint| {
        let (output, data) = context.get_cell(&point).expect("test Cell missing");
        CellMetaBuilder::from_cell_output(output, data)
            .out_point(point)
            .build()
    };
    let resolved = ResolvedTransaction {
        resolved_inputs: tx.input_pts_iter().map(&resolve).collect(),
        resolved_cell_deps: tx
            .cell_deps_iter()
            .map(|dep| {
                assert_eq!(
                    dep.dep_type(),
                    DepType::Code.into(),
                    "expected direct Code deps"
                );
                resolve(dep.out_point())
            })
            .collect(),
        resolved_dep_groups: vec![],
        transaction: tx,
    };
    let consensus = ConsensusBuilder::default()
        .hardfork_switch(HardForks {
            ckb2021: CKB2021::new_dev_default(),
            ckb2023: CKB2023::new_dev_default(),
        })
        .build();
    let verifier = TransactionScriptsVerifier::new(
        Arc::new(resolved),
        context.clone(),
        Arc::new(consensus),
        Arc::new(TxVerifyEnv::new_submit(&HeaderBuilder::default().build())),
    );
    let error = verifier
        .verify_single(
            ScriptGroupType::Type,
            &type_script.calc_script_hash(),
            max_cycles,
        )
        .expect_err("Type script accepted invalid transaction");
    match error {
        ScriptError::ValidationFailure(_, code) => assert_eq!(code, expected as i8),
        other => panic!("expected Type validation error {expected:?}: {other:?}"),
    }
}

/// Install outputs of an already verified transaction as the next inputs.
/// Spent inputs are removed, so accidental fixture-level replay is rejected.
pub fn commit_outputs(context: &mut Context, tx: &TransactionView) {
    for input in tx.inputs() {
        context.cells.remove(&input.previous_output());
        context.transaction_infos.remove(&input.previous_output());
    }
    context
        .cells_by_data_hash
        .retain(|_, point| context.cells.contains_key(point));
    context
        .cells_by_type_hash
        .retain(|_, point| context.cells.contains_key(point));
    for (point, (cell, data)) in &context.cells {
        context
            .cells_by_data_hash
            .entry(CellOutput::calc_data_hash(data))
            .or_insert_with(|| point.clone());
        if let Some(script) = cell.type_().to_opt() {
            context
                .cells_by_type_hash
                .entry(script.calc_script_hash())
                .or_insert_with(|| point.clone());
        }
    }
    for (index, (cell, data)) in tx.outputs().into_iter().zip(tx.outputs_data()).enumerate() {
        context.create_cell_with_out_point(
            OutPoint::new_builder()
                .tx_hash(tx.hash())
                .index(index as u32)
                .build(),
            cell,
            data.raw_data(),
        );
    }
}

pub fn replace_data(tx: TransactionView, index: usize, data: Bytes) -> TransactionView {
    tx::edit::replace_data(tx, index, data).expect("invalid test output index")
}

pub fn replace_output(tx: TransactionView, index: usize, output: CellOutput) -> TransactionView {
    tx::edit::replace_output(tx, index, output).expect("invalid test output index")
}

pub fn replace_since(tx: TransactionView, index: usize, since: u64) -> TransactionView {
    tx::edit::replace_since(tx, index, since).expect("invalid test input index")
}

pub fn witnesses<A: BinaryCodec>(action: &A, index: usize, count: usize) -> Vec<Bytes> {
    tx::action_witnesses(action, index, count).expect("invalid test witness index")
}

pub struct LayoutConfig {
    /// State Type, State Lock, Reserve Lock role numbers, in that order.
    pub roles: [u8; 3],
    pub state_capacity: u64,
    pub reserve_capacity: u64,
}

pub enum Funding {
    Capacity(u64),
    Token { capacity: u64, amount: u128 },
}

pub struct StateReserveFixture {
    pub context: Context,
    pub code: OutPoint,
    pub xudt_code: OutPoint,
    pub wallets: [Script; 3],
    pub asset: PlainXudtAsset,
    pub layout: StateReserveLayout,
    pub funding: OutPoint,
    pub pair: Option<(OutPoint, OutPoint)>,
    pub max_cycles: u64,
}

impl StateReserveFixture {
    pub fn new(
        root: &Path,
        package: &str,
        asset: PlainXudtAsset,
        config: LayoutConfig,
        funding: Funding,
    ) -> Self {
        let mut context = Context::default();
        let code = context.deploy_cell(load_binary(root, package));
        let (xudt_code, token_type) = deploy_xudt(&mut context, root, asset);
        let wallets = wallets(&mut context);
        let cell = match funding {
            Funding::Capacity(capacity) => Cell::capacity(&wallets[0], capacity),
            Funding::Token { capacity, amount } => {
                Cell::token(&wallets[0], &token_type, capacity, amount)
            }
        };
        let funding = context.create_cell(cell.output, cell.data);
        let id = entity_id(input(funding.clone()).as_slice().try_into().unwrap(), 0);
        let [state_role, lock_role, reserve_role] = config.roles;
        let state_type = context
            .build_script_with_hash_type(
                &code,
                ScriptHashType::Data2,
                Bytes::from(args::encode(state_role, &id, &[])),
            )
            .unwrap();
        let role = |role| {
            state_type
                .clone()
                .as_builder()
                .args(args::encode(role, &id, &[]).pack())
                .build()
        };
        let layout = StateReserveLayout {
            state_lock: role(lock_role),
            reserve_lock: role(reserve_role),
            state_type,
            token_type,
            state_capacity: config.state_capacity,
            reserve_capacity: config.reserve_capacity,
        };
        Self {
            context,
            code,
            xudt_code,
            wallets,
            asset,
            layout,
            funding,
            pair: None,
            max_cycles: 40_000_000,
        }
    }

    /// Synthetic predecessor for isolated attack vectors. Continuous lifecycle
    /// tests use committed OutPoints instead; the caller supplies reserve data.
    pub fn pair<S: BinaryCodec>(
        &mut self,
        before: &S,
        reserve_amount: u128,
    ) -> (OutPoint, OutPoint) {
        if let Some(pair) = &self.pair {
            return pair.clone();
        }
        let pair = CellPair::at(ckb_hash(&before.encode()).pack());
        let state = self.layout.state_output();
        let reserve = self.layout.reserve(reserve_amount);
        self.context.create_cell_with_out_point(
            pair.state.clone(),
            state,
            Bytes::from(before.encode()),
        );
        self.context
            .create_cell_with_out_point(pair.reserve.clone(), reserve.output, reserve.data);
        pair.into()
    }

    pub fn cell(&mut self, cell: Cell) -> OutPoint {
        self.context.create_cell(cell.output, cell.data)
    }

    pub fn assert_valid(&mut self, tx: TransactionView) -> TransactionView {
        assert_valid(&mut self.context, tx, self.max_cycles)
    }

    pub fn assert_error(&mut self, tx: TransactionView, error: Error) {
        assert_error(&mut self.context, tx, error, self.max_cycles)
    }

    pub fn verify_time(
        &mut self,
        tx: &TransactionView,
        median_ms: u64,
    ) -> Result<(), ckb_testtool::ckb_error::Error> {
        time::verify(&mut self.context, tx, median_ms)
    }

    pub fn assert_valid_at(&mut self, tx: TransactionView, seconds: u64) -> TransactionView {
        self.verify_time(&tx, seconds.checked_mul(1000).expect("timestamp overflow"))
            .expect("since is not mature");
        self.assert_valid(tx)
    }

    pub fn commit(&mut self, tx: TransactionView) {
        let tx = self.assert_valid(tx);
        self.install(&tx);
    }

    pub fn commit_at(&mut self, tx: TransactionView, seconds: u64) {
        let tx = self.assert_valid_at(tx, seconds);
        self.install(&tx);
    }

    fn install(&mut self, tx: &TransactionView) {
        assert!(
            tx.outputs().len() >= 2,
            "commit expects State/Reserve outputs 0/1"
        );
        assert_eq!(
            tx.outputs().get(0).unwrap().type_().to_opt().as_ref(),
            Some(&self.layout.state_type)
        );
        assert_eq!(tx.outputs().get(0).unwrap().lock(), self.layout.state_lock);
        assert_eq!(
            tx.outputs().get(1).unwrap().lock(),
            self.layout.reserve_lock
        );
        assert_eq!(
            tx.outputs().get(1).unwrap().type_().to_opt().as_ref(),
            Some(&self.layout.token_type)
        );
        commit_outputs(&mut self.context, tx);
        self.pair = Some(CellPair::at(tx.hash()).into());
    }
}
