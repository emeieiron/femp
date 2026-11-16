use std::env;

use femp_core::environment::Host;

use crate::node;
use crate::process::try_stdout;
use crate::solver;

/// What this machine provides to run compiled code and to prove claims.
pub fn host() -> Host {
    let node = node::found();
    let z3 = solver::found();
    Host {
        os: env::consts::OS.to_string(),
        arch: env::consts::ARCH.to_string(),
        node: node.map(|node| node.path.clone()),
        node_major: node.map(|node| node.major),
        has_c_compiler: try_stdout("cc", &["--version"]).is_some(),
        z3: z3.map(|z3| z3.path.clone()),
        z3_version: z3.map(|z3| z3.version.clone()),
    }
}
