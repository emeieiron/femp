// Runs property cases against a WebAssembly module compiled from a FeMP
// property harness. It speaks FeMP's line protocol on its standard streams:
// each request is a sequence of decimal lines fed to the module through
// `std::io::Stdin` and each answer is read from `std::io::Stdout`, with
// abnormal ends reported as `!revert <hex>`, `!oom`, `!abort <code>`,
// `!trap <message>` or `!error <message>`.
//
//   node harness.mjs <module.wasm> <manifest.json> <entries.json>
//
// `entries.json` lists the harness entry points in id order:
//   { "kind": "property", "export": "...", "params": n }
//   { "kind": "machine", "export": "...", "init": n, "commands": [n, ...] }
//   { "kind": "unsupported" }
import { readFileSync } from "node:fs";
import { createInterface } from "node:readline";

import { GuestAbort, GuestRevert, instantiate } from "./host.js";

const [modulePath, manifestPath, entriesPath] = process.argv.slice(2);
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const entries = JSON.parse(readFileSync(entriesPath, "utf8"));

let out = "";
let decoder = new TextDecoder();
const guest = await instantiate(readFileSync(modulePath), {
  manifest,
  onWrite: ({ stream, bytes }) => {
    if (stream === "stdout") {
      out += decoder.decode(bytes, { stream: true });
    }
  },
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
  if (error instanceof GuestRevert) {
    if (error.data === null) {
      return "!oom";
    }
    return `!revert ${Buffer.from(error.data).toString("hex")}`;
  }
  if (error instanceof GuestAbort) {
    return `!abort ${error.code}`;
  }
  if (error instanceof WebAssembly.RuntimeError) {
    return `!trap ${error.message}`;
  }
  return `!error ${String(error?.message ?? error).replace(/\s+/g, " ")}`;
}

async function readCount(count, into) {
  for (let index = 0; index < count; index += 1) {
    into.push(await next());
  }
}

async function invoke(entry, inputLines) {
  guest.host.setStdin(inputLines.length > 0 ? `${inputLines.join("\n")}\n` : "");
  out = "";
  decoder = new TextDecoder();
  let ended = null;
  try {
    guest.call(entry.export);
  } catch (error) {
    ended = failure(error);
  }
  out += decoder.decode();
  if (out.length > 0) {
    process.stdout.write(out.endsWith("\n") ? out : `${out}\n`);
  }
  if (ended !== null) {
    write(ended);
  }
}

for (;;) {
  const id = Number(await next());
  const entry = entries[id];
  if (!entry || entry.kind === "unsupported") {
    write(`!error entry point ${id} has no export in this module`);
    continue;
  }
  const inputLines = [];
  if (entry.kind === "machine") {
    await readCount(entry.init, inputLines);
    const stepsLine = await next();
    inputLines.push(stepsLine);
    const steps = BigInt(stepsLine);
    for (let step = 0n; step < steps; step += 1n) {
      const commandLine = await next();
      inputLines.push(commandLine);
      await readCount(entry.commands[Number(commandLine)] ?? 0, inputLines);
    }
  } else {
    await readCount(entry.params, inputLines);
  }
  await invoke(entry, inputLines);
}
