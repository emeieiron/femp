// One table of cases, run unchanged against every built artifact. A case
// expects either a value or one of the shared error codes.

const STANDARD = [1000n, 100n, 50n, 200n];
const MAX_U64 = 18446744073709551615n;

const vested = (name, schedule, now, expect) => ({ name, op: "vested", schedule, now, expect });
const claim = (name, schedule, amount, now, expect) => ({
  name,
  op: "claim",
  schedule,
  amount,
  now,
  expect,
});

export const VECTORS = [
  vested("before the cliff", STANDARD, 149n, { value: 0n }),
  vested("at the cliff", STANDARD, 150n, { value: 250n }),
  vested("halfway", STANDARD, 200n, { value: 500n }),
  vested("just before the end", STANDARD, 299n, { value: 995n }),
  vested("at the end", STANDARD, 300n, { value: 1000n }),
  vested("long after the end", STANDARD, 1000000n, { value: 1000n }),
  vested("large amounts", [18000000000000000000n, 0n, 0n, 4n], 3n, {
    value: 13500000000000000000n,
  }),
  vested("zero total", [0n, 100n, 50n, 200n], 200n, { error: 1 }),
  vested("zero duration", [1000n, 100n, 50n, 0n], 200n, { error: 2 }),
  vested("cliff after the end", [1000n, 100n, 201n, 200n], 200n, { error: 3 }),
  vested("end overflows", [1000n, MAX_U64, 0n, 1n], 200n, { error: 4 }),
  claim("part of the vested amount", STANDARD, 200n, 200n, { value: 200n }),
  claim("all of the vested amount", STANDARD, 500n, 200n, { value: 500n }),
  claim("nothing", STANDARD, 0n, 200n, { error: 5 }),
  claim("more than vested", STANDARD, 501n, 200n, { error: 6 }),
  claim("before the cliff", STANDARD, 1n, 149n, { error: 6 }),
];
