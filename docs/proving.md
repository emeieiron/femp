# Proving

`femp prove` proves the claims of a FeMP project for **all** inputs, where `femp prop` checks
them for many generated ones. It reads the same declarations: a `#[property]`, `#[machine]`
or `#[differential]` is written once, then tested by `femp prop` and proved by `femp prove`.
`#[spec]` functions add what a function promises, for every input it accepts.

> Test it with generated inputs. Prove it for all of them. Replay every counterexample.

The prover lowers the code with the Fe compiler's own front end, turns each claim into
verification conditions and asks the [Z3](https://github.com/Z3Prover/z3) solver whether they
can fail. A failure comes back as a minimal counterexample, which femp runs on the compiled
targets and keeps as a regression that `femp prop` replays from then on.

```
PROVED      core::vested_never_exceeds_total  on evm, native, wasm
REFUTED     core::halves_stay_small           returns false
            x = 250
            evm returned false · native returned false · wasm returned false
            saved to regressions/core/halves_stay_small.json
```

## What each claim means

| Claim | Proved when |
| --- | --- |
| `#[property]` | For every input in its `#[gen]` domains, it returns `true` without panicking. |
| `#[differential]` | Every target computes the same outcome. Targets that lower to the same code agree by construction; code that differs under `#[cfg(target = ..)]` is compared side by side: both sides return the same value, or both panic. |
| `#[machine]` | By induction: `#[init]` establishes every `#[invariant]`, and each `#[command]`, from any state on which the invariants hold, keeps them. Nothing panics, observations included. |
| `#[spec]` | For every input its `requires` allow, its target returns without panicking and its `ensures` hold. |

A panic is any way the code stops abnormally: checked arithmetic that leaves its type,
division by zero, a signed `MIN / -1`, an index past the end of an array, a failed `assert`,
and `core::panic`.

A proof is per target. `usize` is 256 bits on the EVM, 64 natively and 32 in WebAssembly, and
`#[cfg(target = ..)]` code differs, so each target's program is proved; targets whose programs
are the same are proved once.

### Machines that are not inductive

Induction can fail although no run fails: the invariants may allow a state that `#[init]` never
leads to. femp then searches for a real sequence of commands from `#[init]`, up to `--steps`
long. If it finds one, the machine is **refuted** with that sequence:

```
REFUTED     core::Counter  invariant `few_big_adds` fails after step 3
            new()
            1. add(amount = 10)
            2. add(amount = 10)
            3. add(amount = 10)
```

If it finds none, the machine is **unknown**, with the state the command breaks. Strengthen the
invariants until they describe every state that can occur. The demonstrator's `VestingModel`
needed one that says the schedule stays valid, because `vested_at` cannot overflow on a valid
schedule but can on an arbitrary one.

## Specs

A spec is an ordinary Fe function that states what another function promises. It takes the
target's arguments, `requires` what callers must provide, calls the target once, `ensures`
what the result satisfies, and returns it. The statements come from the `femp_spec` library,
which `femp new` puts in `spec/` and wires into the shared module:

```fe
use femp_spec::{ensures, requires}

#[spec]
fn claim_spec(position: Position, amount: u64, now: u64) -> Result<VestingError, Position> {
    requires(position.claimed <= position.schedule.total)
    let result = position.claim(amount, now)
    match result {
        Result::Ok(updated) => ensures(updated.claimed == position.claimed + amount),
        Result::Err(_) => ensures(true),
    }
    result
}
```

A spec names its target by its own name (`claim_spec` describes `claim`) or with
`target = "Position::claim"`. Options:

| Option | Meaning |
| --- | --- |
| `prove` | The default: the spec is proved, then used in place of its target. |
| `trusted` | The spec is used without proof, and listed under "assumed" in the report. |
| `transparent` | The spec is proved, but calls keep its target's body. |
| `timeout = 30` | Seconds the solver may spend on each of the spec's obligations. |

Where a proved or trusted spec stands in for its target, a call asserts the spec's `requires`,
and then knows only what the spec `ensures` of the result. The same arguments always give the
same result, since Fe code without effects is a function of its arguments. This keeps proofs
of callers small and lets them use code the prover cannot model, but it only proves what the
spec says. A claim that needs more is reported as unknown: *a stronger spec may prove it*.
Make a spec `transparent` when callers rely on more than it states. A claim proved with a spec
that is itself not proved is reported as unknown too.

At run time, `requires`, `ensures` and `invariant` check what they state. A public spec of
primitive parameters and result is also run by `femp prop`: a case its `requires` rejects is
discarded, and a failed `ensures` or any panic fails it. WebAssembly traps alike when a spec
rejects a case and when it fails, so specs run on the EVM and natively only.

## Loops

A loop with `invariant(..)` as the first statement of its body is proved by induction: the
invariant must hold when the loop starts, and every iteration that starts in a state satisfying
it must end in one too. The invariant is checked at the loop's head, before the condition is
tested, and may only read the loop's state.

```fe
use femp_spec::invariant

pub fn count_up(_ n: u8) -> u8 {
    var i: u8 = 0
    while i < n {
        invariant(i <= n)
        i = i + 1
    }
    i
}
```

A loop without an invariant is unrolled for `--unroll` iterations (8 by default). If it can run
longer, the claim is unknown rather than proved, so a proof never quietly covers only short
runs. A `for` loop over an array of up to `--unroll` elements needs no invariant.

## Running

```sh
femp prove                    # every claim on every platform
femp prove -f vest            # claims whose name contains `vest`
femp prove -t evm             # one platform's programs only
femp prove --list             # the claims, as `femp prop --list` shows them, plus every spec
femp prove --dump-vir --keep-smt   # keep what the prover saw under build/prove
```

| Option | Default | Effect |
| --- | --- | --- |
| `-t <platform>` | every platform | Prove the programs of these platforms. |
| `-f <pattern>` | – | Only claims whose name contains the pattern. |
| `--timeout <s>` | 20 | Seconds the solver may spend on one assertion. |
| `--unroll <n>` | 8 | Iterations a loop without an invariant is unrolled for. |
| `--steps <k>` | 6 | Longest sequence a machine search tries. |
| `--jobs <n>` | CPUs | Tasks proved at once, each with its own solver process. |
| `--no-replay` | – | Do not run counterexamples on the compiled targets. |
| `--json` | – | Also write `build/prove/report.json`. |
| `--dump-vir` | – | Write each task after every pass to `build/prove/vir`. |
| `--keep-smt` | – | Write each task's SMT-LIB script to `build/prove/smt`. |
| `--strict` | – | Also fail on claims the prover does not support. |

The run fails when a claim is refuted or unknown, and with `--strict` when one is not
supported. Reports contain no timings, so equal runs give equal reports.

## Verdicts

| Verdict | Meaning |
| --- | --- |
| `PROVED` | The claim holds for every input, on the targets listed. |
| `REFUTED` | A counterexample breaks it. It was minimized toward the simplest inputs, in the order `femp prop` shrinks to, then run on the compiled targets and saved under `regressions/`. |
| `UNKNOWN` | The solver ran out of time, a loop may run longer than it was unrolled, a spec's `requires` may not hold where it stands in, or a machine is not inductive. The reason says which. |
| `UNSUPPORTED` | The claim reaches code the prover has no model for, such as effects, storage, mutable borrows or recursion. The location is reported. |
| `SKIPPED` | A trusted spec, which is assumed rather than proved. |

A counterexample the compiled targets do not reproduce means the proof's model of the code and
the compiled code disagree. femp reports that and fails the run, unless the counterexample only
exists because a spec leaves the result open, in which case the claim is unknown.

The report ends with what the run took on trust: trusted specs, and generators the prover
cannot read as a set of values, which it replaces by the whole type.

## Installing Z3

The installer puts a pinned Z3 build in `~/.femp/z3`. femp looks for Z3 in this order:
`FEMP_Z3`, `FEMP_HOME/z3/bin/z3`, then `z3` on `PATH`; `femp doctor` shows which one it uses.
The Linux builds need glibc 2.38 or newer; on an older system, install `z3` with the system's
package manager.

## Limits

- Pure code only: integers, `bool`, structs, enums, tuples and arrays. Effects, contract
  storage, mutable borrows, raw pointers, strings and recursion are reported as not supported.
- Machines whose methods use effects, such as contract machines, are not supported yet.
- Differentials count any two panics as agreeing, whatever their reasons.
