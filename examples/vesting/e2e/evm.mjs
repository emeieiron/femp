import { spawn, spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const BYTECODE = fileURLToPath(new URL("../build/evm/contracts/Vesting.bin", import.meta.url));
const CONSTRUCTOR = "constructor(uint64,uint64,uint64,uint64)";
const DEPLOY_TIMESTAMP = 2;
const TIMEOUT_MS = 10000;
const POLL_INTERVAL_MS = 50;

/**
 * Runs cases against the deployed contract on a local development chain.
 * Needs Foundry's `anvil` and `cast`; without them the platform is skipped.
 */
export async function start() {
  if (!existsSync(BYTECODE)) {
    return { skipped: "not built" };
  }
  if (!hasTool("anvil") || !hasTool("cast")) {
    return { skipped: "anvil and cast (Foundry) not found" };
  }

  const port = 20000 + (process.pid % 20000);
  // The chain starts at timestamp 1 so that cases can place blocks at the
  // small timestamps the table uses.
  const node = spawn("anvil", ["--port", String(port), "--timestamp", "1", "--silent"], {
    stdio: "ignore",
  });
  const rpc = client(`http://127.0.0.1:${port}`);
  await waitUntilReady(rpc);

  const [sender] = await rpc("eth_accounts");
  const bytecode = `0x${readFileSync(BYTECODE, "utf8").trim()}`;
  const failureSelector = cast("sig", "VestingFailed(uint32)");

  // A revert carries `VestingFailed(code)`: its selector, then the code.
  function decode(failure) {
    const data = failure.data ?? "";
    if (!data.startsWith(failureSelector)) {
      throw failure;
    }
    return { error: Number(BigInt(`0x${data.slice(failureSelector.length)}`)) };
  }

  async function call(request, block) {
    try {
      return { value: BigInt(await rpc("eth_call", request, block)) };
    } catch (failure) {
      return decode(failure);
    }
  }

  async function run(vector) {
    const snapshot = await rpc("evm_snapshot");
    try {
      const args = cast("abi-encode", CONSTRUCTOR, ...vector.schedule.map(String)).slice(2);
      const creation = { from: sender, data: bytecode + args };
      const simulated = await call(creation, "latest");
      if ("error" in simulated) {
        return simulated;
      }
      await rpc("evm_setNextBlockTimestamp", DEPLOY_TIMESTAMP);
      const receipt = await mined(rpc, await rpc("eth_sendTransaction", creation));
      const contract = receipt.contractAddress;

      if (vector.op === "vested") {
        const data = cast("calldata", "vestedAt(uint64)", String(vector.now));
        return await call({ to: contract, data }, "latest");
      }
      // A claim reads the block's time, so it is simulated in a pending
      // block placed at the case's timestamp.
      await rpc("evm_setNextBlockTimestamp", Number(vector.now));
      const data = cast("calldata", "claim(uint64)", String(vector.amount));
      return await call({ from: sender, to: contract, data }, "pending");
    } finally {
      await rpc("evm_revert", snapshot);
    }
  }

  return { run, stop: () => node.kill() };
}

function hasTool(name) {
  return spawnSync(name, ["--version"], { stdio: "ignore" }).status === 0;
}

function cast(...args) {
  const outcome = spawnSync("cast", args, { encoding: "utf8" });
  if (outcome.status !== 0) {
    throw new Error(`cast ${args.join(" ")} failed: ${outcome.stderr}`);
  }
  return outcome.stdout.trim();
}

function client(url) {
  let id = 0;
  return async (method, ...params) => {
    const response = await fetch(url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: ++id, method, params }),
    });
    const { result, error } = await response.json();
    if (error) {
      throw Object.assign(new Error(error.message), { data: error.data });
    }
    return result;
  };
}

async function waitUntilReady(rpc) {
  return retry(() => rpc("eth_chainId"), "the local chain did not start");
}

// A transaction's receipt only exists once its block has been mined.
async function mined(rpc, transaction) {
  return retry(async () => {
    const receipt = await rpc("eth_getTransactionReceipt", transaction);
    if (!receipt) {
      throw new Error("no receipt yet");
    }
    return receipt;
  }, "the transaction was not mined");
}

async function retry(attempt, complaint) {
  const deadline = Date.now() + TIMEOUT_MS;
  for (;;) {
    try {
      return await attempt();
    } catch (failure) {
      if (Date.now() > deadline) {
        throw new Error(`${complaint}: ${failure.message}`);
      }
      await new Promise((resolve) => setTimeout(resolve, POLL_INTERVAL_MS));
    }
  }
}
