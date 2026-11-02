use ckb_contract_core::{BinaryCodec, Error, StateTransition, CKB};
use ckb_contract_testkit::{
    self as kit,
    ckb_testtool::{
        ckb_types::{
            bytes::Bytes,
            core::{ScriptHashType, TransactionBuilder, TransactionView},
            packed::{CellDep, CellInput, CellOutput, Script},
            prelude::*,
        },
        context::Context,
    },
};
use framework_counter_script::{Action, State};

struct Fixture {
    context: Context,
    script: Script,
    wallet: Script,
}
impl Fixture {
    fn new() -> Self {
        let root = kit::workspace_root(env!("CARGO_MANIFEST_DIR"));
        let mut context = Context::default();
        let code = context.deploy_cell(kit::load_binary(&root, "framework-layout-test-script"));
        let script = context
            .build_script_with_hash_type(&code, ScriptHashType::Data2, Bytes::new())
            .unwrap();
        let wallet = kit::wallets(&mut context)[0].clone();
        Self {
            context,
            script,
            wallet,
        }
    }

    fn batch(&mut self, config_data: &[u8]) -> TransactionView {
        let mut builder = TransactionBuilder::default();
        for (value, increment) in [(5, 1), (10, 2)] {
            let mut before = State::new(self.wallet.calc_script_hash().unpack());
            before.value = value;
            let output = CellOutput::new_builder()
                .capacity(300 * CKB)
                .lock(self.wallet.clone())
                .type_(Some(self.script.clone()).pack())
                .build();
            let point = self
                .context
                .create_cell(output.clone(), Bytes::from(before.encode()));
            builder = builder
                .input(CellInput::new_builder().previous_output(point).build())
                .output(output)
                .output_data(
                    Bytes::from(before.apply(&Action { increment }).unwrap().encode()).pack(),
                );
        }
        let config = self.context.create_cell(
            CellOutput::new_builder()
                .capacity(100 * CKB)
                .lock(self.script.clone())
                .build(),
            Bytes::from(config_data.to_vec()),
        );
        builder
            .cell_dep(CellDep::new_builder().out_point(config).build())
            .build()
    }
}

#[test]
fn context_validates_multiple_states_and_a_raw_dependency_without_roles() {
    let mut f = Fixture::new();
    let tx = f.batch(&[42]);
    kit::assert_valid(&mut f.context, tx.clone(), 30_000_000);
    let mut outputs: Vec<_> = tx.outputs().into_iter().collect();
    outputs[1] = outputs[1]
        .clone()
        .as_builder()
        .type_(None::<Script>.pack())
        .build();
    kit::assert_error(
        &mut f.context,
        tx.clone()
            .as_advanced_builder()
            .set_outputs(outputs)
            .build(),
        Error::Structure,
        30_000_000,
    );
    let mut data: Vec<_> = tx.outputs_data().into_iter().collect();
    data.swap(0, 1);
    kit::assert_error(
        &mut f.context,
        tx.clone()
            .as_advanced_builder()
            .set_outputs_data(data)
            .build(),
        Error::Transition,
        30_000_000,
    );
    let mut data: Vec<_> = tx.outputs_data().into_iter().collect();
    let mut malformed = data[1].raw_data().to_vec();
    malformed.push(0);
    data[1] = Bytes::from(malformed).pack();
    kit::assert_error(
        &mut f.context,
        tx.as_advanced_builder().set_outputs_data(data).build(),
        Error::Encoding,
        30_000_000,
    );
    let bad_config = f.batch(&[41]);
    kit::assert_error(&mut f.context, bad_config, Error::Structure, 30_000_000);
}

#[test]
fn dependency_selection_checks_absent_type_and_rejects_duplicate_scripts() {
    let mut f = Fixture::new();
    let tx = f.batch(&[42]);
    let extra = f.context.create_cell(
        CellOutput::new_builder()
            .capacity(101 * CKB)
            .lock(f.script.clone())
            .build(),
        Bytes::from(vec![0]),
    );
    let duplicate = tx
        .clone()
        .as_advanced_builder()
        .cell_dep(CellDep::new_builder().out_point(extra).build())
        .build();
    kit::assert_error(&mut f.context, duplicate, Error::Structure, 30_000_000);
    let wrong_type = f.context.create_cell(
        CellOutput::new_builder()
            .capacity(100 * CKB)
            .lock(f.script.clone())
            .type_(Some(f.script.clone()).pack())
            .build(),
        Bytes::from(vec![42]),
    );
    let changed = tx
        .as_advanced_builder()
        .set_cell_deps(vec![CellDep::new_builder().out_point(wrong_type).build()])
        .build();
    kit::assert_error(&mut f.context, changed, Error::Structure, 30_000_000);
}

#[test]
fn optional_cell_absence_does_not_hide_bad_present_cells() {
    let mut f = Fixture::new();
    let tx = f.batch(&[42]);
    let extra = CellOutput::new_builder()
        .capacity(300 * CKB)
        .lock(f.wallet.clone())
        .build();
    let valid = tx
        .clone()
        .as_advanced_builder()
        .output(extra.clone())
        .output_data(Bytes::from(State::new([1; 32]).encode()).pack())
        .build();
    kit::assert_valid(&mut f.context, valid, 30_000_000);
    let malformed = tx
        .clone()
        .as_advanced_builder()
        .output(extra.clone())
        .output_data(Bytes::from(vec![1]).pack())
        .build();
    kit::assert_error(&mut f.context, malformed, Error::Encoding, 30_000_000);
    let typed = tx
        .as_advanced_builder()
        .output(
            extra
                .as_builder()
                .type_(Some(f.script.clone()).pack())
                .build(),
        )
        .output_data(Bytes::from(State::new([1; 32]).encode()).pack())
        .build();
    kit::assert_error(&mut f.context, typed, Error::Structure, 30_000_000);
}
