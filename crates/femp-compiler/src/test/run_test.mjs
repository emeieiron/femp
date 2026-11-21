// Runs one WebAssembly test of `femp test` in its own Node.js process.
//
//   node run_test.mjs <module.wasm> <export> <output limit in bytes>
//
// The module is instantiated with femp's host runtime (`host.js`, next to
// this file), the export is called, and the host is checked for files the
// test left open. Exit status 0 means the test passed. On a failure one line
// on standard error says what happened, and the status is 1. A test that
// panics with a code or reverts with data (`GuestRevert`) exits with status 3
// and writes `revert <data in hex>` on standard error so the runner can judge
// `#[test(should_revert)]`. Standard output carries logged values and any
// captured stdout/stderr stream writes.
import { readFileSync } from "node:fs";

import { ContractError, GuestAbort, GuestRevert, instantiate } from "./host.js";

function describe(error) {
  if (error instanceof WebAssembly.RuntimeError) {
    return `trap: ${error.message}`;
  }
  if (error instanceof GuestRevert) {
    return error.message;
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

const [modulePath, entry, limitArg] = process.argv.slice(2);
const limit = Number(limitArg);
if (!modulePath || !entry || !Number.isSafeInteger(limit)) {
  console.error("usage: node run_test.mjs <module.wasm> <export> <output limit in bytes>");
  process.exit(2);
}

let printed = 0;
const count = (length) => {
  printed += length;
  if (printed > limit) {
    throw new Error("the test exceeded its output limit");
  }
};

const logs = [];
let guest = null;
let failure = null;
let revert = null;
try {
  guest = await instantiate(readFileSync(modulePath), {
    onLog: (value) => {
      logs.push(value);
      count(`log ${value}\n`.length);
    },
    onWrite: ({ bytes }) => count(bytes.length),
  });
  guest.call(entry);
  if (guest.host.openHandles > 0) {
    failure = `host contract violation: the test left ${guest.host.openHandles} file handle(s) open`;
  }
} catch (error) {
  if (error instanceof GuestRevert && error.data !== null) {
    revert = Buffer.from(error.data).toString("hex");
  } else {
    failure = describe(error);
  }
}
for (const value of logs) {
  console.log(`log ${value}`);
}
for (const stream of ["stdout", "stderr"]) {
  const text = guest?.host[stream] ?? "";
  if (text !== "") {
    console.log(`${stream}:\n${text.replace(/\n$/, "")}`);
  }
}
if (revert !== null) {
  console.error(`revert ${revert}`);
  process.exitCode = 3;
} else if (failure !== null) {
  console.error(failure);
  process.exitCode = 1;
}
