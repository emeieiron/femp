use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use femp_core::Result;
use femp_core::property::{Executor, Outcome, Sequence, Suite, Support, Trace, Value};
use femp_harness::{Session, wire};

use super::render::machine_fits;

/// Runs cases through `harness.mjs`, which keeps one module instance alive
/// in Node.js (restarting it automatically after a failed call) and answers
/// on its standard output.
pub struct WasmExecutor {
    session: Session,
    suite: Suite,
}

impl WasmExecutor {
    pub fn new(
        node: &Path,
        runner: &Path,
        module: &Path,
        manifest: &Path,
        entries: &Path,
        suite: Suite,
        timeout: Duration,
    ) -> Self {
        let args: Vec<OsString> = [runner, module, manifest, entries]
            .iter()
            .map(|path| path.as_os_str().to_owned())
            .collect();
        Self {
            session: Session::new(node, args, timeout),
            suite,
        }
    }
}

impl Executor for WasmExecutor {
    fn supports_property(&self, _index: usize) -> Support {
        Support::Yes
    }

    fn supports_machine(&self, index: usize) -> Support {
        if machine_fits(&self.suite.machines[index]) {
            Support::Yes
        } else {
            Support::No("its methods use effects a WebAssembly module does not provide".to_string())
        }
    }

    fn call(&mut self, property: usize, cases: &[Vec<Value>]) -> Result<Vec<Outcome>> {
        let id = self.suite.property_id(property);
        let result = self.suite.properties[property].result;
        cases
            .iter()
            .map(|args| {
                let reply =
                    self.session
                        .exchange(&wire::property_request(id, args), 1, wire::is_status)?;
                Ok(wire::property_outcome(result, reply))
            })
            .collect()
    }

    fn run(&mut self, machine: usize, sequence: &Sequence) -> Result<Trace> {
        let id = self.suite.machine_id(machine);
        let definition = &self.suite.machines[machine];
        let reply = self.session.exchange(
            &wire::sequence_request(id, sequence),
            wire::sequence_reply_len(definition, sequence),
            wire::is_status,
        )?;
        wire::trace(definition, sequence, reply)
    }
}
