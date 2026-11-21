export declare const MANIFEST_VERSION: number;
export declare const CONTRACT_VERSION: number;

/** Meaning of Solidity `Panic(uint256)` codes reported by Fe modules. */
export declare const PANIC_CODES: Readonly<Record<number, string>>;

/** The module or the caller broke the host contract. */
export declare class ContractError extends Error {}

/** The module ended a call through `std::wasm::Abort`. */
export declare class GuestAbort extends Error {
  readonly code: number;
  constructor(code: number);
}

/**
 * The module panicked with a code or reverted with data (e.g. failed `assert!`,
 * arithmetic overflow, division by zero, or index out of bounds).
 */
export declare class GuestRevert extends Error {
  readonly data: Uint8Array | null;
  readonly reason: string | null;
  constructor(data: Uint8Array | null);
}

/** Decodes Solidity `Panic(uint256)` or `Error(string)` revert bytes. */
export declare function revertReason(data: Uint8Array): string | null;

export type ModuleSource =
  | BufferSource
  | URL
  | Response
  | Promise<BufferSource | URL | Response>;

/** The result of an export that returns a tagged reply. */
export type Reply = { ok: true; value: number } | { ok: false; code: number };

export interface StreamWrite {
  stream: "stdout" | "stderr";
  bytes: Uint8Array;
}

export interface ManifestValueType {
  type: string;
  wasm: string[];
}

export interface ManifestParam extends ManifestValueType {
  name: string;
}

export interface ManifestImport {
  module: string;
  name: string;
  kind: string;
  type?: { params: string[]; results: string[] };
}

export interface ManifestExport {
  name: string;
  kind: string;
  role?: string;
  params?: ManifestParam[];
  result?: ManifestValueType | null;
}

export interface Manifest {
  manifest_version: number;
  contract_version: number;
  module: { file: string; sha256: string; size: number; initial_memory_pages: number };
  optimization: string;
  compiler: string;
  imports: ManifestImport[];
  exports: ManifestExport[];
}

export interface HostOptions {
  /** When given, the module must match this manifest exactly. */
  manifest?: Manifest;
  /** Implementations of the program's own imported functions. */
  imports?: Record<string, Record<string, unknown>>;
  /** Read-only files the module may open by name. */
  files?: Record<string, string | Uint8Array>;
  /** Initial standard input buffer (empty by default). */
  stdin?: string | Uint8Array;
  /** Milliseconds since the Unix epoch. Defaults to the system clock. */
  now?: () => number;
  /** Receives values the module logs. Defaults to the console. */
  onLog?: (value: number | bigint) => void;
  /** Receives each write to standard output or standard error. */
  onWrite?: (write: StreamWrite) => void;
}

export interface HostState {
  /** Files the module opened and has not closed. */
  readonly openHandles: number;
  /** Text written to standard output so far. */
  readonly stdout: string;
  /** Text written to standard error so far. */
  readonly stderr: string;
  /** Adds or replaces a file the module may open from now on. */
  setFile(name: string, contents: string | Uint8Array): void;
  /** Replaces the standard input buffer and resets the read position. */
  setStdin(contents: string | Uint8Array): void;
  /** Clears captured stdout and stderr buffers. */
  clearStreams(): void;
}

export declare class Guest {
  readonly instance: WebAssembly.Instance;
  readonly host: HostState;
  call(name: string, ...args: unknown[]): unknown;
}

export declare function instantiate(source: ModuleSource, options?: HostOptions): Promise<Guest>;
