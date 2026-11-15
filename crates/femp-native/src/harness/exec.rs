use std::path::PathBuf;
use std::time::Duration;

use femp_core::Result;
use femp_core::property::{Executor, Outcome, Sequence, Suite, Support, Trace, Value};
use femp_harness::{Session, wire};

/// Runs cases through the harness executable, which stays alive between
/// cases and is restarted after one aborts.
pub struct NativeExecutor {
    session: Session,
    suite: Suite,
}

impl NativeExecutor {
    pub fn new(executable: PathBuf, suite: Suite, timeout: Duration) -> Self {
        Self {
            session: Session::new(executable, Vec::new(), timeout),
            suite,
        }
    }
}

impl Executor for NativeExecutor {
    fn supports_property(&self, _index: usize) -> Support {
        Support::Yes
    }

    fn supports_machine(&self, index: usize) -> Support {
        if self.suite.machines[index].uses_effects() {
            Support::No("its methods use effects a native program does not provide".to_string())
        } else {
            Support::Yes
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
