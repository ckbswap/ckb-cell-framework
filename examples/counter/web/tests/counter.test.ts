import assert from "node:assert/strict";
import { test } from "node:test";
import { ccc } from "@ckb-ccc/core";
import {
  U64_MAX,
  STATE_CAPACITY,
  TESTNET_GENESIS,
  encodeState,
  decodeState,
  encodeAction,
  parseIncrement,
  nextState,
  parseDeployment,
  counterScript,
  findCounters,
  readCounter,
  createCounterTransaction,
  updateCounterTransaction,
  assertTestnet,
  verifyDeployment,
  sendPrepared,
  type Deployment,
} from "../src/lib/counter.ts";

const hash = (byte: string) => `0x${byte.repeat(32)}` as ccc.Hex;
const deployment: Deployment = {
  version: 2,
  network: "testnet",
  hashType: "data2",
  codeHash: hash("aa"),
  outPoint: { txHash: hash("bb"), index: "0x0" },
};
const owner = ccc.Script.from({
  codeHash: hash("cc"),
  hashType: "data2",
  args: "0x01",
});
const funding = ccc.Cell.from({
  outPoint: { txHash: hash("11"), index: 0 },
  cellOutput: { lock: owner, capacity: 1000n * 100_000_000n },
  outputData: "0x",
});

test("Counter encoding matches Rust v1 golden bytes exactly", () => {
  const state = {
    owner: hash("11"),
    nonce: 0x0102030405060708n,
    value: 0x1112131415161718n,
  };
  const bytes = `0x434e543031${"11".repeat(32)}08070605040302011817161514131211`;
  assert.equal(encodeState(state), bytes);
  assert.deepEqual(decodeState(bytes), state);
  assert.equal(encodeAction(0x0102030405060708n), "0x0807060504030201");
});

test("strict decoding rejects wrong prefixes, truncation, and trailing bytes", () => {
  const valid = encodeState({ owner: owner.hash(), nonce: 0n, value: 0n });
  for (const invalid of [
    "0x",
    valid.slice(0, -2),
    `${valid}00`,
    valid.replace("434e543031", "434e543032"),
  ]) {
    assert.throws(() => decodeState(invalid), /encoding/);
  }
});

test("u64 values retain full precision and overflow is rejected before signing", () => {
  const state = { owner: owner.hash(), nonce: U64_MAX, value: U64_MAX };
  assert.deepEqual(decodeState(encodeState(state)), state);
  assert.equal(parseIncrement(U64_MAX.toString()), U64_MAX);
  assert.throws(() => nextState(state, 1n), /u64/);
  assert.throws(() => nextState({ ...state, nonce: 0n }, 1n), /u64/);
  assert.throws(() => encodeState({ ...state, value: -1n }), /u64/);
});

test("invalid increments never reach transaction assembly", () => {
  for (const invalid of [
    "",
    "0",
    "-1",
    "1.5",
    "1e3",
    " 1",
    "01",
    (U64_MAX + 1n).toString(),
  ]) {
    assert.throws(() => parseIncrement(invalid));
  }
  assert.throws(() => encodeAction(0n), /positive/);
});

test("deployment parser rejects mainnet, wrong hash types, and invalid OutPoints", () => {
  assert.deepEqual(parseDeployment(deployment), deployment);
  for (const invalid of [
    null,
    { ...deployment, network: "mainnet" },
    { ...deployment, version: 1 },
    { ...deployment, hashType: "type" },
    { ...deployment, codeHash: "0x12" },
    {
      ...deployment,
      outPoint: { ...deployment.outPoint, index: "0x100000000" },
    },
  ]) {
    assert.throws(() => parseDeployment(invalid));
  }
});

test("creation binds the serialized first input and keeps State at output zero", () => {
  const prepared = createCounterTransaction(deployment, funding);
  // Independent BLAKE2b-256 vector: CellInput(since=0, tx=0x11..11, index=0)
  // followed by output index u64 LE, with personalization ckb-default-hash.
  assert.equal(
    prepared.id,
    "0x95104db1ddfd88d13f9b38346ad1a273905f4d097a0aabefe0b9f6123568dd65",
  );
  assert.equal(prepared.tx.outputs[0].capacity, STATE_CAPACITY);
  assert(
    prepared.tx.outputs[0].type?.eq(counterScript(deployment, prepared.id)),
  );
  assert(
    prepared.tx.outputs[0].lock.eq(owner),
  );
  assert.deepEqual(decodeState(prepared.tx.outputsData[0]), {
    owner: owner.hash(),
    nonce: 0n,
    value: 0n,
  });
  assert(prepared.tx.inputs[0].previousOutput.eq(funding.outPoint));
  assert.equal(prepared.tx.cellDeps[0].depType, "code");
});

function counter() {
  const created = createCounterTransaction(deployment, funding);
  const cell = ccc.Cell.from({
    outPoint: { txHash: created.tx.hash(), index: 0 },
    cellOutput: created.tx.outputs[0],
    outputData: created.tx.outputsData[0],
  });
  return readCounter(deployment, cell);
}

test("updates include the owner input and place Action in the actual State witness", () => {
  const before = counter();
  const update = updateCounterTransaction(deployment, before, funding, 7n);
  assert(update.tx.inputs[0].previousOutput.eq(before.cell.outPoint));
  assert(update.tx.inputs[1].previousOutput.eq(funding.outPoint));
  assert.equal(update.tx.getWitnessArgs(0)?.inputType, "0x0700000000000000");
  assert.equal(update.tx.getWitnessArgs(1)?.inputType, undefined);
  assert.deepEqual(decodeState(update.tx.outputsData[0]), {
    owner: owner.hash(),
    nonce: 1n,
    value: 7n,
  });
  assert.equal(update.tx.outputs[0].capacity, STATE_CAPACITY);
});

test("another wallet cannot assemble an owner-authorized update", () => {
  const stranger = funding.clone();
  stranger.cellOutput.lock.args = "0x02";
  assert.throws(
    () => updateCounterTransaction(deployment, counter(), stranger, 1n),
    /owner/,
  );
});

test("loading a state checks its owner lock, capacity and raw instance ID", () => {
  for (const mutate of [
    (cell: ccc.Cell) => {
      cell.cellOutput.lock.args = "0x02";
    },
    (cell: ccc.Cell) => {
      cell.cellOutput.capacity += 1n;
    },
    (cell: ccc.Cell) => {
      cell.cellOutput.type!.args = `0x00${cell.cellOutput.type!.args.slice(2)}`;
    },
  ]) {
    const cell = counter().cell.clone();
    mutate(cell);
    assert.throws(() => readCounter(deployment, cell));
  }
});

test("updated Counters can be loaded at any output position", () => {
  const cell = counter().cell.clone();
  cell.outPoint.index = 7n;
  assert.equal(readCounter(deployment, cell).cell.outPoint.index, 7n);
});

test("Cell queries match the type code without a role prefix", async () => {
  const cell = counter().cell.clone();
  cell.outPoint.index = 3n;
  let query: unknown;
  const client = {
    async *findCells(search: unknown) { query = search; yield cell; },
  } as unknown as ccc.Client;
  const results = await findCounters(client, deployment, owner.hash());
  assert.equal(results.length, 1);
  assert.equal(results[0].cell.outPoint.index, 3n);
  assert.equal((query as { script: { args: string } }).script.args, "0x");
});

test("network checks verify genesis even when the address prefix says testnet", async () => {
  await assertTestnet({
    addressPrefix: "ckt",
    getHeaderByNumber: async () => ({ hash: TESTNET_GENESIS }),
  } as unknown as ccc.Client);
  await assert.rejects(
    assertTestnet({ addressPrefix: "ckb" } as ccc.Client),
    /testnet/,
  );
  await assert.rejects(
    assertTestnet({
      addressPrefix: "ckt",
      getHeaderByNumber: async () => ({ hash: hash("00") }),
    } as unknown as ccc.Client),
    /testnet/,
  );
});

test("code validation rejects a spent or mismatched deployment cell", async () => {
  const client = {
    addressPrefix: "ckt",
    getHeaderByNumber: async () => ({ hash: TESTNET_GENESIS }),
    getCellLive: async () => undefined as ccc.Cell | undefined,
  };
  await assert.rejects(
    verifyDeployment(client as unknown as ccc.Client, deployment),
    /missing|spent/,
  );
  client.getCellLive = async () => funding;
  await assert.rejects(
    verifyDeployment(client as unknown as ccc.Client, deployment),
    /code hash/,
  );
});

test("signing cannot change the prepared transaction's outputs", async () => {
  let broadcast = false;
  const signer = {
    client: {
      addressPrefix: "ckt",
      getHeaderByNumber: async () => ({ hash: TESTNET_GENESIS }),
      sendTransaction: async () => {
        broadcast = true;
        return hash("00");
      },
    },
    signTransaction: async (tx: ccc.Transaction) => {
      tx.outputs[0].capacity += 1n;
      return tx;
    },
  } as unknown as ccc.Signer;
  let recorded = false;
  await assert.rejects(
    sendPrepared(
      signer,
      { ...createCounterTransaction(deployment, funding), fee: 1n },
      () => {
        recorded = true;
      },
    ),
    /Wallet changed/,
  );
  assert.equal(broadcast, false);
  assert.equal(recorded, false);
});

test("the signed transaction hash is retained before an ambiguous RPC timeout", async () => {
  const expected = createCounterTransaction(deployment, funding);
  let recorded: ccc.Hex | undefined;
  const signer = {
    client: {
      addressPrefix: "ckt",
      getHeaderByNumber: async () => ({ hash: TESTNET_GENESIS }),
      sendTransaction: async () => {
        assert.equal(recorded, expected.tx.hash());
        throw new Error("RPC timed out");
      },
    },
    signTransaction: async (tx: ccc.Transaction) => tx,
  } as unknown as ccc.Signer;
  await assert.rejects(
    sendPrepared(signer, { ...expected, fee: 1n }, (hash) => {
      recorded = hash;
    }),
    /timed out/,
  );
  assert.equal(recorded, expected.tx.hash());
});
