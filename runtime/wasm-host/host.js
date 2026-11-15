// Host runtime for WebAssembly modules compiled from Fe. It supplies the
// imports of the Fe host contract, checks a module against its build
// manifest, and converts values at the module boundary. It has no
// dependencies and runs in browsers and in Node.js 20 or newer.

export const MANIFEST_VERSION = 1;
export const CONTRACT_VERSION = 1;

const IMPORT_MODULE = "env";
const MAX_TEXT_BYTES = 65536;
const FILE_NOT_FOUND = 1;
const FILE_OUT_OF_RANGE = 2;
const FILE_BAD_NAME = 3;

export class ContractError extends Error {
  constructor(message) {
    super(message);
    this.name = "ContractError";
  }
}

export class GuestAbort extends Error {
  constructor(code) {
    super(`the module aborted with code ${code}`);
    this.name = "GuestAbort";
    this.code = code;
  }
}

export async function instantiate(source, options = {}) {
  const bytes = await loadBytes(source);
  const host = new Host(options);
  const imports = mergeImports(host.imports(), options.imports);

  const module = await WebAssembly.compile(bytes);
  requireImports(module, imports);
  if (options.manifest) {
    await verifyManifest(options.manifest, bytes, module);
  }

  const instance = await WebAssembly.instantiate(module, imports);
  host.memory = instance.exports.memory;
  return new Guest(instance, options.manifest, host);
}

export class Guest {
  constructor(instance, manifest, host) {
    this.instance = instance;
    this.host = host;
    this.signatures = new Map(
      (manifest?.exports ?? [])
        .filter((entry) => entry.kind === "function" && !entry.role)
        .map((entry) => [entry.name, entry]),
    );
  }

  // With a manifest, arguments and the result are converted according to
  // the export's Fe types; without one they pass through as raw values.
  call(name, ...args) {
    const target = this.instance.exports[name];
    if (typeof target !== "function") {
      throw new ContractError(`the module has no exported function \`${name}\``);
    }
    const signature = this.signatures.get(name);
    if (!signature) {
      return target(...args);
    }
    if (args.length !== signature.params.length) {
      throw new ContractError(
        `\`${name}\` takes ${signature.params.length} arguments, but ${args.length} were given`,
      );
    }

    const arena = new Arena(this.instance.exports);
    try {
      const lowered = signature.params.flatMap((param, index) =>
        lowerArgument(name, param, args[index], arena),
      );
      return liftResult(signature.result, target(...lowered));
    } finally {
      arena.release();
    }
  }
}

// Text arguments live in the module's memory only for the duration of one
// call: everything allocated after the mark is freed when the call ends.
class Arena {
  constructor(exports) {
    this.exports = exports;
    this.mark = undefined;
  }

  place(bytes) {
    const { alloc, arena_mark: arenaMark, memory } = this.exports;
    if (typeof alloc !== "function" || typeof arenaMark !== "function") {
      throw new ContractError("the module takes text but exports no allocator");
    }
    if (this.mark === undefined) {
      this.mark = arenaMark();
    }
    const address = alloc(bytes.length) >>> 0;
    new Uint8Array(memory.buffer, address, bytes.length).set(bytes);
    return address;
  }

  release() {
    if (this.mark !== undefined) {
      this.exports.arena_release(this.mark);
    }
  }
}

class Host {
  constructor(options) {
    this.now = options.now ?? (() => Date.now());
    this.onLog = options.onLog ?? ((value) => console.log(value));
    this.files = new Map(
      Object.entries(options.files ?? {}).map(([name, contents]) => [name, toBytes(contents)]),
    );
    this.handles = new Map();
    this.nextHandle = 1;
    this.memory = undefined;
  }

  get openHandles() {
    return this.handles.size;
  }

  // Adds or replaces a file the module may open from now on.
  setFile(name, contents) {
    this.files.set(name, toBytes(contents));
  }

  imports() {
    return {
      [IMPORT_MODULE]: {
        sys_wasm_log_i32: (value) => this.onLog(value),
        sys_wasm_log_u64: (value) => this.onLog(BigInt.asUintN(64, value)),
        sys_wasm_now_ms: () => BigInt(Math.trunc(this.now())),
        sys_wasm_abort: (code) => {
          throw new GuestAbort(code >>> 0);
        },
        sys_wasm_files_open: (address, length) => this.openFile(address, length),
        sys_wasm_files_size: (handle) => replyOk(this.file(handle).length),
        sys_wasm_files_read_byte: (handle, index) => this.readByte(handle, index),
        sys_wasm_files_close: (handle) => {
          this.file(handle);
          this.handles.delete(handle >>> 0);
        },
      },
    };
  }

  openFile(address, length) {
    const name = this.readText(address >>> 0, length >>> 0);
    if (name === undefined || name.length === 0) {
      return replyError(FILE_BAD_NAME);
    }
    const contents = this.files.get(name);
    if (contents === undefined) {
      return replyError(FILE_NOT_FOUND);
    }
    const handle = this.nextHandle++;
    this.handles.set(handle, contents);
    return replyOk(handle);
  }

  readByte(handle, index) {
    const contents = this.file(handle);
    const position = index >>> 0;
    return position < contents.length ? replyOk(contents[position]) : replyError(FILE_OUT_OF_RANGE);
  }

  file(handle) {
    const contents = this.handles.get(handle >>> 0);
    if (contents === undefined) {
      throw new ContractError(`the module used file handle ${handle >>> 0}, which is not open`);
    }
    return contents;
  }

  readText(address, length) {
    if (length > MAX_TEXT_BYTES) {
      throw new ContractError(`the module passed ${length} bytes of text; the limit is ${MAX_TEXT_BYTES}`);
    }
    if (address + length > this.memory.buffer.byteLength) {
      throw new ContractError("the module passed text that lies outside its memory");
    }
    try {
      const bytes = new Uint8Array(this.memory.buffer, address, length);
      return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    } catch {
      return undefined;
    }
  }
}

// A reply is one 64-bit word: the tag in the low half (0 for success, an
// error number otherwise) and the payload in the high half.
function replyOk(value) {
  return BigInt(value >>> 0) << 32n;
}

function replyError(code) {
  return BigInt(code);
}

function decodeReply(word) {
  const bits = BigInt.asUintN(64, word);
  const tag = Number(bits & 0xffffffffn);
  return tag === 0 ? { ok: true, value: Number(bits >> 32n) } : { ok: false, code: tag };
}

function lowerArgument(exportName, param, value, arena) {
  const where = `argument \`${param.name}\` of \`${exportName}\``;
  if (param.type === "HostText") {
    if (typeof value !== "string" && !(value instanceof Uint8Array)) {
      throw new ContractError(`${where} must be text`);
    }
    const bytes = toBytes(value);
    if (bytes.length > MAX_TEXT_BYTES) {
      throw new ContractError(`${where} is ${bytes.length} bytes; the limit is ${MAX_TEXT_BYTES}`);
    }
    return [arena.place(bytes), bytes.length];
  }
  if (param.type === "bool") {
    return [value ? 1 : 0];
  }
  if (typeof value !== "bigint" && !Number.isInteger(value)) {
    throw new ContractError(`${where} must be an integer`);
  }
  return [isWide(param.type) ? BigInt(value) : Number(value)];
}

function liftResult(result, raw) {
  if (!result) {
    return undefined;
  }
  const type = result.type;
  if (type === "Reply") {
    return decodeReply(raw);
  }
  if (type === "bool") {
    return (raw & 1) === 1;
  }
  if (isWide(type)) {
    return type === "u64" ? BigInt.asUintN(64, raw) : BigInt.asIntN(64, raw);
  }
  const bits = narrowBits(type);
  const unused = 32 - bits;
  return type.startsWith("u") ? (raw << unused) >>> unused : (raw << unused) >> unused;
}

function isWide(type) {
  return type === "u64" || type === "i64";
}

// `usize` and `isize` are 32 bits wide in a module.
function narrowBits(type) {
  const bits = Number(type.slice(1));
  return Number.isInteger(bits) ? bits : 32;
}

function toBytes(contents) {
  return typeof contents === "string" ? new TextEncoder().encode(contents) : contents;
}

async function loadBytes(source) {
  const resolved = await source;
  if (resolved instanceof ArrayBuffer) {
    return new Uint8Array(resolved);
  }
  if (ArrayBuffer.isView(resolved)) {
    return new Uint8Array(resolved.buffer, resolved.byteOffset, resolved.byteLength);
  }
  if (typeof Response !== "undefined" && resolved instanceof Response) {
    return new Uint8Array(await resolved.arrayBuffer());
  }
  if (resolved instanceof URL) {
    if (resolved.protocol === "file:") {
      const { readFile } = await import("node:fs/promises");
      return new Uint8Array(await readFile(resolved));
    }
    const response = await fetch(resolved);
    if (!response.ok) {
      throw new ContractError(`could not load ${resolved}: HTTP ${response.status}`);
    }
    return new Uint8Array(await response.arrayBuffer());
  }
  throw new ContractError("a module must be given as bytes, a URL or a Response");
}

function mergeImports(standard, extra = {}) {
  const merged = { ...extra };
  merged[IMPORT_MODULE] = { ...standard[IMPORT_MODULE], ...extra[IMPORT_MODULE] };
  return merged;
}

function requireImports(module, imports) {
  for (const wanted of WebAssembly.Module.imports(module)) {
    if (imports[wanted.module]?.[wanted.name] === undefined) {
      throw new ContractError(
        `the module imports ${wanted.module}.${wanted.name}, which the host does not provide`,
      );
    }
  }
}

async function verifyManifest(manifest, bytes, module) {
  if (manifest.manifest_version !== MANIFEST_VERSION) {
    throw new ContractError(`unsupported manifest version ${manifest.manifest_version}`);
  }
  if (manifest.contract_version !== CONTRACT_VERSION) {
    throw new ContractError(`unsupported host contract version ${manifest.contract_version}`);
  }

  const actual = await sha256(bytes);
  if (actual !== manifest.module.sha256) {
    throw new ContractError(
      `the module has SHA-256 ${actual} but the manifest describes ${manifest.module.sha256}`,
    );
  }

  const key = (entry) => `${entry.module ?? ""}.${entry.name}:${entry.kind}`;
  requireSameEntries("imports", WebAssembly.Module.imports(module).map(key), manifest.imports.map(key));
  requireSameEntries("exports", WebAssembly.Module.exports(module).map(key), manifest.exports.map(key));
}

function requireSameEntries(what, actual, described) {
  const expected = new Set(described);
  const mismatch =
    actual.find((entry) => !expected.has(entry)) ?? described.find((entry) => !actual.includes(entry));
  if (mismatch !== undefined) {
    throw new ContractError(`the module's ${what} differ from its manifest at ${mismatch}`);
  }
}

async function sha256(bytes) {
  const subtle = globalThis.crypto?.subtle;
  if (!subtle) {
    throw new ContractError(
      "cannot verify the module: Web Crypto is unavailable (serve the page over https or from localhost)",
    );
  }
  const digest = new Uint8Array(await subtle.digest("SHA-256", bytes));
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("");
}
