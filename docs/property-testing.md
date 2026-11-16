# Property testing

Fe Property Testing checks properties of a FeMP project with generated inputs, on every
platform at once. You declare a property once in Fe. `femp prop` then:

1. generates inputs from a seed,
2. runs the same code compiled for the EVM, as a native executable and as WebAssembly,
3. compares what each target did, and
4. shrinks any failure to a minimal counterexample, which it saves for later runs.

> Write a property once. Test it everywhere. Minimize every failure.

## Declaring entry points

Entry points are ordinary Fe functions and structs marked with attributes. They need no
compiler support, because the Fe compiler accepts attributes it does not know.

### Properties

```fe
use ingot::schedule::Schedule

#[property(cases = 500)]
#[gen(cliff = "0..=10_000", duration = "1..=10_000")]
pub fn vested_never_exceeds_total(total: u64, start: u64, cliff: u64, duration: u64, now: u64) -> bool {
    match Schedule::new(total, start, cliff, duration) {
        Result::Ok(schedule) => schedule.vested_at(now) <= total,
        Result::Err(_) => true,
    }
}
```

A `#[property]` returns `bool`. It passes when every target returns `true` for every case.

### Differentials

```fe
#[differential]
pub fn schedule_error(total: u64, start: u64, cliff: u64, duration: u64) -> u32 {
    match Schedule::new(total, start, cliff, duration) {
        Result::Ok(_) => 0,
        Result::Err(error) => error.code(),
    }
}
```

A `#[differential]` returns any primitive. It passes when every target produces the same
outcome. A panic counts as an outcome, so agreeing panics are fine and so are agreeing values.

### Machines

A `#[machine]` struct is a state machine:

- an `#[init]` function builds the first state;
- `#[command]` methods each return the next state;
- after the initializer and after every step, each `#[invariant]` must hold and each
  `#[observe]` value is recorded.

```fe
#[machine(steps = "1..=24", runs = 100)]
pub struct VestingModel { pub position: Position, pub now: u64, pub last_error: u32 }

impl Copy for VestingModel {}

impl VestingModel {
    #[init]
    #[gen(total = "1..=1_000_000", duration = "1..=2_000")]
    pub fn open(total: u64, start: u64, cliff: u64, duration: u64) -> VestingModel { ... }

    #[command]
    #[gen(seconds = "0..=500")]
    pub fn wait(self, seconds: u64) -> VestingModel { ... }

    #[command(weight = 2)]
    pub fn claim(self, amount: u64) -> VestingModel { ... }

    #[invariant]
    pub fn claims_never_exceed_the_total(self) -> bool { ... }

    #[observe]
    pub fn claimed(self) -> u64 { ... }
}
```

A run generates a sequence of commands, and the targets must agree record by record. Machine
structs must implement `Copy`.

### Contract machines

A machine declared in an `evm` module may use the `Create`, `Call`, `Ctx` and `Log` effects.
That lets it deploy the project's contracts and send them messages:

```fe
#[machine(steps = "1..=16", clock = "0..=150")]
pub struct VestingContractMachine { pub vesting: Address, pub expected: Position, ... }

impl VestingContractMachine {
    #[init]
    pub fn deploy(total: u64, ...) -> VestingContractMachine uses (create: mut Create) {
        let vesting = create.create2<Vesting>(value: Wei::zero(), args: (total, ...), salt: 0)
        ...
    }

    #[command]
    pub fn claim(self, amount: u64) -> VestingContractMachine uses (call: mut Call, ctx: Ctx) {
        let accepted = self.vesting.try_call(VestingMsg::Claim { amount }).success()
        ...
    }

    #[invariant]
    pub fn stores_the_expected_total(self) -> bool uses (call: Call) { ... }
}
```

How a contract machine runs on the EVM:

- Each command is its own transaction, and the machine's state persists in storage between
  transactions.
- `clock` advances the block timestamp by a generated number of seconds before each step. The
  delay shrinks toward zero.
- A command whose transaction reverts with a custom error is recorded as rejected: its effects
  are rolled back and the run continues. A panic or a halt fails the run.

### Specs

A public `#[spec]` function whose parameters and result are primitives runs as an entry point
too. A case its `requires` rejects is discarded and counted (`256 cases (31 rejected)`); a
failed `ensures` or any other panic fails it. A WebAssembly module traps alike when a spec
rejects a case and when it fails, so specs run on the EVM and natively, and show as not run on
`wasm`. See [proving](proving.md) for what specs state and how `femp prove` uses them.

### Rules

| | |
| --- | --- |
| Parameter and result types | `bool`, `u8`…`u256` and `i8`…`i256`. `usize` and `isize` are rejected because their width differs per backend. |
| Wide types | Values wider than 64 bits cannot cross a WebAssembly export, so those entry points run on the EVM and natively and show as not run on `wasm`. |
| Visibility | Entry points and machine methods must be `pub`. |
| Shape | Entry points cannot be generic or conditionally compiled. |
| Effects | Entry points in shared modules cannot declare effects. |
| Location | Entry points may live in any module of an ingot. One in a shared module runs on every platform; one in a platform module runs on that platform only. |
| Names | Names must be unique within a module. |

`femp prop --list` shows what was found. A malformed declaration is reported with its file and
line.

## Generators

`#[gen(param = "spec")]` chooses how a parameter is generated. A parameter without a spec uses
`any`.

| Spec | Values |
| --- | --- |
| `any` | Every value of the type. |
| `lo..=hi`, `lo..hi` | A range. Bounds are decimal or `0x` hex, may contain `_`, and may be `min` or `max`. |
| `one_of(a, b, ...)` | One of the listed values; earlier ones count as simpler. |
| `const(a)` | Always `a`. |

How cases are generated:

- **Biased toward interesting values.** Random draws favour values where integer code tends to
  break, such as the range bounds, zero and its neighbours, the type's limits, and powers of
  two with their neighbours. They also favour small magnitudes as much as large ones.
- **Fixed first cases.** The first case of an entry uses the simplest value of every parameter,
  and the second the value farthest from zero.
- **Exhaustive for small domains.** When every input of a property fits in its case budget, the
  property is enumerated exhaustively instead. `--exhaustive` raises that limit to 2²⁰ inputs.

Domain-specific values are built in Fe from primitive parameters. For example, a property takes
four integers and builds a `Schedule` from them. Structs, enums and collections as parameters
are planned together with Fe codec support.

Custom generators are registered by name in Rust (`femp_prop::generators::Registry`) and
written as `name(args)` in a spec.

## Running

```sh
femp prop                     # every entry point on every platform
femp prop -f claim            # entry points whose name contains `claim`
femp prop -t evm -t native    # some platforms only
femp prop --seed 0x2a         # reproduce a run
femp prop --cases 2000        # more cases (or machine runs) per entry point
femp prop -O 0,2              # each platform at two optimization levels
```

| Option | Effect |
| --- | --- |
| `--seed <n>` | Seed for case generation, decimal or `0x` hex. It defaults to a fresh random seed, which is printed. `FEMP_PROP_SEED` sets it too. |
| `--cases <n>` | Cases or machine runs per entry point, overriding `cases` and `runs` in the attributes. The default is 256 cases and 100 runs. |
| `--exhaustive` | Enumerate every input of entry points with up to 2²⁰ inputs. |
| `-O <levels>` | Compile every platform at each listed level. Each level is its own target, such as `evm-O0` and `evm-O2`, so the run also compares optimization levels. |
| `--compare <mode>` | `normalized` (default), `strict` or `lenient`; see below. |
| `--no-shrink` | Report failures as found. |
| `--max-shrinks <n>` | Most replays spent minimizing one failure (default 2000). |
| `--no-regressions` | Neither replay nor store regressions. |
| `--timeout <s>` | Longest a single case may run (default 10 seconds). |
| `--list` | Show the entry points and their locations. |
| `--json` | Also write the report to `build/prop/report.json`. |

The command exits with a failure when an entry point fails, or when one cannot be run.

## Outcomes and comparison

Every target's result is normalized before targets are compared:

| Outcome | EVM | Native | WebAssembly |
| --- | --- | --- | --- |
| returned a value | the ABI word | the printed value | the export's result |
| panicked (with a reason) | `Panic(uint256)` codes, `Error(string)`, or an empty revert | the `panicked: ...` line before the abort | — |
| panicked (reason unknown) | — | — | an `unreachable` trap |
| reverted | other revert data | — | — |
| aborted | — | — | `abort(code)` |
| trapped | out of gas, other halts | a crash or a timeout | another trap or a timeout |

Panic reasons are arithmetic overflow, division by zero, assertion failed, index out of bounds,
out of memory, an explicit panic, or a message.

| Mode | Agreement |
| --- | --- |
| `normalized` | Values must be equal. Panics agree unless both report a reason and the reasons differ, because WebAssembly traps carry no reason. |
| `strict` | Outcomes must be identical. |
| `lenient` | Values must be equal; any two abnormal outcomes agree. |

A case fails in one of two ways:

- **Diverges**: the targets disagree. Divergence is the more specific finding, so it is
  reported first.
- **Fails**: the targets agree, but a property is not `true`, an invariant does not hold, a
  machine step panics, or a target traps.

## Shrinking

Every case is a function of a tape of bounded choices, and generators map smaller choices to
simpler values. To shrink a failure, FeMP edits the tape and replays it through the same
generators, so a constrained value stays within its constraint.

Shrinking does the following:

- **Edits the tape.** It deletes machine steps, zeroes them, lowers choices by binary search,
  moves magnitude from one choice to the next, and removes single choices.
- **Keeps only simpler tapes that still fail the same way.** A tape is simpler when it is
  shorter, or equally long and smaller choice by choice. A change is kept only if the case
  still fails in the same way (diverges or fails), so shrinking always ends.
- **Respects a budget.** The `--max-shrinks` limit bounds the number of replays.

The planted-bug fixture shows the results:

- a divergence that starts at 1000 is reported as `x = 1000`;
- a property that is false from 250 is reported as `x = 250`;
- a machine bug that needs three large additions is reported as exactly those three steps.

## Regressions

A minimized failure is saved to `regressions/<module>/<entry>.json` with:

- the seed and case number;
- the choice tape;
- the counterexample;
- each target's outcome;
- the femp and Fe versions and the optimization levels.

Every later run replays the stored cases before generating new ones, so commit the directory to
keep them. If a declaration changes so that a stored tape no longer produces the stored
counterexample, the stored values are replayed directly. If they no longer fit at all, the
entry is noted as stale and skipped.

## Reports

```
FAIL  core::rounding  diverges across targets after 2 cases
      x = 1000
      evm     returned 1000
      native  returned 1000
      wasm    returned 999
      seed 0x0000000000000007, case 1, minimized in 78 runs
      saved to regressions/core/rounding.json
      reproduce: femp prop -f rounding --seed 0x0000000000000007
```

The report holds no timings, so two runs with the same seed print the same report.

## How it runs

For each module with entry points, and for each target, the platform adapter writes a harness
ingot under `build/prop/<target>/<module>/`, compiles it with `fe`, and starts an executor:

| Platform | Harness | Execution |
| --- | --- | --- |
| `evm` | A contract with one message per entry point. Machines keep their state in storage. | An embedded EVM (revm). Properties are calls whose effects are discarded; each machine run starts from the freshly deployed state. |
| `native` | A `main` that reads requests from standard input and prints one value per line. | One long-lived process, restarted after a case aborts. |
| `wasm` | One export per property. Per machine, an export that reads the whole sequence from a host file and logs each record. | `runtime/wasm-host/harness.mjs` in Node.js, through the same host runtime as the generated bindings. |

The engine (`femp-prop`) only sees an executor interface. Generators, shrink passes and
comparison strategies are each behind a trait or registry, and a new platform takes part by
implementing `Platform::property_harness`. See
[adding a platform](adding-a-platform.md#6-property-harness).
