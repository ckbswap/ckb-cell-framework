# Counter Web

A React + TypeScript client for the framework's Counter contract, using CCC for wallet connections, Cell queries, transaction assembly, signing, and broadcasting on CKB Pudge testnet.

## Install and run

Requires Node.js 22.12+ and npm, in addition to the framework's Rust build prerequisites.

```bash
# From the repository root:
npm --prefix examples/counter/web ci
bash scripts/deploy-counter.sh
```

The command builds the Counter binary and inspects it locally. It does not sign or send a transaction. To deploy it, use a dedicated, funded testnet account:

```bash
# Bash or Zsh: enter the testnet key without echoing it.
read -r -s CKB_PRIVATE_KEY
export CKB_PRIVATE_KEY
bash scripts/deploy-counter.sh --send
unset CKB_PRIVATE_KEY
```

The deployment script uses `@ckb-ccc/shell`. It verifies the testnet genesis hash, creates a code Cell containing the RISC-V binary, completes inputs and fees, signs, estimates cycles, broadcasts, waits for commitment, and writes `public/deployment.json`. The code Cell uses the deployer's Lock. Keep it live while any Counter instances depend on it.

Code deployment occupies roughly **one CKB per binary byte**, plus the Cell's other fields. A standard secp256k1 Lock adds 61 CKB. The script prints the capacity required by the selected binary and the actual deployer Lock. Each Counter instance separately occupies **300 CKB** and this example does not support closing it. Test funds are available from the [CKB faucet](https://faucet.nervos.org/).

The private key is read only from the Node process environment. Never put it in `VITE_*` variables, deployment JSON, or frontend code.

```bash
cd examples/counter/web
npm run check:testnet
npm run dev
```

Open the URL printed by Vite. The app automatically loads `public/deployment.json`. You can also import a deployment JSON file through the page. Connect a testnet wallet, create a Counter, select an instance, and increment it. The app shows its value, nonce, current OutPoint, and explorer links, and refreshes state after confirmation. Only the exact owner Lock can authorize an update.

## Deployment options

From `examples/counter/web`:

```bash
npm run deploy:testnet -- --help
npm run deploy:testnet -- --send --rpc https://testnet.ckb.dev/
npm run deploy:testnet -- --send --out /path/to/deployment.json
npm run deploy:testnet -- --resume
```

`--binary PATH` selects a compiled binary. `--force` permits replacing a nonempty deployment configuration. Broadcasting requires `--send`; the default command only inspects the binary offline.

The script saves the deterministic transaction hash in `<output>.pending` before broadcasting. If the RPC response or confirmation times out, use `--resume` with the same output path to check that transaction and finish writing the configuration without deploying again. If the transaction was never accepted or was rejected, verify its status before removing the pending file and starting another deployment.

The frontend also retains its last pending transaction locally and blocks further submissions until its status is checked. Use **Check confirmation** or the explorer link after a timeout.

The optional `.env.example` documents `VITE_CKB_RPC_URL` for a custom public testnet RPC. `CKB_RPC_URL` selects the Node scripts' RPC. The app and scripts reject other networks by checking genesis, even if the endpoint is configured as a testnet client.

Deployment configuration format:

```json
{
  "version": 2,
  "network": "testnet",
  "codeHash": "0x<32-byte binary data hash>",
  "hashType": "data2",
  "outPoint": {
    "txHash": "0x<deployment transaction hash>",
    "index": "0x0"
  }
}
```

The app verifies that the code Cell is live and its data hash matches this configuration. Version 2 identifies the current Counter protocol: raw 32-byte instance IDs in the Type args, with each Counter Cell protected by its owner's wallet Lock. The client creates Counters at output zero; the contract also permits updates at other output positions.

Build and deploy the current binary to generate a version 2 configuration. `public/deployment.json` is `null` until a current deployment is configured; deploy the contract or import its configuration before creating Counters.

## Validation

```bash
npm test
npm run build
npm run test:vm
```

- Node tests check Rust-compatible wire encoding, full u64 precision, malformed data, instance IDs, owner authorization, witness placement, and deployment/network validation.
- `test:vm` builds the Rust Counter and runs the optional `counter_web` test using ckb-testtool. It invokes the same TypeScript builders as the UI, executes their serialized creation and update transactions in CKB-VM, commits the predecessor, and checks that output tampering is rejected.
- VM fixtures use `ALWAYS_SUCCESS` wallets and require no network or private keys. Browser wallet signing and funded testnet deployment require a real testnet account.

The standard Rust VM suite leaves this Node-dependent test ignored. Run `npm run test:vm` explicitly after `npm ci`.

## Sources

- [CCC React wallet connector](https://docs.ckbccc.com/en/docs/packages/core-packages/connector-react)
- [CCC source and API](https://github.com/ckb-devrel/ccc)
- [Counter contract](../contract/src/lib.rs)
