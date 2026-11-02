# CKB Cell Framework

A framework for writing CKB contracts in Rust, with typed Cells, declarative
loading constraints, a shared `Context<C>` container, automatic data encoding,
off-chain transaction builders, and local VM testing tools.

Contracts declare the input, output, and dependency Cells they need. Handlers
validate state transitions, authorization, and asset settlement. Clients build
transactions in advance; on-chain scripts verify that those transactions follow
the protocol.

## Get started

Run these commands in this repository's workspace:

```sh
rustup target add riscv64imac-unknown-none-elf
cargo cell help
cargo test -p framework-counter-script
cargo cell build framework-counter-script
cargo cell test
```

Building requires Clang with RISC-V support. See [Getting Started](docs/getting-started.md)
for environment setup, your first test run, and project generation.

## Documentation

Start at the [documentation home](docs/README.md), or read the chapters in order:

1. [Getting Started](docs/getting-started.md): set up, build, test, and generate a project.
2. [Program Structure](docs/program-structure.md): entry points, handlers, and state transitions.
3. [Cells and Context](docs/cells-and-context.md): field declarations, selection, and constraints.
4. [State and Codecs](docs/state-and-codecs.md): `CellCodec`, Actions, and wire formats.
5. [Transaction Construction](docs/transactions.md): outputs, witnesses, and multiple reserves.
6. [Testing and IDL](docs/testing-and-idl.md): pure tests, VM tests, and interface export.
7. [API and Command Reference](docs/reference.md): commands, helpers, and error codes.

## Examples

| Example | Topics |
|---|---|
| [Counter](examples/counter/contract/src/lib.rs) | Creation and updates, wallet Locks, operation-specific Cell collections; also the project template |
| [Vault](examples/vault/README.md) | Owner deposits and withdrawals, reserve validation, closing, and CKB refunds |
| [Escrow](examples/escrow/README.md) | Two-party authorization, release and refund, one-time settlement |
| [Vesting](examples/vesting/contract/src/lib.rs) | Scheduled claims, time constraints, and the State input's `since` |
| [Limit Order](examples/limit-order/README.md) | Fixed prices, partial fills, exact net payments, and rounding |
| [AMM](examples/amm/README.md) | Two reserves, constant-product swaps, and official xUDT LP tokens |

## Workspace

| Crate | Responsibility |
|---|---|
| `ckb-contract-core` | `no_std` data, codecs, math, state transitions, and layout descriptions |
| `ckb-contract-derive` | The `CellCodec` and `Cells` derive macros |
| `ckb-contract-onchain` | `no_std` Cell loading, syscall adapters, authorization, and asset checks |
| `ckb-contract-tx` | Off-chain unsigned transaction construction |
| `ckb-contract-testkit` | Local CKB-VM testing tools and fixtures |
| `ckb-contract-idl` | Host-side JSON interface export |
| `cargo-cell` | Workspace commands for project creation, builds, tests, and IDL export |
