# Program Structure

A contract normally separates data, Cell collections, business handlers, and VM
startup. The snippets in this chapter use the repository's Counter types; see the
[complete Counter contract](../examples/counter/contract/src/lib.rs) for the full
implementation. Generated projects have the same structure; replace the example
package paths with your own package name.

## Project files

```text
contract/src/
├── state.rs    Data, codec derives, and pure transitions
├── cells.rs    Load parameters, Cell collections, and field constraints
├── lib.rs      Script dispatch, handlers, protocol and settlement checks
└── main.rs     CKB-VM startup and exit-code mapping
```

`lib.rs` is a `#![no_std]` library, usually with `extern crate alloc`, shared by
VM startup and host-side code. `main.rs` builds separately through the `vm`
feature, while host tests exercise the library directly.

## What a Cell contains

A Cell has capacity, a Lock Script, an optional Type Script, and data. Clients
spend input Cells and propose new output Cells. Contracts read the transaction
and decide whether it is valid.

| Component | Use in a contract |
|---|---|
| capacity | CKB capacity in shannons; `CKB = 100_000_000` |
| Lock | Constrain spending of an input Cell |
| Type | Validate creation, updates, destruction, assets, and other rules |
| data | Application-defined bytes, optionally encoded with `CellCodec` |
| OutPoint | A previous transaction hash and output index identifying a Cell to spend |

Counter places the owner's wallet Lock on its State Cell and uses the Counter
script in the Type field. Its data contains `owner`, `nonce`, and `value`. After
the client constructs a successor output, the contract checks that its data
equals the result of applying the Action to the previous state.

## Define the transition first

`StateTransition::apply` computes a value. It does not read the transaction or
modify a live Cell. Clients and contracts can reuse the same calculation:

```rust
use ckb_contract_core::StateTransition;
use framework_counter_script::{Action, State};

let before = State::new([1; 32]);
let action = Action { increment: 7 };
let after = before.apply(&action).unwrap();
assert_eq!(after.value, 7);
assert_eq!(after.nonce, 1);
assert_eq!(after.owner, before.owner);
```

See [State and Codecs](state-and-codecs.md) for data definitions and the complete
transition implementation.

## Receive Cells through Context

The framework provides a shared container:

```rust
use ckb_contract_onchain::context::Context;
use framework_counter_script::cells::Update;

fn proposed_value(ctx: &Context<Update>) -> u64 {
    ctx.cells.after.data().value
}
```

`ctx.cells` contains the application's declared fields. `ctx.script` contains the
full executing Script. `Context::<Update>::load(&args)` loads that script and
invokes the collection's loader.

Counter's `Create` declares one output state; `Update` declares input and output
states. Field constraints check full script identity, capacity, decoded data, and
wallet Lock relationships. See [Cells and Context](cells-and-context.md) for the
declarations.

## Creation handler

Creation verifies the instance ID, initial data, and an owner authorization
input. This function uses Counter's `Create` and `CounterArgs`:

```rust,no_run
use ckb_contract_core::{Error, Result};
use ckb_contract_onchain::{
    cells, context::{Context, InputAuthority}, state as state_cells, token,
};
use framework_counter_script::{cells::{CounterArgs, Create}, State};

pub fn create(ctx: Context<Create>, args: &CounterArgs) -> Result<()> {
    let after = &ctx.cells.after;
    state_cells::reject_type_as_input_lock(&ctx.script)?;
    token::reject_standard_mint(&[])?;
    if cells::creation_id(after.index())? != args.id
        || *after.data() != State::new(after.data().owner)
    {
        return Err(Error::Genesis);
    }
    InputAuthority::load_external(after.data().owner, &ctx.script)?;
    Ok(())
}
```

Counter uses a raw 32-byte ID in script args. The creation ID is derived from the
Molecule bytes of the first `CellInput` and the state output position. Clients
must fix that first input and output position before constructing the Type
Script args.

## Update handler

After loading both states, verify the complete expected state and authorization:

```rust,no_run
use ckb_contract_core::{Error, Result, StateTransition};
use ckb_contract_onchain::{
    cells, context::{Context, InputAuthority}, state as state_cells, token,
};
use framework_counter_script::{cells::Update, Action};

pub fn update(ctx: Context<Update>) -> Result<()> {
    let before = &ctx.cells.before;
    let after = &ctx.cells.after;
    state_cells::reject_type_as_input_lock(&ctx.script)?;
    token::reject_standard_mint(&[])?;
    let action = cells::action::<Action>(before.index())?;
    if before.data().apply(&action)? != *after.data() {
        return Err(Error::Transition);
    }
    InputAuthority::load_external(before.data().owner, &ctx.script)?;
    Ok(())
}
```

`before.index()` is the actual State input position. Counter puts the Action in
that input's `WitnessArgs.input_type`; collection loading does not infer this
calling convention.

## Entry dispatch

`verify()` parses the executing script's arguments, selects a collection, and
calls its business handler. Counter distinguishes creation and updates by looking
for a matching Type input:

```rust,no_run
use ckb_contract_core::{Error, Result};
use ckb_contract_onchain::{
    cells::{self, ScriptField}, context::Context, syscall, Source,
};
use ckb_std::high_level::load_script;
use framework_counter_script::{
    cells::{CounterArgs, Create, Update}, create, update,
};

pub fn verify() -> Result<()> {
    let current = syscall(load_script())?;
    let raw_args = current.args().raw_data();
    let id: [u8; 32] = raw_args.as_ref().try_into().map_err(|_| Error::Encoding)?;
    let inputs = cells::find(&current, Source::Input, ScriptField::Type)?;
    let args = CounterArgs { script: current, id };
    if inputs.is_empty() {
        create(Context::<Create>::load(&args)?, &args)
    } else {
        update(Context::<Update>::load(&args)?)
    }
}
```

Counter's collections require one state output for creation and exactly one
input and one output for updates. This protocol has no close operation or fixed
update output position.

## VM startup

`main.rs` starts the VM and maps errors to exit codes. This code targets the
RISC-V contract binary:

```rust,ignore
#![no_std]
#![no_main]

ckb_std::entry!(entry);
ckb_std::default_alloc!();

fn entry() -> i8 {
    framework_counter_script::verify().map_or_else(|error| error as i8, |_| 0)
}
```

Zero means success; a nonzero exit rejects the transaction. See the
[error reference](reference.md) for framework codes.

## Asset contract handlers

Vault and the other asset examples call protocol and settlement functions
explicitly, in this order:

```text
verify_account(Context<VaultCells>, &VaultArgs)
├── cells::verify_protocol(&ctx, args)
└── verify_settlement(&ctx)
```

The protocol function checks lifecycle rules, reserve provenance, and actor
authorization. Settlement checks asset supply, the owner's net payments, and
capacity refunds. See [Vault's `verify_account`](../examples/vault/contract/src/lib.rs)
for the full implementation.

Next: [Cells and Context](cells-and-context.md).
