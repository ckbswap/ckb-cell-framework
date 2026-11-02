# API and Command Reference

## CLI

Run commands inside the framework workspace. The entry point is implemented in
[`crates/cli/src/main.rs`](../crates/cli/src/main.rs).

| Command | Arguments and outputs |
|---|---|
| `cargo cell help` | Show supported commands |
| `cargo cell new my-app` | Generate `contracts/my-app/{contract,tests}` and register workspace members |
| `cargo cell build` | Build the default `framework-counter-script` |
| `cargo cell build framework-amm-script` | Build a package's same-named VM binary at `build/<package>` |
| `cargo cell test` | Check formatting, host tests, tools, builds, IDL, and VM validation |
| `cargo cell idl framework-vesting-script` | Export a registered complete interface |
| `cargo cell idl framework-vesting-script --out path.json` | Select an interface output path |

A VM binary has the same name as its package and uses the `vm` feature. After
generating a project, run `cargo check -p <name>-script` to update the lockfile.

## Data and math

Provided by [`ckb-contract-core`](../crates/core/src/lib.rs):

| API | Purpose |
|---|---|
| `Hash` | `[u8; 32]` |
| `CKB` | Shannons per CKB: `100_000_000` |
| `BinaryCodec` | `encode` and `decode` |
| `StateTransition` | Compute the expected successor with `apply(&self, action)` |
| `ckb_hash` / `entity_id` | Hash script bytes and derive instance IDs from input/output positions |
| `math::mul_div_floor` / `mul_div_ceil` | Wide multiplication/division with explicit rounding |
| `math::sqrt_product` / `U256` | Product square roots and wide integer arithmetic |
| `since::absolute_timestamp` / `decode_absolute_timestamp` | Canonical absolute timestamp since encoding and decoding |

Net flow and supply checks:

```rust
use ckb_contract_core::{check_flow, check_supply};

// outputs + spent == inputs + received
assert_eq!(check_flow(100, 80, 30, 10), Ok(()));
// outputs + burned == inputs + minted
assert_eq!(check_supply(100, 110, 10, 0), Ok(()));
```

Both functions check addition overflow. For `check_flow`, before/after are the
party's actual aggregated balances; `spent` and `received` are the expected net
payments and receipts.

## On-chain Cells and scripts

Provided by [`ckb-contract-onchain`](../crates/onchain/src/lib.rs):

| API | Purpose |
|---|---|
| `context::{Cell, Cells, Context}` | Typed fields, derived collections, and the execution container |
| `context::RawData` | Original bytes without business-specific decoding |
| `context::InputAuthority` | Match an authorization input; `load_external` excludes protocol code |
| `cells::find` / `exactly_one` | Find Cells by full Lock or Type |
| `cells::count_role` | Count matching code_hash/hash_type and role across all instance IDs |
| `cells::action::<A>(index)` | Decode the actual State input's `WitnessArgs.input_type` |
| `cells::creation_id(output_index)` | Derive the ID from the first input and output position |
| `cells::require_same_origin(state, companion, output_index)` | Check a shared predecessor transaction and the companion's output position |
| `cells::transaction_fee` | Total input capacity minus total output capacity |
| `cells::require_payment(owner, expected, allowance)` | Require owner's net capacity receipts plus allowance to reach expected |
| `script::script_hash` | Hash a full Script |
| `script::with_role` | Preserve code identity and construct role + ID + payload args |
| `script::plain_xudt` | Construct official data1 xUDT with zero flags and 36-byte args |
| `state::reject_type_as_input_lock` | Reject a full Script used in any input Lock field |

`script::with_role` and role counting implement an optional application argument
scheme. Generic `Cells` reads the declared scripts and fields; it does not choose
roles or output layouts automatically.

## Asset helpers

| API | Purpose |
|---|---|
| `core::token::PlainXudtAsset` | Official plain xUDT owner parameter and original 32/36-byte args form |
| `core::token::AmountEncoding::Exact16` | Require exactly 16 bytes |
| `core::token::AmountEncoding::Prefix16` | Read the first 16 bytes and allow trailing data |
| `onchain::token::total` | Sum by full Type hash, optionally filtered by wallet Lock hash |
| `onchain::token::require_supply_change` | Check exact mint/burn amounts using prefix amounts |
| `onchain::token::reject_standard_mint` | Reject net issuance of official xUDT/sUDT, with optional full Type hash exemptions |

Here `core` and `onchain` refer to their respective framework crates. The net
issuance guard covers official binary code identities; custom tokens require
their own issuance validation. Exempted Types need separate exact supply and
party flow checks, as in [AMM's LP settlement](../examples/amm/contract/src/lib.rs).

## Off-chain helpers

| API | Purpose |
|---|---|
| `tx::Cell` | Keep output and data together; construct capacity and token Cells |
| `tx::StateReserveTx` | Build a State + single Reserve transaction at output positions 0/1 |
| `tx::context::ResolvedLayout` | Resolve role layouts and full asset scripts into concrete scripts |
| `tx::context::CellSet` | State and reserve OutPoints from one predecessor transaction |
| `tx::context::ContextTx` | Build creation, update, and close transactions from a layout |
| `tx::edit` | Edit Actions, since, outputs, and data with bounds checks |

Here `tx` means `ckb-contract-tx`. Applications handle code dependencies, UTXO
liveness, economic rules, fees, and signing. See [Transaction Construction](transactions.md).

## Error codes

Framework `Error` uses `repr(i8)`. Example VM entry points map it directly to an
exit code.

| Error | Code | Common cause |
|---|---:|---|
| `Encoding` | 5 | Invalid bytes, prefix, tag, args, or since format |
| `Arithmetic` | 6 | Integer overflow or an unrepresentable arithmetic result |
| `Structure` | 7 | Invalid Cell selection, count, position, provenance, scripts, or syscall result |
| `Authorization` | 8 | Missing actor input or a public protocol Lock impersonating a wallet |
| `Capacity` | 9 | Wrong capacity or insufficient close refund |
| `Token` | 10 | Wrong token amount, asset, supply, or net payment |
| `Genesis` | 11 | Invalid creation ID or initial data |
| `Transition` | 12 | Forbidden Action, wrong state, or failed collection-level business predicate |

Applications can define an error type implementing `From<Error>` and convert
framework failures in handlers. Select it for a derived collection with
`#[cells(error = AppError)]`, then explicitly map application errors to stable
nonzero VM exit codes. Different scripts may use different error conventions;
VM tests should identify the target script's expected rejection.

## Implementation locations

- [Collection derive](../crates/derive/src/cell.rs) and [codec derive](../crates/derive/src/codec.rs).
- [Context and Cell loader](../crates/onchain/src/context.rs).
- [Net flows and errors](../crates/core/src/lib.rs).
- [Transaction builders](../crates/tx/src/lib.rs) and [multiple-reserve builders](../crates/tx/src/context.rs).
- [Test helpers](../crates/testkit/src/lib.rs) and [IDL export](../crates/idl/src/lib.rs).

Return to the [documentation home](README.md).
