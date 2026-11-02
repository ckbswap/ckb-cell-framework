// Invoked only by the ignored Rust VM integration test. No RPC or private keys.
import { readFile } from "node:fs/promises";
import { ccc } from "@ckb-ccc/core";
import {
  createCounterTransaction,
  updateCounterTransaction,
  readCounter,
  type Deployment,
} from "../src/lib/counter.ts";

const owner = ccc.Script.fromBytes(process.env.CCC_VM_OWNER_SCRIPT!);
const codePoint = ccc.OutPoint.fromBytes(process.env.CCC_VM_CODE_OUT_POINT!);
const ownerDep = ccc.OutPoint.fromBytes(process.env.CCC_VM_OWNER_DEP!);
const binary = await readFile(
  new URL("../../../../build/framework-counter-script", import.meta.url),
);
const deployment: Deployment = {
  version: 2,
  network: "testnet",
  hashType: "data2",
  codeHash: ccc.hashCkb(binary),
  outPoint: { txHash: codePoint.txHash, index: ccc.numToHex(codePoint.index) },
};
const funding = (byte: string) =>
  ccc.Cell.from({
    outPoint: { txHash: `0x${byte.repeat(32)}`, index: 0 },
    cellOutput: { capacity: 1000n * 100_000_000n, lock: owner },
    outputData: "0x",
  });
const creation = createCounterTransaction(deployment, funding("11"));
creation.tx.cellDeps.push(
  ccc.CellDep.from({ outPoint: ownerDep, depType: "code" }),
);
creation.tx.addOutput({ capacity: 699n * 100_000_000n, lock: owner }, "0x");
const cell = ccc.Cell.from({
  outPoint: { txHash: creation.tx.hash(), index: 0 },
  cellOutput: creation.tx.outputs[0],
  outputData: creation.tx.outputsData[0],
});
const update = updateCounterTransaction(
  deployment,
  readCounter(deployment, cell),
  funding("22"),
  7n,
);
update.tx.cellDeps.push(
  ccc.CellDep.from({ outPoint: ownerDep, depType: "code" }),
);
update.tx.addOutput({ capacity: 999n * 100_000_000n, lock: owner }, "0x");
console.log(
  JSON.stringify({
    creation: ccc.hexFrom(creation.tx.toBytes()),
    update: ccc.hexFrom(update.tx.toBytes()),
  }),
);
