# Getting Started

This chapter walks through building a CKB-VM contract, running local tests, and
generating your own contract in this workspace. Run the commands from the
repository root.

## 1. Set up the environment

You need Rust/Cargo, Python 3, Bash, and Clang with `riscv64` support. Install the
`riscv64imac-unknown-none-elf` Rust target:

```sh
rustup target add riscv64imac-unknown-none-elf
rustc --version
cargo --version
python3 --version
```

The build script searches for a Clang installation with RISC-V support. If your
default Clang does not support that target, point `CLANG` at a suitable LLVM
installation, for example:

```sh
export CLANG=/opt/homebrew/opt/llvm/bin/clang
"$CLANG" --print-targets
```

Adjust the path to your installation. The target list should include `riscv64`.
Dependency versions are recorded in the root `Cargo.lock`; the first build needs
to download the corresponding Cargo dependencies.

## 2. Explore the workspace commands

```sh
cargo cell help
```

`cargo cell` is a Cargo alias defined in this repository's `.cargo/config.toml`.
It runs the workspace's `cargo-cell` crate. Run it from the framework workspace
or one of its subdirectories.

| Command | Purpose |
|---|---|
| `cargo cell new <name>` | Generate a contract and VM test package from the current Counter template |
| `cargo cell build [package]` | Build a contract; defaults to Counter |
| `cargo cell test` | Check formatting, run host and tool tests, build contracts, and run VM tests |
| `cargo cell idl <package>` | Run the package's registered IDL exporter |

These commands do not sign or broadcast transactions.

## 3. Run Counter first

```sh
cargo test -p framework-counter-script
cargo cell build framework-counter-script
```

The first command runs pure logic and codec tests. The second builds a release
RISC-V binary and copies it to:

```text
build/framework-counter-script
```

See [Program Structure](program-structure.md) for the Counter implementation. Its
State Cell uses the owner's wallet Lock and the Counter Type Script, and stores
owner, nonce, and value. Creation initializes value and nonce to zero; each update
checks a positive increment and the exact expected successor state.

## 4. Run the complete local checks

```sh
cargo cell test
```

The test command discovers registered contracts, rebuilds their binaries, and
runs the corresponding VM test packages. It also verifies or caches the official
xUDT binary at `vendor/xudt`; downloading a missing file requires network access.
Once dependencies are cached, you can run:

```sh
CARGO_NET_OFFLINE=true cargo cell test
```

`CARGO_NET_OFFLINE` controls Cargo dependency resolution. Other scripts can still
download xUDT if its cache is missing. To prepare xUDT separately, run:

```sh
python3 scripts/fetch-udt.py
```

Local VM tests execute through `ckb-testtool`, without a running node or network
deployment.

## 5. Generate your contract

This example uses the project name `my-counter`:

```sh
cargo cell new my-counter
cargo check -p my-counter-script
cargo cell build my-counter-script
cargo test -p my-counter-script
cargo test -p my-counter-vm-tests --features vm-tests
```

`new` registers the workspace members, dependency, and VM test package. The
following `cargo check` validates the project and updates `Cargo.lock`, allowing
the build script to use `--locked` afterward. Names must use lowercase kebab-case,
be at most 40 characters long, and not conflict with an existing path or package.

The generated directory is:

```text
contracts/my-counter/
├── contract/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── cells.rs
│       ├── state.rs
│       └── main.rs
└── tests/
    ├── Cargo.toml
    └── src/
        ├── lib.rs
        └── tests.rs
```

The contract package is `my-counter-script`; its VM test package is
`my-counter-vm-tests`. The initial logic, encoding, and tests come from the current
Counter example. Read the data and `apply` in `state.rs`, the creation/update
collections in `cells.rs`, then the handlers in `lib.rs`.

When changing State, Action, or protocol constraints, update the corresponding
client construction and tests. Changing a Rust structure does not migrate the
bytes already stored on-chain.

## 6. Validate your changes

```sh
cargo fmt --all
cargo test -p my-counter-script
cargo cell build my-counter-script
cargo test -p my-counter-vm-tests --features vm-tests
```

VM tests load the binary from `build/<package>`. Rebuild after changing a contract
before running its VM tests, so the test uses the current binary. Finish with
`cargo cell test`; the new package is included automatically.

Next: [Program Structure](program-structure.md).
