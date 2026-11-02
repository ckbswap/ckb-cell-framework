import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../../../", import.meta.url));
for (const [command, args] of [
  ["bash", ["scripts/build.sh", "framework-counter-script"]],
  [
    "cargo",
    [
      "test",
      "--locked",
      "-p",
      "framework-vm-tests",
      "--features",
      "vm-tests",
      "counter_web::ccc_web_transactions_execute_in_ckb_vm",
      "--",
      "--ignored",
      "--exact",
    ],
  ],
] as const) {
  const result = spawnSync(command, [...args], { cwd: root, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status || 1);
}
