# Testing and IDL

Pure transitions can be tested directly on the host. Transaction structure,
authorization inputs, reserves, and settlement need validation against actual
CKB-VM binaries. Data and collection schemas can be exported as JSON on the host
for clients to inspect.

## Pure logic and codec tests

Calculations in `state.rs` do not depend on syscalls, so tests can check inputs,
expected values, and rejection paths directly:

```rust
use ckb_contract_core::{BinaryCodec, Error, StateTransition};
use framework_counter_script::{Action, State};

let before = State::new([1; 32]);
let after = before.apply(&Action { increment: 7 }).unwrap();
assert_eq!(after.value, 7);
assert_eq!(after.owner, before.owner);
assert_eq!(before.apply(&Action { increment: 0 }), Err(Error::Transition));
assert_eq!(State::decode(&after.encode()), Ok(after));

let mut bytes = before.encode();
bytes.push(0);
assert_eq!(State::decode(&bytes), Err(Error::Encoding));
```

Put assertions like these in `#[test]` functions and run:

```sh
cargo test -p framework-counter-script
```

Alongside round-trip tests, verify independent byte vectors so simultaneous
changes to encode and decode cannot silently change the protocol. See the
[Vault codec tests](../examples/vault/contract/tests/encoding.rs).

## Prepare VM tests

VM tests load the actual RISC-V binary from `build/<package>`, so build the
contract first:

```sh
cargo cell build framework-counter-script
cargo test -p framework-vm-tests --features vm-tests tests::
```

Counter's [complete fixture](../tests/src/tests.rs) follows these steps:

1. Create a testtool environment with `Context::default()`.
2. Register contract code with `deploy_cell(load_binary(...))`.
3. Create wallets and funding Cells, then derive the ID from the funding input and creation output position.
4. Construct and validate creation, then register its actual outputs.
5. Consume those outputs for updates and test successor states and rejection paths.

`kit::wallets` uses `ALWAYS_SUCCESS` test wallets. It tests Lock hash matching and
business validation, not the correctness of a real wallet's signature implementation.

## Test helpers

```rust,no_run
use ckb_contract_testkit::{
    self as kit,
    ckb_testtool::{context::Context, ckb_types::core::TransactionView},
};

fn verify_and_commit(context: &mut Context, tx: TransactionView) -> TransactionView {
    let tx = kit::assert_valid(context, tx, 40_000_000);
    kit::commit_outputs(context, &tx);
    tx
}
```

| Helper | Purpose |
|---|---|
| `workspace_root(env!("CARGO_MANIFEST_DIR"))` | Locate the framework workspace |
| `load_binary(root, package)` | Read `build/<package>` |
| `deploy_xudt(context, root, asset)` | Deploy and verify the pinned official xUDT binary and construct the full Type |
| `assert_valid(context, tx, cycles)` | Complete test code dependencies, verify, and return the completed transaction |
| `assert_error(context, tx, error, cycles)` | Check the actual VM validation error code |
| `assert_type_error(context, tx, script, error, cycles)` | Require transaction rejection and check the selected Type's error |
| `commit_outputs(context, tx)` | Remove spent inputs and register outputs under their actual transaction hash |
| `replace_data` / `replace_output` / `replace_since` | Construct invalid transaction vectors |

When several scripts could reject a modified transaction, use `assert_type_error`
to confirm that the target contract enforced the intended rule. A cycle limit
or missing dependency must not stand in for the expected business rejection;
checking formatted error strings alone is insufficient.

## Test complete lifecycles

Consecutive creation, update, and close tests should spend the previous step's
actual OutPoints. When State and Reserve must share a predecessor transaction,
creating two arbitrary inputs with matching contents does not establish that
provenance rule.

Cover authorization, wire format, transitions, and actual settlement for each
operation:

| Layer | Common cases |
|---|---|
| Data | Valid values, zero amounts, overflow, wrong prefixes, truncation, trailing bytes |
| Cell structure | Missing or duplicate Cells, wrong scripts or capacity, output positions, reserve provenance |
| Transitions | Initial values, immutable fields, nonces, terminal states, forbidden lifecycles |
| Authorization | Correct and incorrect actors, public protocol Locks used as wallet identities |
| Settlement | Exact payments, net balances, change, wrong recipients, supply changes, capacity refunds |

See the [Vault](../tests/src/vault.rs), [Escrow](../tests/src/escrow.rs),
[Limit Order](../tests/src/limit_order.rs), and [AMM](../tests/src/amm.rs) tests
for concrete assertions and transaction construction.

## Validate the workspace

```sh
cargo cell test
```

The command checks formatting, runs host tests for default workspace members
and Python tool tests, builds all registered RISC-V contracts, exports registered
IDLs, verifies the official xUDT cache, and runs registered VM test packages.

A contract identifies its VM test package through metadata, which the scaffold
fills automatically:

```toml
[package.metadata.ckb-contract]
vm-tests = "my-counter-vm-tests"
```

The test package must be a workspace member and declare a `vm-tests` feature.
See [`scripts/test-contracts.py`](../scripts/test-contracts.py) for discovery and
registration checks.

## Two schema sources

`CellCodec` generates `CodecSchema::SCHEMA` for data. `Cells` generates a
collection's `schema()`. Export a generic Cell collection directly:

```rust
use ckb_contract_onchain::context::Cells;
use framework_counter_script::cells::Update;

let json = ckb_contract_idl::export_context(&Update::schema()).unwrap();
assert!(json.contains("Update"));
```

This JSON contains collection fields and declared constraints. Rust expressions
remain descriptive text; the exporter neither executes them nor analyzes
business, authorization, or asset checks in handlers. For example, a handler
may explicitly check net issuance while its collection schema reports
`reject_standard_mint: false`, because the collection does not declare
`#[cells(no_mint)]`.

## Export a complete interface

The current complete interface exporter uses `Interface`: State encoding,
a tagged Action enum, a role layout, Cell field bindings, and explicit operation
authority and timestamp bindings.
[Vesting's `interface()`](../examples/vesting/contract/src/lib.rs) is a runnable
example:

```sh
cargo cell idl framework-vesting-script
cargo cell idl framework-vesting-script --out build/idl/vesting.json
```

The default output is `build/idl/framework-vesting-script.json`. The exporter runs
on the host and needs no node.

| Interface field | Description |
|---|---|
| `state` / `action` | Fields, tags, prefixes, and wire format |
| `layout` | State/Reserve roles, capacity, and output positions |
| `context` | Collection fields and declared constraints |
| `bindings` | State/Reserve output field names in a generic collection |
| `instructions` | Operation name, creation/update/close phase, Action, authority field, and optional timestamp field |

`Instruction.authority` references a 32-byte Lock hash field path in State;
`timestamp` references a `u64` field in the corresponding update Action.
Applications supply these bindings. The exporter checks that the referenced
fields and types exist. Capacities are decimal strings to preserve precision
in JavaScript.

The complete format uses role + ID args, a State/Reserve layout, and the actual
State input's witness. Counter uses raw IDs and a struct Action; its collections
can be described with `export_context`, and it currently has no registered
complete interface exporter. The CLI runs registered exporters rather than
inferring arbitrary package interfaces.

## Register an exporter for your package

Applications using the complete interface format can merge configuration like
Vesting's into their existing Cargo.toml:

```toml
[package.metadata.ckb-contract]
vm-tests = "my-app-vm-tests"
idl = "my-app-idl"

[features]
vm = []
idl = ["dep:ckb-contract-idl"]

[[bin]]
name = "my-app-idl"
path = "src/bin/idl.rs"
required-features = ["idl"]
test = false
bench = false

[dependencies]
ckb-contract-idl = { workspace = true, optional = true }
```

Expose the application's `interface()` behind `#[cfg(feature = "idl")]` in the
library. The host binary parses the output path and calls
`ckb_contract_idl::write`; see [Vesting's export entry point](../examples/vesting/contract/src/bin/idl.rs).
Run `cargo check -p my-app-script --features idl` to update the lockfile, then
export with `cargo cell idl my-app-script`.

The JSON describes protocol bytes and explicit bindings. Clients still preserve
full script identity, supply deployment information, construct settlement
outputs, and sign transactions.

Next: [API and Command Reference](reference.md).
