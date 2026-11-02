# Transaction Construction

Off-chain code reads input Cells, computes the expected state, and constructs
outputs and witnesses. `ckb-contract-tx` provides unsigned transaction builders
without a node or wallet dependency. Callers handle queries, Cell liveness, fees,
code dependencies, and wallet signing.

## Construction order

1. Read state, reserve, and funding inputs; verify full scripts, data, and OutPoints.
2. Create an Action, compute the successor with `apply`, and calculate payments and change.
3. Build outputs and put `State::encode()` in the corresponding output data.
4. Bind the Action to the actual State input's `WitnessArgs.input_type` and set any required `since`.
5. Complete code `cell_deps`, fees, and wallet witnesses, then sign and submit.

When a creation ID depends on the first input and State output position, fix
both before constructing script args. Changes to signed inputs, outputs, or
witness contents require signing again.

## Build a Counter update directly

This host-side function illustrates an update using a wallet Lock and Type
Script. Input 0 supplies funding; input 1 is the previous Counter. Output 0 is
CKB change; output 1 is the new Counter.

It assumes 300 CKB of State capacity and a 1000 CKB funding input, leaving a
1 CKB fee. Real clients should read actual input capacities and apply their fee
policy. Callers should compute and validate `after` with `before.apply(action)`.

```rust,no_run
use ckb_contract_core::{BinaryCodec, CKB};
use ckb_contract_tx::{
    ckb_types::{
        bytes::Bytes, core::{TransactionBuilder, TransactionView},
        packed::{CellOutput, OutPoint, Script}, prelude::*,
    },
    edit::with_action, input, BuildError, Cell,
};
use framework_counter_script::{Action, State, STATE_CAPACITY};

pub fn counter_update(
    state_input: OutPoint,
    funding_input: OutPoint,
    owner: &Script,
    state_type: &Script,
    after: &State,
    action: &Action,
) -> Result<TransactionView, BuildError> {
    let state_output = CellOutput::new_builder()
        .capacity(STATE_CAPACITY)
        .lock(owner.clone())
        .type_(Some(state_type.clone()).pack())
        .build();
    let tx = TransactionBuilder::default()
        .input(input(funding_input))
        .input(input(state_input))
        .output(Cell::capacity(owner, 999 * CKB).output)
        .output(state_output)
        .outputs_data([Bytes::new(), Bytes::from(after.encode())].pack())
        .build();
    with_action(tx, 1, action)
}
```

This function leaves code dependencies and signing to the caller. The wallet
Lock protocol determines the signature witness position; the actual State input
determines the Action position. They are not one shared, fixed index.
See the [Counter VM tests](../tests/src/tests.rs) for complete runnable transactions.

## Witness helpers

| Helper | Purpose |
|---|---|
| `action_witness(&action)` | Encode one `WitnessArgs.input_type` |
| `action_witnesses(&action, index, count)` | Create a witness array and check the index |
| `edit::with_action(tx, index, &action)` | Write or replace an input's Action |
| `edit::with_action_bytes(tx, index, bytes)` | Write an already encoded Action |
| `edit::replace_since(tx, index, since)` | Set an input's raw `since` value |

`with_action` preserves existing `WitnessArgs.lock`, `output_type`, and extra
witnesses. An existing witness must be valid WitnessArgs. Complete these edits
before signing.

## State with one Reserve

Applications choosing fixed output positions 0/1 can use `StateReserveLayout`,
`CellPair`, and `StateReserveTx`. Vault, Escrow, Limit Order, and Vesting use this
combination in transaction tests.

For Vault's `Deposit(100)`, supply the previous Cell pair, successor state, and
owner funding input, then rebuild the reserve and return token change:

```rust,no_run
use ckb_contract_core::CKB;
use ckb_contract_tx::{
    ckb_types::{core::TransactionView, packed::{OutPoint, Script}},
    input, BuildError, CellPair, StateReserveLayout, StateReserveTx,
};
use framework_vault_script::{Action, State};

pub fn deposit_100(
    layout: &StateReserveLayout,
    pair: CellPair,
    funding: OutPoint,
    owner: &Script,
    after: &State,
) -> Result<TransactionView, BuildError> {
    StateReserveTx::update(layout, pair, after, after.balance, &Action::Deposit(100))
        .input(input(funding))
        .state_input_index(1)
        .output(layout.token(owner, 999 * CKB, 900))
        .build()
}
```

This snippet assumes a funding input containing 1000 token units and 1000 CKB,
unchanged State/Reserve capacities, and a correctly computed successor state.
The layout's `token_type` must match the state's asset. The builder places
State and Reserve inputs as a consecutive block; the Action follows the State
position automatically.

| Constructor | Outputs |
|---|---|
| `StateReserveTx::create(layout, state, reserve_amount)` | New State and Reserve |
| `StateReserveTx::update(layout, pair, after, reserve_amount, action)` | Successor State and Reserve |
| `StateReserveTx::close(pair, action)` | Refund and other outputs supplied through `.output(...)` |

The builder does not replace on-chain reserve provenance, authorization, or
settlement checks. See the [Vault tests](../tests/src/vault.rs) for a full lifecycle.

## Multiple reserves and custom output positions

Use `TransactionLayout` to describe the application's positions and capacities,
then resolve full scripts through `ResolvedLayout`. AMM's `LAYOUT` contains one
State and two reserves:

```rust,no_run
use ckb_contract_core::Hash;
use ckb_contract_tx::{
    ckb_types::packed::Script, context::ResolvedLayout, BuildError,
};
use framework_amm_script::LAYOUT;

pub fn resolve_pool(
    code: &Script,
    id: &Hash,
    asset_types: &[Script; 2],
) -> Result<ResolvedLayout, BuildError> {
    ResolvedLayout::new(&LAYOUT, code, id, asset_types)
}
```

`ResolvedLayout` derives State/Reserve scripts using role + ID args. The asset
array follows `layout.reserves` order and contains full scripts, not just code
hashes or owner parameters. `CellSet::at(&LAYOUT, tx_hash)` recovers State and
reserve OutPoints from the previous output positions.

The following function builds a multiple-reserve update. Callers supply extra
inputs and outputs for the Action, including funding, payments, change, and LP
Cells:

```rust,no_run
use ckb_contract_tx::{
    ckb_types::{core::TransactionView, packed::CellInput},
    context::{CellSet, ContextTx, ResolvedLayout}, BuildError, Cell,
};
use framework_amm_script::{Action, State};

pub fn pool_update(
    layout: &ResolvedLayout,
    points: CellSet,
    after: &State,
    action: &Action,
    extra_inputs: Vec<CellInput>,
    extra_outputs: Vec<Cell>,
) -> Result<TransactionView, BuildError> {
    let mut tx = ContextTx::update(
        layout, points, after, &[after.reserve_x, after.reserve_y], action,
    )?;
    for input in extra_inputs { tx = tx.input(input); }
    for output in extra_outputs { tx = tx.output(output); }
    tx.build()
}
```

`ContextTx` checks reserve counts, declared predecessor positions, and their
shared transaction hash, and rejects duplicate inputs. It places fixed outputs
according to the layout and fills other positions with extra outputs. It does
not query live chain data or generate economic settlement outputs automatically.
A successfully built transaction still needs validation. See the
[AMM tests](../tests/src/amm.rs) for asset and LP settlement construction.

## Time constraints

Time-based applications bind the Action's timestamp to the actual State input's
`since`. Absolute timestamps use Unix seconds:

```rust
use ckb_contract_core::since::{absolute_timestamp, decode_absolute_timestamp};

let raw = absolute_timestamp(1_700_000_000).unwrap();
assert_eq!(decode_absolute_timestamp(raw), Ok(1_700_000_000));
```

State/Reserve builders expose `.state_since(raw)`; directly built transactions
can use `edit::replace_since`. The contract must read that same input's since
and check it against the Action. Encoding helpers validate format and range;
they do not establish that chain time satisfies the unlock condition.
See [Vesting's binding](../examples/vesting/contract/src/cells.rs) and
[VM tests](../tests/src/vesting.rs).

## Wallet and node integration

After preparing code deps, funding, fees, and authorization witnesses, let the
wallet sign and submit through your application's node interface. Obtain the
deployed code OutPoint, hash_type, and code hash from trusted deployment
information; builders do not discover or deploy contracts.

See [Counter Web](../examples/counter/web/README.md) for a frontend integration.

Next: [Testing and IDL](testing-and-idl.md).
