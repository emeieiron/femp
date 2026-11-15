// Runs property cases against a WebAssembly module compiled from a FeMP
// property harness. It speaks FeMP's line protocol on its standard streams:
// each request is a sequence of decimal lines and each answer is one value
// per line, with abnormal ends reported as `!abort <code>`, `!trap <message>`
// or `!error <message>`.
//
//   node harness.mjs <module.wasm> <manifest.json> <entries.json>
//
// `entries.json` lists the harness entry points in id order:
//   { "kind": "property", "export": "...", "params": n }
//   { "kind": "machine", "export": "...", "init": n, "commands": [n, ...] }
// A machine run reads its operations as little-endian 64-bit words from a
// host file and logs a marker (`log_i32`) before each record of values
// (`log_u64`).
import { readFileSync } from "node:fs";
import { createInterface } from "node:readline";

import { GuestAbort, instantiate } from "./host.js";

const SCRIPT_FILE = "femp_script";

const [modulePath, manifestPath, entriesPath] = process.argv.slice(2);
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const entries = JSON.parse(readFileSync(entriesPath, "utf8"));

let logs = [];
const guest = await instantiate(readFileSync(modulePath), {
  manifest,
  onLog: (value) => logs.push(value),
});

const lines = createInterface({ input: process.stdin, terminal: false })[Symbol.asyncIterator]();

async function next() {
  const { value, done } = await lines.next();
  if (done) {
    process.exit(0);
  }
  return value.trim();
}

function write(text) {
  process.stdout.write(`${text}\n`);
}

function failure(error) {
  if (error instanceof GuestAbort) {
    return `!abort ${error.code}`;
  }
  if (error instanceof WebAssembly.RuntimeError) {
    return `!trap ${error.message}`;
  }
  return `!error ${String(error?.message ?? error).replace(/\s+/g, " ")}`;
}

function argument(text) {
  return BigInt(text);
}

function show(value) {
  return typeof value === "boolean" ? String(value) : value.toString();
}

async function runProperty(entry) {
  const args = [];
  for (let index = 0; index < entry.params; index += 1) {
    args.push(argument(await next()));
  }
  const params = manifest.exports.find((item) => item.name === entry.export)?.params ?? [];
  const lowered = args.map((value, index) => (params[index]?.type === "bool" ? value === 1n : value));
  try {
    write(show(guest.call(entry.export, ...lowered)));
  } catch (error) {
    write(failure(error));
  }
}

async function runMachine(entry) {
  const words = [];
  const read = async (count) => {
    for (let index = 0; index < count; index += 1) {
      words.push(BigInt.asUintN(64, argument(await next())));
    }
  };
  await read(entry.init);
  const steps = argument(await next());
  words.push(steps);
  for (let step = 0n; step < steps; step += 1n) {
    const command = argument(await next());
    words.push(command);
    await read(entry.commands[Number(command)] ?? 0);
  }

  const script = new Uint8Array(words.length * 8);
  const view = new DataView(script.buffer);
  words.forEach((word, index) => view.setBigUint64(index * 8, word, true));
  guest.host.setFile(SCRIPT_FILE, script);

  logs = [];
  let ended;
  try {
    guest.call(entry.export, SCRIPT_FILE);
  } catch (error) {
    ended = failure(error);
  }
  for (const value of logs) {
    write(typeof value === "bigint" ? value.toString() : "@");
  }
  if (ended) {
    write(ended);
  }
}

for (;;) {
  const id = Number(await next());
  const entry = entries[id];
  if (!entry) {
    write(`!error no entry point ${id}`);
    continue;
  }
  if (entry.kind === "machine") {
    await runMachine(entry);
  } else {
    await runProperty(entry);
  }
}
