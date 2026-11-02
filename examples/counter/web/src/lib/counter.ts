import { ccc } from "@ckb-ccc/core";

export const TESTNET_RPC = "https://testnet.ckb.dev/";
export const TESTNET_GENESIS =
  "0x10639e0895502b5688a6be8cf69460d76541bfa4821629d86d62ba0aae3f9606";
export const EXPLORER = "https://pudge.explorer.nervos.org";
export const STATE_CAPACITY = 300n * 100_000_000n;
export const U64_MAX = (1n << 64n) - 1n;
export const FEE_RATE = 1_000n;
export const MAX_FEE = 100_000_000n;
const PREFIX = new TextEncoder().encode("CNT01");

export interface Deployment {
  version: 2;
  network: "testnet";
  codeHash: ccc.Hex;
  hashType: "data2";
  outPoint: { txHash: ccc.Hex; index: string };
}

export interface CounterState {
  owner: ccc.Hex;
  nonce: bigint;
  value: bigint;
}

export interface Counter {
  id: ccc.Hex;
  cell: ccc.Cell;
  state: CounterState;
}

export interface PreparedCounter {
  tx: ccc.Transaction;
  id: ccc.Hex;
  state: CounterState;
  fee: bigint;
}

export function hex32(value: unknown, label = "Hash"): ccc.Hex {
  if (typeof value !== "string" || !/^0x[0-9a-fA-F]{64}$/.test(value)) {
    throw new Error(`${label} must be a 32-byte hexadecimal value.`);
  }
  return value.toLowerCase() as ccc.Hex;
}

export function parseDeployment(value: unknown): Deployment {
  const d = value as Partial<Deployment> | null;
  if (
    !d ||
    d.version !== 2 ||
    d.network !== "testnet" ||
    d.hashType !== "data2"
  ) {
    throw new Error(
      "Import a version 2 Counter testnet deployment generated from the current contract.",
    );
  }
  const index = d.outPoint?.index;
  if (typeof index !== "string" || !/^0x[0-9a-fA-F]{1,8}$/.test(index)) {
    throw new Error("Deployment output index must be a u32 hexadecimal value.");
  }
  return {
    version: 2,
    network: "testnet",
    hashType: "data2",
    codeHash: hex32(d.codeHash, "Code hash"),
    outPoint: {
      txHash: hex32(d.outPoint?.txHash, "Deployment transaction"),
      index,
    },
  };
}

function u64(value: bigint): bigint {
  if (value < 0n || value > U64_MAX)
    throw new Error("Value exceeds the u64 range.");
  return value;
}

export function parseIncrement(value: string): bigint {
  if (!/^[1-9][0-9]*$/.test(value))
    throw new Error("Enter a positive whole number.");
  return u64(BigInt(value));
}

export function encodeState(state: CounterState): ccc.Hex {
  const bytes = new Uint8Array(53);
  bytes.set(PREFIX);
  bytes.set(ccc.bytesFrom(hex32(state.owner, "Owner lock hash")), 5);
  const view = new DataView(bytes.buffer);
  view.setBigUint64(37, u64(state.nonce), true);
  view.setBigUint64(45, u64(state.value), true);
  return ccc.hexFrom(bytes);
}

export function decodeState(data: ccc.BytesLike): CounterState {
  const bytes = ccc.bytesFrom(data);
  if (bytes.length !== 53 || !PREFIX.every((byte, i) => byte === bytes[i])) {
    throw new Error("Invalid Counter state encoding.");
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return {
    owner: ccc.hexFrom(bytes.slice(5, 37)),
    nonce: view.getBigUint64(37, true),
    value: view.getBigUint64(45, true),
  };
}

export function encodeAction(increment: bigint): ccc.Hex {
  if (increment <= 0n) throw new Error("Increment must be positive.");
  const bytes = new Uint8Array(8);
  new DataView(bytes.buffer).setBigUint64(0, u64(increment), true);
  return ccc.hexFrom(bytes);
}

export function nextState(
  before: CounterState,
  increment: bigint,
): CounterState {
  encodeAction(increment);
  return {
    owner: before.owner,
    nonce: u64(before.nonce + 1n),
    value: u64(before.value + increment),
  };
}

export function counterScript(
  deployment: Deployment,
  id: ccc.Hex,
): ccc.Script {
  return ccc.Script.from({
    codeHash: deployment.codeHash,
    hashType: deployment.hashType,
    args: hex32(id, "Counter ID"),
  });
}

export async function assertTestnet(client: ccc.Client): Promise<void> {
  if (
    client.addressPrefix !== "ckt" ||
    (await client.getHeaderByNumber(0))?.hash !== TESTNET_GENESIS
  ) {
    throw new Error("This example only supports the CKB Pudge testnet.");
  }
}

export async function verifyDeployment(
  client: ccc.Client,
  deployment: Deployment,
): Promise<void> {
  await assertTestnet(client);
  const code = await client.getCellLive(deployment.outPoint, true);
  if (!code)
    throw new Error("The deployed code cell is missing or has been spent.");
  if (ccc.hashCkb(code.outputData) !== deployment.codeHash) {
    throw new Error("The code cell does not match the configured code hash.");
  }
}

export function readCounter(deployment: Deployment, cell: ccc.Cell): Counter {
  const type = cell.cellOutput.type;
  if (
    !type ||
    type.codeHash !== deployment.codeHash ||
    type.hashType !== deployment.hashType ||
    !/^0x[0-9a-f]{64}$/.test(type.args)
  ) {
    throw new Error("Cell does not match the Counter deployment and layout.");
  }
  const id = hex32(type.args, "Counter ID");
  const state = decodeState(cell.outputData);
  if (
    cell.cellOutput.lock.hash() !== state.owner ||
    cell.cellOutput.capacity !== STATE_CAPACITY
  ) {
    throw new Error(
      "Counter lock or capacity does not match the contract layout.",
    );
  }
  return { id, cell, state };
}

export async function findCounters(
  client: ccc.Client,
  deployment: Deployment,
  owner: ccc.Hex,
): Promise<Counter[]> {
  const counters: Counter[] = [];
  for await (const cell of client.findCells(
    {
      script: {
        codeHash: deployment.codeHash,
        hashType: deployment.hashType,
        args: "0x",
      },
      scriptType: "type",
      scriptSearchMode: "prefix",
      withData: true,
      filter: {
        outputData: `0x434e543031${hex32(owner).slice(2)}`,
        outputDataSearchMode: "prefix",
      },
    },
    "desc",
    50,
  )) {
    const counter = readCounter(deployment, cell);
    if (counter.state.owner === owner) counters.push(counter);
    if (counters.length >= 100) break;
  }
  return counters;
}

async function authorizationCell(
  signer: ccc.Signer,
  owner: ccc.Script,
): Promise<ccc.Cell> {
  // A related wallet address (e.g. ACP) has a different Lock hash. Seed an
  // explicit input under the exact owner Lock before automatic fee completion.
  for await (const cell of signer.client.findCells({
    script: owner,
    scriptType: "lock",
    scriptSearchMode: "exact",
    withData: true,
    filter: { scriptLenRange: [0, 1], outputDataLenRange: [0, 1] },
  })) {
    if (
      cell.cellOutput.lock.eq(owner) &&
      !cell.cellOutput.type &&
      cell.outputData === "0x"
    )
      return cell;
  }
  throw new Error(
    "No plain CKB cell is available at your wallet address. Fund this testnet address, then refresh.",
  );
}

/** Pure transaction assembly, shared by the UI and the VM fixture exporter. */
export function createCounterTransaction(
  deployment: Deployment,
  funding: ccc.Cell,
): Omit<PreparedCounter, "fee"> {
  const first = ccc.CellInput.from(funding);
  const id = ccc.hashTypeId(first, 0);
  const state: CounterState = {
    owner: funding.cellOutput.lock.hash(),
    nonce: 0n,
    value: 0n,
  };
  const tx = ccc.Transaction.from({
    inputs: [first],
    cellDeps: [{ outPoint: deployment.outPoint, depType: "code" }],
    outputs: [
      {
        capacity: STATE_CAPACITY,
        lock: funding.cellOutput.lock,
        type: counterScript(deployment, id),
      },
    ],
    outputsData: [encodeState(state)],
  });
  return { tx, id, state };
}

export function updateCounterTransaction(
  deployment: Deployment,
  counter: Counter,
  funding: ccc.Cell,
  increment: bigint,
): Omit<PreparedCounter, "fee"> {
  if (counter.state.owner !== funding.cellOutput.lock.hash())
    throw new Error("Only the Counter owner can update it.");
  const state = nextState(counter.state, increment);
  const tx = ccc.Transaction.from({
    inputs: [ccc.CellInput.from(counter.cell), ccc.CellInput.from(funding)],
    cellDeps: [{ outPoint: deployment.outPoint, depType: "code" }],
    outputs: [
      {
        capacity: STATE_CAPACITY,
        lock: counter.cell.cellOutput.lock,
        type: counterScript(deployment, counter.id),
      },
    ],
    outputsData: [encodeState(state)],
  });
  // The action belongs to the State input, not the wallet's signature witness.
  tx.setWitnessArgs(0, { inputType: encodeAction(increment) });
  return { tx, id: counter.id, state };
}

async function complete(
  signer: ccc.Signer,
  prepared: Omit<PreparedCounter, "fee">,
): Promise<PreparedCounter> {
  const first = ccc.hexFrom(prepared.tx.inputs[0].toBytes());
  await prepared.tx.completeInputsByCapacity(signer);
  await prepared.tx.completeFeeBy(signer, FEE_RATE);
  if (
    ccc.hexFrom(prepared.tx.inputs[0].toBytes()) !== first ||
    prepared.tx.outputs[0].capacity !== STATE_CAPACITY ||
    prepared.tx.outputsData[0] !== encodeState(prepared.state)
  ) {
    throw new Error("Transaction completion changed the Counter layout.");
  }
  const fee = await prepared.tx.getFee(signer.client);
  if (fee < 0n || fee > MAX_FEE)
    throw new Error("Transaction fee exceeds the 1 CKB limit.");
  return { ...prepared, fee };
}

export async function prepareCreate(
  signer: ccc.Signer,
  deployment: Deployment,
): Promise<PreparedCounter> {
  await verifyDeployment(signer.client, deployment);
  const owner = (await signer.getRecommendedAddressObj()).script;
  const prepared = createCounterTransaction(
    deployment,
    await authorizationCell(signer, owner),
  );
  return complete(signer, prepared);
}

export async function prepareUpdate(
  signer: ccc.Signer,
  deployment: Deployment,
  selected: Counter,
  increment: bigint,
): Promise<PreparedCounter> {
  await verifyDeployment(signer.client, deployment);
  const latest = await signer.client.getCellLive(selected.cell.outPoint, true);
  if (!latest)
    throw new Error(
      "This Counter cell has been spent. Refresh to load its latest state.",
    );
  const owner = (await signer.getRecommendedAddressObj()).script;
  const prepared = updateCounterTransaction(
    deployment,
    readCounter(deployment, latest),
    await authorizationCell(signer, owner),
    increment,
  );
  return complete(signer, prepared);
}

export async function sendPrepared(
  signer: ccc.Signer,
  prepared: PreparedCounter,
  onSigned?: (hash: ccc.Hex) => void,
): Promise<ccc.Hex> {
  await assertTestnet(signer.client);
  const expectedHash = prepared.tx.hash();
  const signed = await signer.signTransaction(prepared.tx.clone());
  if (signed.hash() !== expectedHash)
    throw new Error("Wallet changed the transaction. Refresh and try again.");
  onSigned?.(expectedHash);
  const returned = await signer.client.sendTransaction(signed, "passthrough");
  if (returned !== expectedHash)
    throw new Error("RPC returned an unexpected transaction hash.");
  return returned;
}

export async function waitCommitted(
  client: ccc.Client,
  txHash: ccc.Hex,
): Promise<void> {
  // Explicit status handling also catches rejected transactions before timeout.
  for (let attempt = 0; attempt < 90; attempt++) {
    const tx = await client.getTransaction(txHash);
    if (tx?.status === "committed") return;
    if (tx?.status === "rejected")
      throw new Error(tx.reason || "The testnet rejected this transaction.");
    await new Promise((resolve) => setTimeout(resolve, 2_000));
  }
  throw new Error(
    "Confirmation is taking longer than expected. Check the transaction in the explorer before retrying.",
  );
}

export function displayError(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "The request failed. Please try again.";
}
