//! Execute the actual React client's CCC transaction builders against CKB-VM.
use ckb_contract_core::{BinaryCodec, Error, CKB};
use ckb_contract_testkit::{
    self as kit,
    ckb_testtool::{
        ckb_types::{bytes::Bytes, core::TransactionView, packed, prelude::*},
        context::Context,
    },
};
use framework_counter_script::State;
use std::process::Command;

fn hex(data: &[u8]) -> String {
    format!(
        "0x{}",
        data.iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn transaction(value: &serde_json::Value) -> TransactionView {
    let bytes: Vec<_> = value
        .as_str()
        .unwrap()
        .strip_prefix("0x")
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect();
    packed::Transaction::from_slice(&bytes).unwrap().into_view()
}

#[test]
#[ignore = "requires npm ci; run npm run test:vm in examples/counter/web"]
fn ccc_web_transactions_execute_in_ckb_vm() {
    let root = kit::workspace_root(env!("CARGO_MANIFEST_DIR"));
    let web = root.join("examples/counter/web");
    let mut context = Context::default();
    let code = context.deploy_cell(kit::load_binary(&root, "framework-counter-script"));
    let wallet = kit::wallets(&mut context)[0].clone();
    let owner_dep = context.cells_by_data_hash.get(&wallet.code_hash()).unwrap();
    let result = Command::new("node")
        .args(["--import", "tsx"])
        .arg(web.join("scripts/export-vm.ts"))
        .current_dir(&web)
        .env("CCC_VM_OWNER_SCRIPT", hex(wallet.as_slice()))
        .env("CCC_VM_CODE_OUT_POINT", hex(code.as_slice()))
        .env("CCC_VM_OWNER_DEP", hex(owner_dep.as_slice()))
        .output()
        .expect("Node.js is required for the CCC integration test");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let fixture: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let creation = transaction(&fixture["creation"]);
    let update = transaction(&fixture["update"]);
    for tx in [&creation, &update] {
        // Only the owner-locked State is carried over. Wallet funding Cells are
        // deterministic fixtures; no private key or network access is involved.
        let input = if tx.hash() == creation.hash() { 0 } else { 1 };
        let cell = kit::tx::Cell::capacity(&wallet, 1000 * CKB);
        context.create_cell_with_out_point(
            tx.inputs().get(input).unwrap().previous_output(),
            cell.output,
            Bytes::new(),
        );
    }
    let verified = kit::assert_valid(&mut context, creation.clone(), 30_000_000);
    assert_eq!(
        verified.hash(),
        creation.hash(),
        "fixture must include all code deps before computing its successor"
    );
    kit::commit_outputs(&mut context, &verified);
    assert_eq!(
        update.inputs().get(0).unwrap().previous_output().tx_hash(),
        creation.hash()
    );
    let verified = kit::assert_valid(&mut context, update.clone(), 30_000_000);
    let state = State::decode(&verified.outputs_data().get(0).unwrap().raw_data()).unwrap();
    assert_eq!((state.nonce, state.value), (1, 7));
    let owner: [u8; 32] = wallet.calc_script_hash().unpack();
    assert_eq!(state.owner, owner);
    let mut invalid = state;
    invalid.value += 1;
    let tampered = kit::replace_data(update, 0, Bytes::from(invalid.encode()));
    kit::assert_error(&mut context, tampered, Error::Transition, 30_000_000);
}
