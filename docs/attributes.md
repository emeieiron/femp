# Attributes

FeMP reads its claims from attributes on ordinary Fe items. The Fe compiler accepts attributes
it does not know and ignores them, so annotated code builds with `fe` unchanged. A malformed
declaration is reported with its file and line before anything runs.

| Attribute | On | Read by |
| --- | --- | --- |
| [`#[property]`](#property) | function | `femp prop`, `femp prove` |
| [`#[differential]`](#differential) | function | `femp prop`, `femp prove` |
| [`#[gen]`](#gen) | entry point, `#[init]`, `#[command]` | `femp prop`, `femp prove` |
| [`#[machine]`](#machine) | struct | `femp prop`, `femp prove` |
| [`#[init]`, `#[command]`, `#[invariant]`, `#[observe]`](#machine-methods) | methods of a machine | `femp prop`, `femp prove` |
| [`#[spec]`](#spec) | function | `femp prove`, `femp prop` |
| [`#[invariant]`](#contract-invariant) | free function in a contract's module | `femp prop`, `femp prove` |
| [`#[ensures]`, `#[reverts]`](#ensures-and-reverts) | free function in a contract's module | `femp prop`, `femp prove` |

## Rules for entry points

`#[property]`, `#[differential]` and machine methods follow the same rules:

| | |
| --- | --- |
| Visibility | `pub`. |
| Shape | Not generic, not under `#[cfg]`, and no `self` outside machine methods. |
| Effects | None in a shared module. In an `evm` module, a machine may use `Create`, `Call`, `Ctx` and `Log`. |
| Types | Parameters and results are `bool`, `u8`…`u256` or `i8`…`i256`. `usize` and `isize` are rejected, because their width differs per platform. |
| Location | Any file of a module. In a shared module it runs on every platform; in a platform module, on that platform. |

## `#[property]`

```fe
#[property(cases = 500)]
pub fn vested_never_exceeds_total(total: u64, start: u64, cliff: u64, duration: u64, now: u64) -> bool
```

Returns `bool`. Passes when every target returns `true` for every input, and agrees.

| Option | Default | Meaning |
| --- | --- | --- |
| `cases = N` | 256 | Generated cases per run. `--cases` overrides it. |

## `#[differential]`

```fe
#[differential]
pub fn schedule_error(total: u64, start: u64, cliff: u64, duration: u64) -> u32
```

Returns `bool` or a fixed-width integer. Passes when every target computes the same outcome.
Takes `cases` like `#[property]`. See [differential testing](differential-testing.md).

## `#[gen]`

```fe
#[gen(cliff = "0..=10_000", duration = "1..=10_000")]
```

Chooses how named parameters are generated, as `parameter = "spec"` pairs. A parameter
without one uses `any`. Several `#[gen]` attributes on one function combine.

| Spec | Values |
| --- | --- |
| `any` | Every value of the type. |
| `lo..=hi`, `lo..hi` | A range. Bounds are decimal or `0x` hex, may contain `_`, and may be `min` or `max`. |
| `one_of(a, b, ...)` | One of the listed values; earlier ones count as simpler. |
| `const(a)` | Always `a`. |
| `name(args)` | A generator registered in Rust with `femp_prop::generators::Registry`. |

`femp prove` reads a generator as the set of values it produces, so a property is proved over
its `#[gen]` domains. A generator it cannot read that way is replaced by the whole type and
listed in the report.

## `#[machine]`

```fe
#[machine(steps = "1..=24", runs = 100)]
pub struct VestingModel { pub position: Position, pub now: u64, pub last_error: u32 }

impl Copy for VestingModel {}
```

A state machine: a public struct that implements `Copy`, whose methods are marked below.

| Option | Default | Meaning |
| --- | --- | --- |
| `steps = "lo..=hi"` | `"1..=16"` | Commands per run. |
| `runs = N` | 100 | Runs per invocation. `--cases` overrides it. |
| `clock = "lo..=hi"` | – | In an `evm` module: seconds the block timestamp advances before each step. Delays shrink toward zero. |

### Machine methods

| Attribute | Signature | Meaning |
| --- | --- | --- |
| `#[init]` | no `self`, returns the machine | Builds the first state. Exactly one. |
| `#[command]` | `self`, returns the machine | One step. At least one. `weight = N` (default 1) makes it N times as likely. |
| `#[invariant]` | only `self`, returns `bool` | Must hold after `#[init]` and after every step. |
| `#[observe]` | only `self`, returns `bool` or a fixed-width integer | Recorded after every step; targets must agree on it. |

`#[init]` and `#[command]` take `#[gen]` for their parameters. A method has at most one of
these attributes.

## `#[spec]`

```fe
#[spec]
pub fn fee_spec(amount: u64, rate: u64) -> u64 {
    let charged = fee(amount, rate)
    ensures(charged <= amount)
    charged
}
```

States what another function promises. It takes the target's arguments, states what it
`requires` of them, calls the target, states what it `ensures` of the result, and returns it.
The target is the function named by the spec's name without `_spec`, or `target = "..."`.

| Option | Meaning |
| --- | --- |
| `prove` | The default: proved, then used in place of its target where it is called. |
| `trusted` | Used without proof and listed as assumed in the report. Not with `prove`. |
| `transparent` | Proved, but callers keep the target's body. |
| `target = "Position::claim"` | The function the spec describes. |
| `timeout = N` | Seconds the solver may spend on each of the spec's obligations. |

A spec cannot be generic, take `self`, be under `#[cfg]`, or declare effects in a shared
module. A public spec whose parameters and result are primitives is also an entry point of
`femp prop`. See [specs](proving.md#specs).

### Statements

The `femp_spec` library, which `femp new` puts in `spec/`, provides the statements:

| Function | At run time | To the prover |
| --- | --- | --- |
| `requires(condition)` | Rejects the case | Assumed of the target's arguments |
| `ensures(condition)` | Fails unless it holds | Proved of the result |
| `invariant(condition)` | Fails unless it holds | First in a loop body: the loop's invariant |
| `implies(premise, conclusion)` | `!premise \|\| conclusion` | The same |

## Contract `#[invariant]`

```fe
#[invariant]
fn claims_stay_within_total(store: VestingStore) -> bool {
    store.claimed <= store.total
}
```

A free function in a contract's module, without `self`, type parameters or effects, that
returns `bool`. Parameters named after the contract's fields receive them; any other
parameter ranges over every value of its type. Must hold after `init` and after every
message.

| Option | Meaning |
| --- | --- |
| `of = "Name"` | The contract it describes, when the module declares several. |

## `#[ensures]` and `#[reverts]`

```fe
#[ensures(message = "Claim")]
fn claims_add_their_amount(store: VestingStore, store_after: VestingStore, amount: u64) -> bool

#[reverts(message = "Mint")]
fn mint_reverts(store: TokenStore, to: Address, amount: u256, caller: Address) -> bool
```

Claims about one message, as free functions without `self`, type parameters or effects that
return `bool`. `#[ensures]` must hold after every transaction of the message that goes
through; `#[reverts]` must hold exactly when the message reverts.

| Parameter | Receives |
| --- | --- |
| A field's name | The storage before the message |
| `<field>_after` | The storage after it (`#[ensures]` only) |
| A message field's name | The message's argument |
| `caller`, `value`, `timestamp`, `block_number`, `origin`, … | The transaction's context, as `u256`, or `Address` for an address |
| Anything else | Every value of its type |

| Option | Meaning |
| --- | --- |
| `message = "Name"` | Required: the message variant the claim describes. |
| `of = "Name"` | The contract, when the module declares several. |

See [contracts](contracts.md).

## Fe attributes FeMP uses

These belong to the Fe compiler; FeMP gives them meaning across platforms.

| Attribute | Meaning |
| --- | --- |
| `#[test]` | A test. In a shared module, `femp test` runs it on every platform and compares the results. |
| `#[test(should_revert)]` | A test that passes when it reverts. |
| `#[cfg(target = "evm")]` | Compiled for one platform only. A shared test under it shows as `-` on the others. |
| `#[selector = sol("name(types)")]` | A message variant's selector, which also gives its name and types in the ABI and the TypeScript bindings. |
| `#[payable]` | A handler that accepts value; its ABI entry is `payable`. |
| `#[event]`, `#[error]` | An event or custom error, typed in the TypeScript bindings. |
