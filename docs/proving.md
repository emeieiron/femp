# Proving

`femp prove` proves the claims of a FeMP project for **all** inputs, where `femp prop` checks
them for many generated ones. It reads the same declarations: a `#[property]`, `#[machine]`
or `#[differential]` is written once, then tested by `femp prop` and proved by `femp prove`.
`#[spec]` functions add what a function promises, for every input it accepts, and a contract's
`#[invariant]`s what its storage keeps after every message.

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
| Contract `#[invariant]` | By induction over messages: the storage `init` leaves satisfies every invariant, and each `recv` arm, from any storage that satisfies them, leaves storage that does. A transaction that reverts changes nothing. |
| `#[ensures(message = ..)]` | From any storage the contract's invariants allow, every transaction of the message that goes through leaves the storage it describes. |
| `#[reverts(message = ..)]` | From any storage the invariants allow, the message reverts exactly when it holds. |

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

## Contracts

An `#[invariant]` outside a `#[machine]` describes a contract's storage. It is a function of
the contract's fields, which its parameters name, and returns `bool`:

```fe
pub contract Vesting uses (ctx: Ctx, log: mut Log) {
    mut store: VestingStore,
    ...
}

/// The contract never pays out more than the schedule's total.
#[invariant]
fn claims_stay_within_total(store: VestingStore) -> bool {
    store.claimed <= store.total
}
```

When a module declares several contracts, `#[invariant(of = "Name")]`, and `of` on `#[ensures]` and `#[reverts]`, say which one.

A parameter that is not a field ranges over every value of its type, so an invariant can speak
of every entry of a `StorageMap`:

```fe
/// No account ever holds more than the cap.
#[invariant]
fn balances_stay_under_the_cap(store: TokenStore, account: Address) -> bool {
    store.balances.get(key: account) <= CAP
}
```

The contract is proved as a machine whose state is its storage:

- A new contract's storage is zero: `0`, `false` and an enum's first variant. `init` runs on it
  with any arguments.
- Each `recv` arm runs with any message, from any storage the invariants allow.
- Anything that stops a handler, a panic or `revert`, `revert_error`, `revert_bytes` or
  `revert_empty`, undoes its transaction, so it cannot break an invariant.
- The block and transaction context `Ctx` reads (`ctx.timestamp`, `ctx.caller`, `ctx.value`
  and the rest) is any value, chosen per transaction; `ctx.value` is zero for a handler that is
  not `#[payable]`. Reads that change during a transaction, such as `gas` or another account's
  balance, are any value at each read.
- `log.emit` leaves storage alone.
- A `StorageMap` holds a value for every key, zero until it is written; `get`, `set`, `remove`
  and `map[key]` read and write it.
- An invariant with parameters beyond the fields is assumed, before each message, at the value
  it is checked at afterwards and at every value of a parameter's type the message carries,
  `ctx.caller` included: those name the entries a message can touch. It holds for every value of
  them, which a proof establishes, but an induction that needs other entries comes back unknown.
- An `Address` is chosen as the 160-bit number it holds, as are `ctx.caller`, `ctx.origin`,
  `ctx.address` and `ctx.coinbase`.
- A call to another contract (`call`, `try_call`, `call_with_default`, `send_value`, `static`
  and `try_static`, on an `Address` or through `Call`) answers any value, and `call`,
  `call_with_default` and `static` may revert as the callee does. A call that may change state
  lets the callee call back first: any of the contract's messages can run from the storage as
  it stands at the call, so the invariants must hold there, and the storage afterwards is any
  storage the invariants allow. While `init` runs the contract has no code, so nothing calls
  back into it.

### Calls to other contracts

Counting a payment before handing it over keeps the invariant even if the payee calls back,
and is proved. Handing it over first is not:

```fe
Pay { amount } -> bool uses (mut store, mut call) {
    if amount > store.limit - store.paid {
        revert_error(OverLimit { amount })
    }
    let accepted = store.payee.call(PayeeMsg::Receive { amount })
    store.paid = store.paid + amount
    accepted
}
```

```
UNKNOWN     contracts::LateVault  invariant `late_paid_within_limit` fails after `Pay` from (VaultStore { ... }), amount = 1
            `Pay` calls another contract, which may call back first; the search tries callees that do not, and no sequence of up to 6 messages from `init` breaks it
```

A payee that calls `Pay` again while it is being paid is paid twice for one check of the
limit. The search from deployment answers each call with values it chooses but never calls
back, so it cannot show that sequence, and the claim is unknown rather than refuted. A contract
that breaks an invariant before a call and restores it afterwards is reported at the call:

```
UNKNOWN     contracts::EagerVault  `Pay` calls another contract at evm/src/lib.fe:92, which can call back while `eager_paid_within_limit` does not hold, from (...)
```

A sequence that goes through a call is not replayed: on the EVM the callee is an account
without code, which answers otherwise than the search chose.

If induction fails, a bounded search from deployment looks for a sequence of up to `--steps`
messages, in which block time and number never decrease. A sequence it finds refutes the
contract:

```
REFUTED     contracts::Vesting  invariant `claims_stay_within_total` fails after step 1
            init(total = 1, start = 0, cliff = 0, duration = 1)
            1. Claim(ctx.timestamp = 1, amount = 1)
            evm accepts every transaction, and its storage then breaks `claims_stay_within_total`
```

The sequence is then sent to the compiled contract in an in-process EVM: it is deployed with
the `init` arguments, and each message is sent with the selector and the context the search
chose. After each transaction the replay reads the contract's storage from the slots the
compiler laid its fields out in, and the map entries at every value the sequence names, decodes
it into the fields, and evaluates the broken invariant on it. Every transaction of a refuting
sequence goes through in the proof and leaves storage that breaks the invariant; a transaction
the EVM rejects, or storage that satisfies the invariant, means the proof's model and the
contract disagree, which fails the run.

### What one message does

`#[ensures]` and `#[reverts]` state what a single message does. Their parameters are named after
what they read: a field is the storage before the message, `<field>_after` the storage after it,
a message field the argument, and `caller`, `value`, `timestamp`, `block_number`, `origin` or
another context field what the transaction runs with. Any other parameter ranges over every
value of its type:

```fe
/// A transfer that goes through moves `amount` from the caller to `to`,
/// unless the caller holds less, and leaves every other account alone.
#[ensures(message = "Transfer")]
fn transfer_moves_tokens(
    store: TokenStore,
    store_after: TokenStore,
    to: Address,
    amount: u256,
    caller: Address,
    account: Address,
) -> bool { ... }

/// Minting reverts exactly when someone other than the owner asks, or when
/// the recipient or the supply would pass its limit.
#[reverts(message = "Mint")]
fn mint_reverts(store: TokenStore, to: Address, amount: u256, caller: Address) -> bool {
    caller != store.owner
        || amount > CAP - store.balances.get(key: to)
        || amount > u256::max() - store.supply
}
```

Each is proved from any storage the contract's invariants allow, which it assumes, proved or not.
`#[reverts]` is proved in both directions: where it holds, the message reverts; where it does not,
the message goes through, so any panic there fails it. When a check fails, a search looks for a
sequence of up to `--steps` messages from deployment that ends in the message and breaks it, and
replays it on the EVM. Without one the claim is unknown, with the storage the invariants allow
that breaks it: stronger invariants may rule that storage out.

```
REFUTED     contracts::transfer_reverts_when_short  `Transfer` goes through although `transfer_reverts_when_short` holds
            init(ctx.caller = 0)
            1. Transfer(ctx.caller = 0, to = 0, amount = 1)
            evm accepts every transaction, and the claim holds on its storage before step 1
```

The replay evaluates the claim on the storage before the message, and after it for
`#[ensures]`, so the EVM confirms both what the message did and what the claim said about it.

`femp prop` tests the same invariants and message claims with generated messages; see
[property testing](property-testing.md#contracts).

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
femp prove --list             # the claims, as `femp prop --list` shows them, plus every spec and contract
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
| `UNKNOWN` | The solver ran out of time, a loop may run longer than it was unrolled, a spec's `requires` may not hold where it stands in, or a machine or contract is not inductive. The reason says which. |
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

- Values are integers, `bool`, structs, enums, tuples, arrays and storage maps. A mutable
  borrow is lent to the function it is passed to, which hands the value back; a function that
  returns a mutable borrow is not supported. Raw pointers, strings and recursion are reported as
  not supported.
- Effects are modeled where a contract provides them: its storage, which any function it is
  passed to may write, the EVM context, and values bound with `with`.
- A contract's messages, `init` and invariants may range over integers, `bool` and `Address`
  only. Creating contracts, delegate calls, raw calls and raw storage access are not supported
  yet. A call that may change state must be made where the contract's storage can be written:
  in a handler, or in a function that writes the storage. A callee's revert data and returned
  bytes are not modeled, only its typed answer and whether it succeeded. An invariant cannot
  sum a map's entries.
- A replayed sequence cannot choose `ctx.address`, `ctx.origin`, `ctx.selfbalance` or
  `ctx.blobbasefee`, which the EVM decides. The replay reads a map's entries at the values the
  sequence names; entries at keys it computes, such as a hash, read as zero.
- Machines whose methods use effects, such as machines that deploy contracts, are not
  supported yet.
- Differentials count any two panics as agreeing, whatever their reasons.
