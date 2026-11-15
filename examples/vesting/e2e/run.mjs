// Runs the same cases against the contract, the command-line program and
// the WebAssembly module, and fails unless all of them agree with the
// expected results. Build the artifacts first with `femp build`.
//
//   node e2e/run.mjs
import * as evm from "./evm.mjs";
import * as native from "./native.mjs";
import * as wasm from "./wasm.mjs";
import { VECTORS } from "./vectors.mjs";

const PLATFORMS = { evm, native, wasm };

function show(outcome) {
  return "error" in outcome ? `error ${outcome.error}` : String(outcome.value);
}

function same(a, b) {
  return show(a) === show(b);
}

function printTable(columns, rows) {
  const widths = columns.map((column, index) =>
    Math.max(column.length, ...rows.map((row) => row[index].length)),
  );
  for (const row of [columns, ...rows]) {
    console.log(row.map((cell, index) => cell.padEnd(widths[index])).join("  ").trimEnd());
  }
}

const runners = {};
for (const [name, platform] of Object.entries(PLATFORMS)) {
  const runner = await platform.start();
  if (runner.skipped) {
    console.log(`${name}: skipped (${runner.skipped})`);
  } else {
    runners[name] = runner;
  }
}

const names = Object.keys(runners);
const rows = [];
let mismatches = 0;
try {
  for (const vector of VECTORS) {
    const outcomes = [];
    for (const name of names) {
      outcomes.push(await runners[name].run(vector));
    }
    const agrees = outcomes.every((outcome) => same(outcome, vector.expect));
    mismatches += agrees ? 0 : 1;
    rows.push([
      `${vector.op}: ${vector.name}`,
      show(vector.expect),
      ...outcomes.map(show),
      agrees ? "ok" : "MISMATCH",
    ]);
  }
} finally {
  for (const runner of Object.values(runners)) {
    runner.stop();
  }
}

console.log();
printTable(["case", "expected", ...names, ""], rows);
console.log();

if (names.length === 0) {
  console.error("Nothing to run: build the artifacts with `femp build` first.");
  process.exit(1);
}
if (mismatches > 0) {
  console.error(`${mismatches} of ${VECTORS.length} cases disagree.`);
  process.exit(1);
}
console.log(`All ${VECTORS.length} cases agree on ${names.join(", ")}.`);
