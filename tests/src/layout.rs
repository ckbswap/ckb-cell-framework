use ckb_contract_core::{entity_id, BinaryCodec, Error, StateTransition, CKB};
use ckb_contract_testkit::{
    self as kit,
    ckb_testtool::{
        ckb_types::{
            bytes::Bytes,
            core::{ScriptHashType, TransactionView},
            packed::{OutPoint, Script},
            prelude::*,
        },
        context::Context,
    },
};
use ckb_contract_tx::{
    context::{CellSet, ContextTx, ResolvedLayout},
    input, Cell,
};
use framework_counter_script::{Action, State};

#[path = "../contracts/layout/src/config.rs"]
mod config;
use config::{ASSETS, LAYOUT};

struct Fixture {
    context: Context,
    layout: ResolvedLayout,
    wallet: Script,
    state: State,
    points: CellSet,
}

impl Fixture {
    fn new() -> Self {
        let root = kit::workspace_root(env!("CARGO_MANIFEST_DIR"));
        let mut context = Context::default();
        let code = context.deploy_cell(kit::load_binary(&root, "framework-layout-test-script"));
        let base = context
            .build_script_with_hash_type(&code, ScriptHashType::Data2, Bytes::new())
            .unwrap();
        let (_, asset0) = kit::deploy_xudt(&mut context, &root, ASSETS[0]);
        let (_, asset1) = kit::deploy_xudt(&mut context, &root, ASSETS[1]);
        let wallet = kit::wallets(&mut context)[0].clone();
        let funding: Vec<_> = [&asset0, &asset1]
            .into_iter()
            .zip([200, 400])
            .map(|(asset, amount)| {
                let cell = Cell::token(&wallet, asset, 1000 * CKB, amount);
                context.create_cell(cell.output, cell.data)
            })
            .collect();
        let first = input(funding[0].clone());
        let id = entity_id(
            first.as_slice().try_into().unwrap(),
            LAYOUT.state.output as u64,
        );
        let layout = ResolvedLayout::new(&LAYOUT, &base, &id, &[asset0, asset1]).unwrap();
        let state = State::new(wallet.calc_script_hash().unpack());
        let creation = ContextTx::create(&layout, &state, &[100, 200])
            .unwrap()
            .input(first)
            .input(input(funding[1].clone()))
            .output(layout.token(0, &wallet, 200 * CKB, 100).unwrap())
            .output(layout.token(1, &wallet, 200 * CKB, 200).unwrap())
            .output(Cell::capacity(&wallet, 799 * CKB))
            .build()
            .unwrap();
        let creation = kit::assert_valid(&mut context, creation, 40_000_000);
        kit::commit_outputs(&mut context, &creation);
        let points = CellSet::at(&LAYOUT, creation.hash()).unwrap();
        Self {
            context,
            layout,
            wallet,
            state,
            points,
        }
    }

    fn update(&mut self, state_index: usize) -> TransactionView {
        let mut funding = Vec::new();
        for _ in 0..2 {
            let cell = Cell::capacity(&self.wallet, 500 * CKB);
            funding.push(self.context.create_cell(cell.output, cell.data));
        }
        let action = Action { increment: 1 };
        ContextTx::update(
            &self.layout,
            self.points.clone(),
            &self.state.apply(&action).unwrap(),
            &[100, 200],
            &action,
        )
        .unwrap()
        .input(input(funding[0].clone()))
        .input(input(funding[1].clone()))
        .state_input_index(state_index)
        .output(Cell::capacity(&self.wallet, 200 * CKB))
        .output(Cell::capacity(&self.wallet, 200 * CKB))
        .output(Cell::capacity(&self.wallet, 599 * CKB))
        .build()
        .unwrap()
    }

    fn replace_reserve(
        &mut self,
        tx: TransactionView,
        reserve: usize,
        input_side: bool,
        cell: Cell,
    ) -> TransactionView {
        if input_side {
            // Corrupt resolved predecessor contents to exercise defensive input
            // checks; each case starts from a fresh, verified creation.
            self.context.create_cell_with_out_point(
                self.points.reserves[reserve].clone(),
                cell.output,
                cell.data,
            );
            tx
        } else {
            let position = LAYOUT.reserves[reserve].output;
            let tx = kit::replace_output(tx, position, cell.output);
            kit::replace_data(tx, position, cell.data)
        }
    }
}

#[test]
fn shared_layout_creates_and_updates_two_reserves_at_sparse_positions() {
    let mut f = Fixture::new();
    for index in 0..=2 {
        let tx = f.update(index);
        let tx = kit::assert_valid(&mut f.context, tx, 40_000_000);
        kit::commit_outputs(&mut f.context, &tx);
        f.state = State::decode(
            &tx.outputs_data()
                .get(LAYOUT.state.output)
                .unwrap()
                .raw_data(),
        )
        .unwrap();
        f.points = CellSet::at(&LAYOUT, tx.hash()).unwrap();
    }
    assert_eq!(f.state.value, 3);
}

#[test]
fn shared_layout_rejects_each_foreign_reserve_wrong_position_and_wrong_amount() {
    for reserve in 0..2 {
        for mode in 0..3 {
            let mut f = Fixture::new();
            let tx = f.update(1);
            let tx = match mode {
                0 => {
                    let cell = f.layout.reserve(reserve, [100, 200][reserve]).unwrap();
                    let foreign = OutPoint::new_builder()
                        .tx_hash([42; 32].pack())
                        .index(LAYOUT.reserves[reserve].output as u32)
                        .build();
                    f.context
                        .create_cell_with_out_point(foreign.clone(), cell.output, cell.data);
                    let mut inputs: Vec<_> = tx.inputs().into_iter().collect();
                    inputs[2 + reserve] = input(foreign);
                    tx.as_advanced_builder().set_inputs(inputs).build()
                }
                1 => {
                    let position = LAYOUT.reserves[reserve].output;
                    let mut outputs: Vec<_> = tx.outputs().into_iter().collect();
                    let mut data: Vec<_> = tx.outputs_data().into_iter().collect();
                    outputs.swap(position, 1);
                    data.swap(position, 1);
                    tx.as_advanced_builder()
                        .set_outputs(outputs)
                        .set_outputs_data(data)
                        .build()
                }
                _ => kit::replace_data(
                    tx,
                    LAYOUT.reserves[reserve].output,
                    kit::amount([99, 199][reserve]),
                ),
            };
            kit::assert_error(
                &mut f.context,
                tx,
                if mode == 2 {
                    Error::Token
                } else {
                    Error::Structure
                },
                40_000_000,
            );
        }
    }
}

#[test]
fn shared_layout_rejects_missing_reserves_on_either_side() {
    for reserve in 0..2 {
        for input_side in [false, true] {
            let mut f = Fixture::new();
            let tx = f.update(1);
            let mut cell = f.layout.reserve(reserve, [100, 200][reserve]).unwrap();
            // Preserve capacity and asset totals while removing the Reserve Lock.
            cell.output = cell.output.as_builder().lock(f.wallet.clone()).build();
            let tx = f.replace_reserve(tx, reserve, input_side, cell);
            kit::assert_type_error(
                &mut f.context,
                tx,
                f.layout.state_type(),
                Error::Structure,
                40_000_000,
            );
        }
    }
}

#[test]
fn shared_layout_rejects_missing_or_swapped_reserve_asset_types() {
    for input_side in [false, true] {
        for reserve in 0..2 {
            let mut f = Fixture::new();
            let tx = f.update(1);
            let mut cell = f.layout.reserve(reserve, [100, 200][reserve]).unwrap();
            cell.output = cell
                .output
                .as_builder()
                .type_(None::<Script>.pack())
                .build();
            let tx = f.replace_reserve(tx, reserve, input_side, cell);
            kit::assert_type_error(
                &mut f.context,
                tx,
                f.layout.state_type(),
                Error::Token,
                40_000_000,
            );
        }
        let mut f = Fixture::new();
        let mut tx = f.update(1);
        for reserve in 0..2 {
            let mut cell = f.layout.reserve(reserve, [100, 200][reserve]).unwrap();
            let other = f.layout.reserve(1 - reserve, [200, 100][reserve]).unwrap();
            cell.output = cell.output.as_builder().type_(other.output.type_()).build();
            cell.data = other.data;
            tx = f.replace_reserve(tx, reserve, input_side, cell);
        }
        // Swapping Type and amount together preserves both assets' total supply.
        // Reserve identity checks must still reject their association with the wrong roles.
        kit::assert_type_error(
            &mut f.context,
            tx,
            f.layout.state_type(),
            Error::Token,
            40_000_000,
        );
    }
}

#[test]
fn shared_layout_rejects_noncanonical_reserve_data_on_either_side() {
    for reserve in 0..2 {
        for input_side in [false, true] {
            for len in [15, 17] {
                let mut f = Fixture::new();
                let tx = f.update(1);
                let mut cell = f.layout.reserve(reserve, [100, 200][reserve]).unwrap();
                let mut bytes = cell.data.to_vec();
                bytes.resize(len, 0);
                cell.data = Bytes::from(bytes);
                let tx = f.replace_reserve(tx, reserve, input_side, cell);
                kit::assert_type_error(
                    &mut f.context,
                    tx,
                    f.layout.state_type(),
                    Error::Token,
                    40_000_000,
                );
            }
        }
    }
}

#[test]
fn shared_layout_rejects_reserve_capacity_changes_on_either_side() {
    for reserve in 0..2 {
        for input_side in [false, true] {
            let mut f = Fixture::new();
            let tx = f.update(1);
            let mut cell = f.layout.reserve(reserve, [100, 200][reserve]).unwrap();
            cell.output = cell
                .output
                .as_builder()
                .capacity(LAYOUT.reserves[reserve].capacity + 1)
                .build();
            let tx = f.replace_reserve(tx, reserve, input_side, cell);
            kit::assert_type_error(
                &mut f.context,
                tx,
                f.layout.state_type(),
                Error::Capacity,
                40_000_000,
            );
        }
    }
}

#[test]
fn shared_layout_rejects_actions_in_a_funding_or_reserve_witness() {
    for state_index in [1, 2] {
        for witness_index in [0, state_index + 1, state_index + 2] {
            let mut f = Fixture::new();
            let tx = f.update(state_index);
            let witnesses =
                kit::witnesses(&Action { increment: 1 }, witness_index, tx.inputs().len());
            let tx = tx
                .as_advanced_builder()
                .set_witnesses(witnesses.into_iter().map(|value| value.pack()).collect())
                .build();
            kit::assert_type_error(
                &mut f.context,
                tx,
                f.layout.state_type(),
                Error::Structure,
                40_000_000,
            );
        }
    }
}
