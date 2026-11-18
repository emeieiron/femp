// Output of real femp runs, shown on the landing page. Each one was captured from the project
// `femp new my-app` creates, or from the examples named beside it, and is quoted as printed.

export const test = `$ femp test
    Tested core [evm]: 3 passed, 0 failed
    Tested core [native]: 3 passed, 0 failed
    Tested core [wasm]: 3 passed, 0 failed
    Tested contracts [evm]: 1 passed, 0 failed

Shared tests across platforms

                            evm     native  wasm
    core::caps_the_rate     pass    pass    pass
    core::charges_the_rate  pass    pass    pass
    core::rounds_down       pass    pass    pass

    Parity every shared test behaves the same on all platforms it ran on`;

export const prop = `$ femp prop
PASS  core::fee_spec  256 cases on evm, native
      not run on wasm: a WebAssembly module traps alike when a spec rejects a case and when it fails

1 passed, 0 failed (seed 0x000000000000002a, normalized comparison)`;

export const prove = `$ femp prove
PROVED      core::fee_spec  on evm, native, wasm

1 proved, 0 refuted, 0 unknown (evm, native, wasm)`;

export const build = `$ femp build
     Built contracts [evm]
    bytecode         build/evm/contracts/Fees.bin
    runtime-bytecode build/evm/contracts/Fees.runtime.bin
    abi              build/evm/contracts/Fees.abi.json
    metadata         build/evm/contracts/Fees.metadata.json
    binding          build/evm/contracts/Fees.ts
     Built cli [native]
    executable       build/native/cli/my_app_cli
     Built web [wasm]
    wasm-module      build/wasm/web/my_app_web.wasm
    wasm-manifest    build/wasm/web/my_app_web.manifest.json
    binding          build/wasm/web/my_app_web.js
    binding          build/wasm/web/my_app_web.d.ts
    runtime          build/wasm/web/host.js
    runtime          build/wasm/web/host.d.ts
    package-manifest build/wasm/web/package.json`;

/** `femp prove` on the vesting example. */
export const vesting = [
  ['PROVED', 'contracts::Vesting', 'on evm'],
  ['PROVED', 'core::vested_never_exceeds_total', 'on evm, native, wasm'],
  ['PROVED', 'core::vesting_never_decreases', 'on evm, native, wasm'],
  ['PROVED', 'core::vesting_waits_for_the_cliff', 'on evm, native, wasm'],
  ['PROVED', 'core::claims_stay_within_what_vested', 'on evm, native, wasm'],
  ['PROVED', 'core::VestingModel', 'on evm, native, wasm'],
  ['PROVED', 'core::claim_spec', 'on evm, native, wasm'],
] as const;

/** `femp prove` on the planted-bug fixture: a counterexample, run on every compiled target. */
export const refuted = `REFUTED     core::rounding  diverges: evm+native and wasm compute different outcomes
            x = 1000
            evm returned 1000 · native returned 1000 · wasm returned 999
            saved to regressions/core/rounding.json`;
