# Cells and Context

`#[derive(Cells)]` declares the Cells a validation step needs and generates the
collection's `load` and `schema` methods. `Context<C>` loads the collection and
the executing Script, then passes them to a business handler.

## Declare a collection

This example reuses Counter's data type and declares input and output states:

```rust,no_run
use ckb_contract_core::{Hash, CKB};
use ckb_contract_onchain::{
    context::{Cell, Cells, Context}, script, Source,
};
use ckb_std::ckb_types::packed::Script;
use framework_counter_script::State;

pub struct Params {
    pub state_type: Script,
    pub id: Hash,
}

#[derive(Cells)]
#[cells(args = Params)]
pub struct Update {
    #[cell(source = Source::Input,
        type_script = Some(&args.state_type), capacity = 300 * CKB,
        constraint = script::script_hash(before.lock()) == before.data().owner)]
    pub before: Cell<State>,
    #[cell(source = Source::Output,
        type_script = Some(&args.state_type), lock = before.lock(),
        capacity = 300 * CKB)]
    pub after: Cell<State>,
}

pub fn load_update(args: &Params) -> ckb_contract_core::Result<Context<Update>> {
    Context::<Update>::load(args)
}
```

Load parameters are caller-supplied scripts, positions, or other configuration;
they are separate from the transaction's Cells. Attribute expressions access
them through `args`. Fields load in declaration order, so a later field can
reference an earlier, validated field, as in `lock = before.lock()`.

## Field constraints

| Attribute | Example | Check |
|---|---|---|
| `source` | `Source::Input`, `Output`, `CellDep`, etc. | Read from the selected syscall source; required |
| `index` | `index = 0` | Check the Cell at that position |
| `lock` | `lock = &args.wallet` | Full Lock Script equality |
| `type_script` | `Some(&args.state_type)` | Full Type Script equality |
| `type_script` | `None::<&Script>` | Require the Type Script to be absent |
| `capacity` | `capacity = 300 * CKB` | Exact capacity equality |
| `constraint` | A Rust expression returning bool | Run after loading and decoding; return `Structure` on failure |

Provide at least one selector: `index`, `lock`, or `type_script`. Omitting
`type_script` leaves Type unrestricted; explicitly setting it to `None` requires
no Type. A capacity attribute checks an exact value, not a minimum.

Use a field predicate for other conditions, such as a minimum capacity:
`constraint = config.capacity() >= args.minimum_capacity`.

## Selection and uniqueness

Without `index`, the loader scans the source and selects Cells by the declared
full Lock/Type conditions:

- `Cell<T>` requires exactly one match. Missing or duplicate matches return `Structure`.
- `Option<Cell<T>>` allows no match but still rejects duplicates.
- After selection, the loader checks capacity, decodes data, and runs `constraint`.

A second Cell with matching scripts still creates ambiguity even if its capacity
or data is invalid. Capacity and decoding cannot hide duplicate matches. With an
explicit `index`, only that position is checked; add counting or other protocol
checks when global uniqueness is required.

Separate fields do not automatically prove that they selected distinct Cells.
When using several indexed fields or overlapping selectors, validate indices,
uniqueness, and relationships as required by the application.

## Optional Cells

Optional fields can cover creation, updates, and closing, or optional
configuration. This collection allows cell dep 1 to be absent. If present, it
must have no Type and satisfy the declared capacity and data requirements:

```rust,no_run
use ckb_contract_core::CKB;
use ckb_contract_onchain::{context::{Cell, Cells, RawData}, Source};
use ckb_std::ckb_types::packed::Script;

#[derive(Cells)]
pub struct OptionalConfig {
    #[cell(source = Source::CellDep, index = 1,
        type_script = None::<&Script>, capacity = 100 * CKB)]
    pub config: Option<Cell<RawData>>,
}
```

`RawData` preserves the original bytes without a business-specific decoder. An
existing indexed Cell with the wrong scripts fails validation. A present typed
`Cell<State>` with invalid data also fails; it does not become `None`. A predicate
on an optional field receives the whole Option and must handle absence itself.

## Reading a Cell

| Method | Result |
|---|---|
| `data()` | Decoded `&T` |
| `source()` / `index()` | Selected source and actual position |
| `lock()` | Full Lock Script |
| `type_script()` | Optional full Type Script |
| `capacity()` | Capacity in shannons |

These values support validation. Modifying a Rust copy of the data does not
modify the transaction's outputs.

## Authorization inputs

Handlers normally check the actor explicitly:

```rust,no_run
use ckb_contract_core::{Hash, Result};
use ckb_contract_onchain::context::InputAuthority;
use ckb_std::ckb_types::packed::Script;

fn require_owner(owner: Hash, protocol: &Script) -> Result<()> {
    InputAuthority::load_external(owner, protocol)?;
    Ok(())
}
```

`owner` is a full Lock Script hash. The input's Lock performs signature or other
authorization logic; `InputAuthority` checks that the input exists.
`load_external` also excludes the supplied protocol's code identity, including
Type-hash aliases, which is useful when the protocol has public Locks.

Authorization can also be declared as a field checked during collection loading:

```rust,no_run
use ckb_contract_core::Hash;
use ckb_contract_onchain::context::{Cells, InputAuthority};
use ckb_std::ckb_types::packed::Script;

pub struct AuthArgs { pub owner: Hash, pub protocol: Script }

#[derive(Cells)]
#[cells(args = AuthArgs)]
pub struct Authorized {
    #[authority(lock_hash = args.owner, protocol = &args.protocol)]
    pub owner: InputAuthority,
}
```

The business examples keep authorization, protocol rules, and settlement in
explicit functions so their validation order is visible.

## Collection parameters and errors

| Collection attribute | Meaning |
|---|---|
| `#[cells(args = Params)]` | Load parameter type; defaults to `()` and supports borrowed `Params<'a>` |
| `#[cells(error = AppError)]` | Explicit collection error type |
| `#[cells(constraint = expression)]` | Run a collection predicate after all fields load; return `Transition` on failure |
| `#[cells(no_mint)]` | Check for net issuance of official xUDT/sUDT during loading, with no exemptions |

Without an explicit error type, it is inferred from the first ordinary Cell's
`BinaryCodec::Error`. Authority-only collections default to framework `Error`.
Custom errors must implement `From<ckb_contract_core::Error>` and accept field
codec errors. A `Cells` collection uses concrete types; generic declarations are
not supported.

## From loading to protocol validation

`Context::load` executes only the declared checks. Applications explicitly
validate their remaining rules:

- Creation IDs, initial data, and allowed creation/update/close combinations.
- Exact equality between `before.apply(action)` and the actual successor state.
- Shared State/Reserve provenance and matching assets and amounts.
- Role counts and policies for other instances or additional Cells.
- Actor authorization, net payments, supply changes, and capacity refunds.

These checks can live in `verify_protocol(&ctx, &args)` and
`verify_settlement(&ctx)`. See
[Vault's declarations and protocol checks](../examples/vault/contract/src/cells.rs).

## Inspect the generated description

```rust
use ckb_contract_onchain::context::Cells;
use framework_counter_script::cells::Update;

let schema = Update::schema();
assert_eq!(schema.fields.len(), 2);
assert_eq!(schema.name, "Update");
```

The schema describes declared fields and constraint expressions. It does not
analyze additional Rust code in handlers. See [Testing and IDL](testing-and-idl.md)
for export options.

Next: [State and Codecs](state-and-codecs.md).
