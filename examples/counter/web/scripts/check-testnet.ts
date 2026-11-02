import { readFile } from "node:fs/promises";
import { parseArgs } from "node:util";
import { fileURLToPath } from "node:url";
import { ccc } from "@ckb-ccc/shell";
import {
  TESTNET_RPC,
  assertTestnet,
  parseDeployment,
  verifyDeployment,
} from "../src/lib/counter.ts";

const { values } = parseArgs({
  options: {
    rpc: { type: "string", default: process.env.CKB_RPC_URL || TESTNET_RPC },
    deployment: {
      type: "string",
      default: fileURLToPath(
        new URL("../public/deployment.json", import.meta.url),
      ),
    },
  },
});
const owner = ccc.ClientPublicTestnet.open({
  urls: [values.rpc!],
  timeout: 15_000,
});
try {
  const client = owner.value;
  await assertTestnet(client);
  console.log(
    `CKB Pudge testnet connected. Tip: ${(await client.getTipHeader()).number}`,
  );
  const input: unknown = JSON.parse(await readFile(values.deployment!, "utf8"));
  if (input !== null) {
    const deployment = parseDeployment(input);
    await verifyDeployment(client, deployment);
    console.log(
      `Live code cell verified: ${deployment.outPoint.txHash}:${deployment.outPoint.index}`,
    );
  } else {
    console.log(
      "No Counter deployment configured yet. Deploy the binary, then run this check again.",
    );
  }
} catch (error) {
  console.error(
    error instanceof Error ? error.message : "Testnet check failed.",
  );
  process.exitCode = 1;
} finally {
  await owner.dispose();
}
