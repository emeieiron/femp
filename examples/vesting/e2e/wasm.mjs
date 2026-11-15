import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";

const BINDINGS = new URL("../build/wasm/web/vesting_web.js", import.meta.url);
const HOST = new URL("../build/wasm/web/host.js", import.meta.url);

/** Runs cases through the WebAssembly module and its generated bindings. */
export async function start() {
  if (!existsSync(fileURLToPath(BINDINGS))) {
    return { skipped: "not built" };
  }
  const { VestingWeb } = await import(BINDINGS.href);
  const { GuestAbort } = await import(HOST.href);
  const vesting = await VestingWeb.load();

  function run(vector) {
    try {
      const value =
        vector.op === "vested"
          ? vesting.vestedAt(...vector.schedule, vector.now)
          : vesting.claim(...vector.schedule, 0n, vector.amount, vector.now);
      return { value };
    } catch (failure) {
      if (failure instanceof GuestAbort) {
        return { error: failure.code };
      }
      throw failure;
    }
  }

  return { run, stop() {} };
}
