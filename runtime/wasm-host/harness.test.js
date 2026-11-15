import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const here = (path) => fileURLToPath(new URL(path, import.meta.url));

const ENTRIES = [
  { kind: "property", export: "femp_entry_0", params: 2 },
  { kind: "property", export: "femp_entry_1", params: 1 },
  { kind: "machine", export: "femp_entry_2", init: 1, commands: [1, 0] },
  { kind: "unsupported" },
];

// Sends `request` to a fresh runner and resolves with its output lines.
async function exchange(request) {
  const entries = join(tmpdir(), `femp-entries-${process.pid}.json`);
  await writeFile(entries, JSON.stringify(ENTRIES));
  const runner = spawn(process.execPath, [
    here("./harness.mjs"),
    here("./test/harness_probe.wasm"),
    here("./test/harness_probe.manifest.json"),
    entries,
  ]);
  let output = "";
  runner.stdout.on("data", (chunk) => {
    output += chunk;
  });
  runner.stdin.end(request);
  const code = await new Promise((resolve) => runner.on("close", resolve));
  assert.equal(code, 0);
  return output.trim().split("\n");
}

test("answers properties with one value per line", async () => {
  const lines = await exchange("0\n3\n0\n0\n20\n0\n0\n20\n1\n1\n-5\n");
  assert.deepEqual(lines, ["true", "false", "true", "5"]);
});

test("reports a trap and keeps answering", async () => {
  const lines = await exchange("1\n-128\n1\n7\n");
  assert.deepEqual(lines, ["!trap unreachable", "-7"]);
});

test("runs a machine sequence from a host file", async () => {
  // Start at 10, add 5, halve, add 1.
  const lines = await exchange("2\n10\n3\n0\n5\n1\n0\n1\n");
  assert.deepEqual(lines, ["@", "10", "@", "15", "@", "7", "@", "8"]);
});

test("names entry points it cannot run", async () => {
  const lines = await exchange("3\n");
  assert.match(lines[0], /^!error/);
});
