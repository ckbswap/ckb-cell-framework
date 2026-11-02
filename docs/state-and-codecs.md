# State and Codecs

Applications define State and Action fields; `CellCodec` generates their
encoding and decoding. `StateTransition` computes the expected successor from
the previous state and an Action. Handlers still compare the actual output and
validate authorization and asset transfers separately.

## Define data

This is Counter's data and transition logic, runnable directly on the host:

```rust
use ckb_contract_core::{BinaryCodec, Error, Hash, Result, StateTransition};
use ckb_contract_derive::CellCodec;

#[derive(CellCodec, Clone, Debug, PartialEq, Eq)]
#[codec(prefix = b"CNT01")]
pub struct State {
    pub owner: Hash,
    pub nonce: u64,
    pub value: u64,
}

#[derive(CellCodec, Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub increment: u64,
}

impl StateTransition for State {
    type Action = Action;

    fn apply(&self, action: &Action) -> Result<Self> {
        if action.increment == 0 {
            return Err(Error::Transition);
        }
        Ok(Self {
            owner: self.owner,
            nonce: self.nonce.checked_add(1).ok_or(Error::Arithmetic)?,
            value: self.value.checked_add(action.increment).ok_or(Error::Arithmetic)?,
        })
    }
}

let before = State { owner: [1; 32], nonce: 0, value: 0 };
let after = before.apply(&Action { increment: 7 }).unwrap();
let bytes = after.encode();
assert_eq!(bytes.len(), 53);
assert_eq!(State::decode(&bytes), Ok(after));
```

The State encoding contains a 5-byte `CNT01` prefix, a 32-byte owner, an 8-byte
nonce, and an 8-byte value. Action declares no prefix, so its encoding is just
the 8-byte increment. Import the `BinaryCodec` trait to call `encode` and `decode`.

## What CellCodec generates

| Trait | Responsibility |
|---|---|
| `BinaryCodec` | Top-level `encode` and strict `decode`, including the optional prefix |
| `WireValue` | Inline body encoding when embedded in another value |
| `CodecSchema` | The `SCHEMA` constant describing fields, types, prefixes, and enum tags |

The generated `BinaryCodec::Error` is framework `Error`. Decoding validates the
byte format; it does not enforce business rules for amounts, permissions, or
asset relationships.

## Actions with multiple operations

Use an enum with explicit tags for multiple operations. This shape matches
Vault's Action:

```rust
use ckb_contract_core::BinaryCodec;
use ckb_contract_derive::CellCodec;

#[derive(CellCodec, Debug, PartialEq, Eq)]
#[codec(prefix = b"VLA01")]
pub enum Action {
    #[codec(tag = 0)]
    Deposit(u128),
    #[codec(tag = 1)]
    Withdraw(u128),
    #[codec(tag = 2)]
    Close,
}

assert_eq!(Action::Deposit(10).encode().len(), 22);
assert_eq!(Action::Close.encode(), b"VLA01\x02");
assert_eq!(Action::decode(b"VLA01\x03"), Err(ckb_contract_core::Error::Encoding));
```

Each variant needs a unique `u8` tag. Variants may have no fields, named fields,
or tuple fields, such as `Close`, `Fill { taker: Hash, amount: u128 }`, and
`Deposit(u128)`.

## Supported data types

| Type | Encoding |
|---|---|
| `u8` / `u32` / `u64` / `u128` | Little-endian integers of the corresponding width |
| `bool` | One byte; only 0 or 1 is accepted |
| `[u8; N]`, including `Hash` | N bytes, unchanged |
| `PlainXudtAsset` | A 32-byte owner followed by the `extended_args` bool |
| Named struct deriving `CellCodec` | Fields in declaration order |
| Enum deriving `CellCodec` | A `u8` tag followed by the selected variant's fields |

Fields may also use other types implementing `WireValue`. The framework does
not currently provide automatic wire implementations for `Vec<T>`, `String`,
general `Option<T>`, or arbitrary integer types. Derived types do not support
generics, tuple structs, or unit structs.

`Option<Cell<T>>` in a Cell collection means the selected Cell may be absent; it
is separate from encoding an `Option<T>` in data.

## Nested data and prefixes

Nested structures use the same derive. An embedded value writes only its body,
without repeating its top-level prefix:

```rust
use ckb_contract_core::BinaryCodec;
use ckb_contract_derive::CellCodec;

#[derive(CellCodec)]
#[codec(prefix = b"INNER")]
struct Inner { value: u64 }

#[derive(CellCodec)]
#[codec(prefix = b"OUT")]
struct Outer { inner: Inner }

assert_eq!(Inner { value: 7 }.encode().len(), 5 + 8);
assert_eq!(Outer { inner: Inner { value: 7 } }.encode().len(), 3 + 8);
```

Applications choose prefixes explicitly to identify data types or protocol
versions. Prefixes are not generated automatically and do not define migration
rules.

## Strict decoding and business validation

Generated decoders reject mismatched prefixes, truncated fields, trailing bytes,
unknown enum tags, and invalid bools. After decoding, applications still check:

- Initial state values required by the protocol.
- Business ranges for nonces, balances, quotes, and timestamps.
- Preservation of immutable fields.
- Exact equality between the expected and proposed successor states.

Adding, removing, or reordering fields changes the wire format. When existing
data must remain usable, design explicit versions and migrations, update clients,
and verify independent byte vectors.

## Read the schema

```rust
use ckb_contract_core::wire::CodecSchema;
use framework_counter_script::State;

assert_eq!(State::SCHEMA.prefix, b"CNT01");
```

`CodecSchema` describes data encoding; `Cells::schema()` describes a collection's
declarations. Both can contribute to interface export, covered in
[Testing and IDL](testing-and-idl.md).

Next: [Transaction Construction](transactions.md).
