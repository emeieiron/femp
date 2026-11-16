// Runs one WebAssembly test of `femp test` in its own Node.js process.
//
//   node run_test.mjs <module.wasm> <export>
//
// The module is instantiated with femp's host runtime (`host.js`, next to
// this file), the export is called, and the host is checked for files the
// test left open. Exit status 0 means the test passed. On a failure one line
// on standard error says what happened, and the status is 1. Standard output
// carries the values the test logged, one per line.
import { readFileSync } from "node:fs";

import { ContractError, GuestAbort, instantiate } from "./host.js";

function describe(error) {
  if (error instanceof WebAssembly.RuntimeError) {
    return `trap: ${error.message} (Fe code traps on a failed assertion, arithmetic overflow, division by zero and an index outside an array)`;
  }
  if (error instanceof GuestAbort) {
    return `the test aborted with code ${error.code}`;
  }
  if (error instanceof ContractError) {
    return `host contract violation: ${error.message}`;
  }
  if (error instanceof RangeError) {
    return `the test ran out of stack: ${error.message}`;
  }
  return String(error?.stack ?? error);
}

const [modulePath, entry] = process.argv.slice(2);
if (!modulePath || !entry) {
  console.error("usage: node run_test.mjs <module.wasm> <export>");
  process.exit(2);
}

const logs = [];
let failure = null;
try {
  const guest = await instantiate(readFileSync(modulePath), {
    onLog: (value) => logs.push(value),
  });
  guest.call(entry);
  if (guest.host.openHandles > 0) {
    failure = `host contract violation: the test left ${guest.host.openHandles} file handle(s) open`;
  }
} catch (error) {
  failure = describe(error);
}
for (const value of logs) {
  console.log(`log ${value}`);
}
if (failure !== null) {
  console.error(failure);
  process.exit(1);
}
