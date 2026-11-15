import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";

const EXECUTABLE = fileURLToPath(new URL("../build/native/cli/vesting_cli", import.meta.url));

/** Runs cases through the command-line program, one process per case. */
export async function start() {
  if (!existsSync(EXECUTABLE)) {
    return { skipped: "not built" };
  }
  return { run, stop() {} };
}

function run(vector) {
  const schedule = vector.schedule.map(String);
  const args =
    vector.op === "vested"
      ? ["vested", ...schedule, String(vector.now)]
      : ["claim", ...schedule, "0", String(vector.amount), String(vector.now)];
  const outcome = spawnSync(EXECUTABLE, args, { encoding: "utf8" });
  return outcome.status === 0 ? { value: BigInt(outcome.stdout.trim()) } : { error: outcome.status };
}
