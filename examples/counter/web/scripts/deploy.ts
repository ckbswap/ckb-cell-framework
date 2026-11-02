import { readFile, writeFile, mkdir, access, unlink } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { ccc } from "@ckb-ccc/shell";
import {
  assertTestnet,
  TESTNET_RPC,
  EXPLORER,
  FEE_RATE,
  MAX_FEE,
  waitCommitted,
  parseDeployment,
  verifyDeployment,
  type Deployment,
} from "../src/lib/counter.ts";

const { values } = parseArgs({
  options: {
    binary: {
      type: "string",
      default: fileURLToPath(
        new URL("../../../../build/framework-counter-script", import.meta.url),
      ),
    },
    out: {
      type: "string",
      default: fileURLToPath(
        new URL("../public/deployment.json", import.meta.url),
      ),
    },
    rpc: { type: "string", default: process.env.CKB_RPC_URL || TESTNET_RPC },
    send: { type: "boolean", default: false },
    resume: { type: "boolean", default: false },
    force: { type: "boolean", default: false },
    help: { type: "boolean", short: "h" },
  },
});

async function main() {
  if (values.help) {
    console.log(`Deploy the Counter binary to CKB TESTNET using CCC.

  npm run deploy:testnet                     Inspect the binary (offline)
  npm run deploy:testnet -- --send            Sign, broadcast, and write deployment.json
  npm run deploy:testnet -- --resume          Confirm a previously broadcast deployment

Options: --binary PATH --out PATH --rpc URL --force
Signing: set CKB_PRIVATE_KEY in the environment of this Node process only.
Never put a private key in a VITE_* variable or a frontend configuration file.
--force permits replacing an existing deployment file after confirmation.`);
    return;
  }
  const binary = await readFile(values.binary!);
  if (
    binary.length < 20 ||
    binary.subarray(0, 4).toString("hex") !== "7f454c46" ||
    binary.readUInt16LE(18) !== 243
  ) {
    throw new Error(
      "Expected a RISC-V ELF binary. Run cargo cell build framework-counter-script first.",
    );
  }
  const codeHash = ccc.hashCkb(binary);
  console.log(
    `Network: CKB Pudge testnet\nBinary: ${resolve(values.binary!)}\nBytes: ${binary.length}\nCode hash: ${codeHash}\nHash type: data2`,
  );
  console.log(
    `Minimum code-cell capacity with a standard CKB lock: ${binary.length + 61} CKB`,
  );
  if (values.send && values.resume)
    throw new Error("Choose --send or --resume.");
  if (!values.send && !values.resume) {
    console.log(
      "Offline inspection complete. Use --send with a funded testnet signing account to deploy.",
    );
    return;
  }
  const privateKey = process.env.CKB_PRIVATE_KEY ?? "";
  if (!values.resume && !/^(0x)?[0-9a-fA-F]{64}$/.test(privateKey)) {
    throw new Error(
      "Set CKB_PRIVATE_KEY to a funded testnet private key before using --send.",
    );
  }
  const out = resolve(values.out!);
  const pendingPath = `${out}.pending`;
  await mkdir(dirname(out), { recursive: true });
  // The tracked placeholder is null. Refuse to overwrite a real deployment
  // unless requested, and check write access before spending any capacity.
  try {
    const previous = await readFile(out, "utf8");
    if (previous.trim() !== "null" && !values.force)
      throw new Error(
        "Deployment file already exists. Use --out PATH or --force.",
      );
    await access(out, 2);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    await access(dirname(out), 2);
  }
  const owner = ccc.ClientPublicTestnet.open({
    urls: [values.rpc!],
    timeout: 15_000,
  });
  try {
    const client = owner.value;
    await assertTestnet(client);
    if (values.resume) {
      const deployment = parseDeployment(
        JSON.parse(await readFile(pendingPath, "utf8")),
      );
      if (deployment.codeHash !== codeHash)
        throw new Error(
          "Pending deployment belongs to a different binary. Select it with --binary.",
        );
      console.log(
        `Resuming: ${EXPLORER}/transaction/${deployment.outPoint.txHash}`,
      );
      await waitCommitted(client, deployment.outPoint.txHash);
      await verifyDeployment(client, deployment);
      await writeFile(out, `${JSON.stringify(deployment, null, 2)}\n`);
      await unlink(pendingPath);
      console.log(`Confirmed. Frontend deployment saved to ${out}`);
      return;
    }
    try {
      await access(pendingPath);
      throw new Error(
        "A pending deployment already exists. Use --resume before sending another transaction.",
      );
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    const signer = new ccc.SignerCkbPrivateKey(
      client,
      privateKey.startsWith("0x") ? privateKey : `0x${privateKey}`,
    );
    const address = await signer.getRecommendedAddressObj();
    const tx = ccc.Transaction.from({
      outputs: [{ lock: address.script }],
      outputsData: [ccc.hexFrom(binary)],
    });
    console.log(
      `Deployer: ${address.toString()}\nCode-cell capacity: ${ccc.fixedPointToString(tx.outputs[0].capacity)} CKB`,
    );
    await tx.completeInputsByCapacity(signer);
    await tx.completeFeeBy(signer, FEE_RATE);
    const fee = await tx.getFee(client);
    if (fee < 0n || fee > MAX_FEE)
      throw new Error("Deployment fee exceeds the 1 CKB limit.");
    const rawHash = tx.hash();
    const signed = await signer.signTransaction(tx.clone());
    if (signed.hash() !== rawHash)
      throw new Error("Signing changed the deployment transaction.");
    // Check the fully signed transaction before broadcasting it.
    const cycles = await client.estimateCycles(signed);
    console.log(
      `Fee: ${ccc.fixedPointToString(fee)} CKB\nEstimated cycles: ${cycles}`,
    );
    const txHash = signed.hash();
    const deployment: Deployment = {
      version: 2,
      network: "testnet",
      codeHash,
      hashType: "data2",
      outPoint: { txHash, index: "0x0" },
    };
    // Persist the deterministic hash before broadcast: an RPC timeout may
    // occur after acceptance, and must not lead to another deployment.
    await writeFile(pendingPath, `${JSON.stringify(deployment, null, 2)}\n`, {
      flag: "wx",
    });
    console.log(
      `Transaction: ${EXPLORER}/transaction/${txHash}\nPending deployment saved to ${pendingPath}`,
    );
    const returned = await client.sendTransaction(signed, "passthrough");
    if (returned !== txHash)
      throw new Error("RPC returned an unexpected transaction hash.");
    await waitCommitted(client, txHash);
    await verifyDeployment(client, deployment);
    await writeFile(out, `${JSON.stringify(deployment, null, 2)}\n`);
    await unlink(pendingPath);
    console.log(`Confirmed. Frontend deployment saved to ${out}`);
  } finally {
    await owner.dispose();
  }
}

main().catch((error: unknown) => {
  // Avoid printing SDK objects: signers and error payloads may contain secrets.
  console.error(
    error instanceof Error
      ? error.message.replace(/\b(?:0x)?[0-9a-fA-F]{64}\b/g, "[hash redacted]")
      : "Deployment failed.",
  );
  process.exitCode = 1;
});
