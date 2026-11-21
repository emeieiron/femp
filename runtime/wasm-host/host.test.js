import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

import {
  CONTRACT_VERSION,
  ContractError,
  GuestAbort,
  GuestRevert,
  instantiate,
} from "./host.js";

const moduleUrl = new URL("./test/probe.wasm", import.meta.url);
const manifest = JSON.parse(
  await readFile(new URL("./test/probe.manifest.json", import.meta.url), "utf8"),
);

function load(options = {}) {
  return instantiate(moduleUrl, { manifest, ...options });
}

test("manifest uses host contract version 3", () => {
  assert.equal(CONTRACT_VERSION, 3);
  assert.equal(manifest.contract_version, 3);
});

test("64-bit integers cross as bigint", async () => {
  const guest = await load();
  assert.equal(guest.call("add", 2n, 40n), 42n);
  assert.equal(guest.call("add", 2 ** 63, 1), 9223372036854775809n);
});

test("narrow integers keep their sign and width", async () => {
  const guest = await load();
  assert.equal(guest.call("negate", 5), -5);
  assert.equal(guest.call("narrow", -1), -1);
  assert.equal(guest.call("widest", 4294967295), 4294967295);
});

test("booleans cross as booleans", async () => {
  const guest = await load();
  assert.equal(guest.call("is_even", 4), true);
  assert.equal(guest.call("is_even", 5), false);
  assert.equal(guest.call("invert", true), false);
});

test("text is placed in the module's memory for one call", async () => {
  const guest = await load();
  assert.equal(guest.call("text_length", "héllo"), 6);
  assert.equal(guest.call("text_length", new Uint8Array([1, 2, 3])), 3);
  const before = guest.instance.exports.arena_mark();
  guest.call("text_length", "x".repeat(1000));
  assert.equal(guest.instance.exports.arena_mark(), before);
});

test("replies decode to a value or an error code", async () => {
  const guest = await load();
  assert.deepEqual(guest.call("double_up_to_100", 21), { ok: true, value: 42 });
  assert.deepEqual(guest.call("double_up_to_100", 101), { ok: false, code: 7 });
});

test("the clock and the log are supplied by the host", async () => {
  const logged = [];
  const guest = await load({ now: () => 1234, onLog: (value) => logged.push(value) });
  assert.equal(guest.call("stamp"), 1234n);
  assert.equal(guest.call("report", 99n), undefined);
  assert.deepEqual(logged, [99n]);
});

test("abort surfaces as an exception carrying the code and restarts the instance", async () => {
  const guest = await load();
  const initial = guest.instance;
  assert.throws(
    () => guest.call("stop", 6),
    (error) => error instanceof GuestAbort && error.code === 6,
  );
  assert.notEqual(guest.instance, initial);
  assert.equal(guest.call("add", 1n, 2n), 3n);
});

test("panics and assert messages surface as GuestRevert with decoded reasons", async () => {
  const guest = await load();
  const initial = guest.instance;
  assert.throws(
    () => guest.call("add", (1n << 64n) - 1n, 1n),
    (error) =>
      error instanceof GuestRevert &&
      error.data !== null &&
      error.reason === "arithmetic overflow",
  );
  assert.notEqual(guest.instance, initial);

  assert.throws(
    () => guest.call("check_positive", -5),
    (error) =>
      error instanceof GuestRevert &&
      error.data !== null &&
      error.reason === "must be positive",
  );
  assert.equal(guest.call("check_positive", 7), 7);
});

test("standard input and output streams cross the host contract", async () => {
  const writes = [];
  const guest = await load({
    stdin: "hello wasm\n",
    onWrite: (entry) => writes.push(entry),
  });
  assert.equal(guest.call("echo_line"), 10);
  assert.equal(guest.host.stdout, "hello wasm\n");
  assert.equal(guest.host.stderr, "");
  assert.equal(writes.length > 0, true);

  guest.host.clearStreams();
  guest.host.setStdin("second\n");
  assert.equal(guest.call("echo_line"), 6);
  assert.equal(guest.host.stdout, "second\n");
});

test("files are read-only, support bulk reads, and are tracked by handle", async () => {
  const guest = await load({
    files: {
      "notes.txt": "one\ntwo\n",
      "nums.bin": new Uint8Array([10, 20, 30, 40]),
    },
  });
  assert.deepEqual(guest.call("file_size", "notes.txt"), { ok: true, value: 8 });
  assert.deepEqual(guest.call("file_size", "missing.txt"), { ok: false, code: 1 });
  assert.deepEqual(guest.call("file_size", ""), { ok: false, code: 3 });
  assert.deepEqual(guest.call("file_sum", "nums.bin"), { ok: true, value: 100 });
  assert.equal(guest.host.openHandles, 0);

  guest.call("open_and_keep", "notes.txt");
  assert.equal(guest.host.openHandles, 1);
});

test("files can be added after instantiation", async () => {
  const guest = await load();
  assert.deepEqual(guest.call("file_size", "later.txt"), { ok: false, code: 1 });
  guest.host.setFile("later.txt", new Uint8Array([1, 2, 3]));
  assert.deepEqual(guest.call("file_size", "later.txt"), { ok: true, value: 3 });
  guest.host.setFile("later.txt", "four");
  assert.deepEqual(guest.call("file_size", "later.txt"), { ok: true, value: 4 });
});

test("arguments are checked against the manifest", async () => {
  const guest = await load();
  assert.throws(() => guest.call("add", 1n), ContractError);
  assert.throws(() => guest.call("add", 1.5, 2n), /must be an integer/);
  assert.throws(() => guest.call("text_length", 5), /must be text/);
  assert.throws(() => guest.call("text_length", "x".repeat(65537)), /limit is 65536/);
  assert.throws(() => guest.call("no_such_export"), /no exported function/);
});

test("a module that differs from its manifest is refused", async () => {
  const bytes = new Uint8Array(await readFile(moduleUrl));
  bytes[bytes.length - 1] ^= 0xff;
  await assert.rejects(instantiate(bytes, { manifest }), /SHA-256/);

  const wrongExports = { ...manifest, exports: manifest.exports.slice(1) };
  await assert.rejects(instantiate(moduleUrl, { manifest: wrongExports }), /exports differ/);

  const wrongVersion = { ...manifest, contract_version: 99 };
  await assert.rejects(instantiate(moduleUrl, { manifest: wrongVersion }), /contract version 99/);
});

test("a module can run without a manifest using raw values", async () => {
  const guest = await instantiate(await readFile(moduleUrl));
  assert.equal(guest.call("add", 2n, 3n), 5n);
});

test("an import the host lacks is named", async () => {
  // A module importing `env.custom` and nothing else.
  const bytes = new Uint8Array([
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x04, 0x01, 0x60, 0x00, 0x00, 0x02,
    0x0e, 0x01, 0x03, 0x65, 0x6e, 0x76, 0x06, 0x63, 0x75, 0x73, 0x74, 0x6f, 0x6d, 0x00, 0x00,
  ]);
  await assert.rejects(instantiate(bytes), /imports env\.custom, which the host does not provide/);
  await instantiate(bytes, { imports: { env: { custom() {} } } });
});
