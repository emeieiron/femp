# Contracts

A contract lives in an `evm` module and keeps its logic in shared code, where it is tested on
every platform. FeMP then checks the contract itself in four ways:

| | Checks | Command |
| --- | --- | --- |
| Unit tests | Calls you write, against the deployed contract | `femp test` |
| Storage invariants | What storage keeps after every sequence of messages | `femp prop`, `femp prove` |
| Message claims | What one message does, and when it reverts | `femp prop`, `femp prove` |
| Contract machines | A model you write, run against the deployed contract | `femp prop` |

The examples come from a vesting contract, which releases a total linearly over a schedule and
lets its owner claim what has vested.

## The contract

The contract keeps the schedule and what was claimed in storage. Its handlers delegate to the
shared `Position`, binding the shared `Clock` port to the block timestamp:

```fe
struct VestingStore { total: u64, start: u64, cliff: u64, duration: u64, claimed: u64 }

pub contract Vesting uses (ctx: Ctx, log: mut Log) {
    mut store: VestingStore,

    init(total: u64, start: u64, cliff: u64, duration: u64) uses (mut store) {
        match Schedule::new(total, start, cliff, duration) {
            Result::Ok(schedule) => { /* store the schedule */ },
            Result::Err(error) => revert_error(VestingFailed { code: error.code() }),
        }
    }

    recv VestingMsg {
        Claim { amount } -> u64 uses (mut store, ctx, mut log) {
            let current = position(
                total: store.total,
                start: store.start,
                cliff: store.cliff,
                duration: store.duration,
                claimed: store.claimed,
            )
            let outcome = with (Clock = block_clock()) {
                vesting_core::claim_now(current, amount)
            }
            match outcome {
                Result::Ok(updated) => {
                    store.claimed = updated.claimed
                    log.emit(TokensClaimed { amount, total_claimed: updated.claimed })
                    updated.claimed
                },
                Result::Err(error) => revert_error(VestingFailed { code: error.code() }),
            }
        }
        // VestedAt, ClaimableAt, Claimable, Claimed
    }
}
```

## Unit tests

Tests in the `evm` module deploy the contract through the `Evm` effect and call it. A test
marked `should_revert` passes when the call reverts:

```fe
#[test(should_revert)]
fn claiming_more_than_vested_reverts() uses (evm: mut Evm) {
    let vesting = evm.create2<Vesting>(value: Wei::zero(), args: (1000, 0, 0, 1), salt: 2)
    let _: u64 = evm.call(
        addr: vesting,
        gas: 200000,
        value: Wei::zero(),
        message: VestingMsg::Claim { amount: 1001 },
    )
}
```

## Storage invariants

An `#[invariant]` outside a machine is a claim about the contract's storage. Its parameters
name the contract's fields, and it returns `bool`:

```fe
/// The contract never pays out more than the schedule's total.
#[invariant]
fn claims_stay_within_total(store: VestingStore) -> bool {
    store.claimed <= store.total
}

/// Deployment accepts only a valid schedule, and no message changes it.
#[invariant]
fn schedule_stays_valid(store: VestingStore) -> bool {
    store.total > 0
        && store.duration > 0
        && store.cliff <= store.duration
        && (store.start as u256) + (store.duration as u256) <= (u64::max() as u256)
}
```

`femp prop` deploys the compiled contract with generated `init` arguments, sends it generated
messages, each its own transaction with block time moving forward, and after every
transaction reads the storage and checks every invariant. `femp prove` proves them by
induction over messages: `init` establishes them, and every handler, from any storage that
satisfies them, keeps them.

```text
PASS  contracts::Vesting                    100 runs on evm
PROVED      contracts::Vesting                    on evm
```

### When a handler is wrong

Suppose `Claim` checks the amount against what has vested, but forgets what was already
claimed:

```fe
Claim { amount } -> u64 uses (mut store, ctx, mut log) {
    // ...
    if amount == 0 || amount > schedule.vested_at(now) {
        revert_error(VestingFailed { code: 3 })
    }
    store.claimed = store.claimed + amount
    // ...
}
```

```bash
femp prop -f Vesting --seed 1
```

```text
FAIL  contracts::Vesting                 fails after 53 runs
      init(total = 1, start = 0, cliff = 0, duration = 1)
      1. Claim(amount = 1)  (+1s)
      2. Claim(amount = 1)
      evm  invariant `claims_stay_within_total` fails after step 2
      seed 0x0000000000000001, case 52, minimized in 534 runs
      saved to regressions/contracts/Vesting.json
      reproduce: femp prop -f Vesting --seed 0x0000000000000001
```

The sequence is shrunk to the simplest one that still fails: a schedule of one token, a claim
once it has vested, and the same claim again. It is saved and replayed by every later run.

## Message claims

`#[ensures]` and `#[reverts]` state what a single message does. Parameters are named after
what they read: a field is the storage before the message, `<field>_after` the storage after
it, a message field its argument, and `timestamp`, `caller`, `value` and the other context
fields what the transaction runs with.

```fe
/// A claim that goes through adds its amount to what was claimed and leaves
/// the schedule as it was.
#[ensures(message = "Claim")]
fn claims_add_their_amount(store: VestingStore, store_after: VestingStore, amount: u64) -> bool {
    store_after.claimed == store.claimed + amount
        && store_after.total == store.total
        && store_after.start == store.start
        && store_after.cliff == store.cliff
        && store_after.duration == store.duration
}

/// A claim reverts exactly when it asks for nothing, or for more than has
/// vested and not yet been claimed.
#[reverts(message = "Claim")]
fn claims_revert_beyond_what_vested(store: VestingStore, amount: u64, timestamp: u256) -> bool {
    let current = position(
        total: store.total,
        start: store.start,
        cliff: store.cliff,
        duration: store.duration,
        claimed: store.claimed,
    )
    amount == 0 || amount > current.claimable_at(timestamp.downcast_truncate())
}
```

`#[reverts]` is checked in both directions: where it holds the message must revert, and where
it does not the message must go through. Both claims are assumed only from storage the
contract's invariants allow.

```text
PASS  contracts::claims_add_their_amount           100 runs on evm
PASS  contracts::claims_revert_beyond_what_vested  100 runs on evm

PROVED      contracts::claims_add_their_amount           on evm
PROVED      contracts::claims_revert_beyond_what_vested  on evm
```

## Contract machines

A [`#[machine]`](property-testing.md#machines) in an `evm` module may use the `Create`, `Call`,
`Ctx` and `Log` effects, so it can deploy the project's contracts and drive them while keeping
a model of what they should hold. This one checks the deployed contract against the shared
`Position` logic, claim by claim:

```fe
#[machine(steps = "1..=16", runs = 50, clock = "0..=150")]
pub struct VestingContractMachine {
    pub vesting: Address,
    pub expected: Position,
    pub rejected: u64,
    pub disagreements: u64,
}

impl VestingContractMachine {
    #[init]
    #[gen(total = "1..=1_000_000", start = "0..=500", cliff = "0..=500", duration = "1..=1_000")]
    pub fn deploy(total: u64, start: u64, cliff: u64, duration: u64) -> VestingContractMachine
    uses (create: mut Create) { ... }

    /// Sends the claim to the contract and applies it to the model.
    #[command]
    #[gen(amount = "0..=1_000_000")]
    pub fn claim(self, amount: u64) -> VestingContractMachine uses (call: mut Call, ctx: Ctx) {
        let now: u64 = ctx.timestamp().as_seconds().downcast_truncate()
        let accepted = self.vesting.try_call(VestingMsg::Claim { amount }).success()
        ...
    }

    #[invariant]
    pub fn accepts_the_same_claims(self) -> bool {
        self.disagreements == 0
    }

    #[invariant]
    pub fn stores_the_expected_total(self) -> bool uses (call: Call) {
        let claimed: u64 = self.vesting.static(VestingMsg::Claimed {})
        claimed == self.expected.claimed
    }
}
```

```text
PASS  contracts::VestingContractMachine     50 runs on evm
```

Each command is its own transaction, `clock` advances block time before each step, and a
command whose transaction reverts with a custom error is recorded as rejected. Machines like
this are run by `femp prop`; `femp prove` lists them as not supported.

## What the checks assume

- **Context.** `ctx.timestamp`, `ctx.caller`, `ctx.value` and the other context fields are
  any value per transaction. `ctx.value` is zero unless the handler is `#[payable]`. Block
  time and number never decrease along a sequence.
- **Reverts.** A transaction that reverts or panics changes nothing, so it cannot break an
  invariant.
- **Maps.** A `StorageMap` holds a value for every key. An invariant may take a parameter that
  is not a field, such as an `account: Address`, and then holds for every value of it.
- **Calls to other contracts** answer any value, and a call that may change state lets the
  callee call back first. Updating storage before handing over a payment is proved; handing
  it over first is reported.
- **Replay.** A sequence that refutes a contract is sent to the compiled contract in an
  in-process EVM, its storage is read back from the slots the compiler assigned, and the
  claim is evaluated on it. A model that disagrees with the compiled contract fails the run.

[Proving contracts](proving.md#contracts) covers the model in full, and
[property testing](property-testing.md#contracts) how sequences are generated.

## Using the contract

`femp build` writes the contract's bytecode, ABI and metadata, as `fe build` does, and a
TypeScript module for viem, ethers or any other library. See
[build outputs and bindings](bindings.md#evm-contracts).
