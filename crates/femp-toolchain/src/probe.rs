use std::collections::BTreeSet;
use std::env;
use std::path::Path;
use std::process::Command;

use femp_core::Result;
use femp_core::environment::{Environment, Host, Toolchain};

use crate::process::{capture, try_stdout};
use crate::resolve::{ensure_version, locate};

pub fn environment(pinned_version: &str) -> Result<Environment> {
    let fe = locate(pinned_version)?;
    let version = ensure_version(&fe, pinned_version)?;
    let backends = backends(&fe)?;
    Ok(Environment {
        toolchain: Toolchain {
            fe,
            version,
            backends,
        },
        host: host(),
    })
}

pub fn host() -> Host {
    Host {
        os: env::consts::OS.to_string(),
        arch: env::consts::ARCH.to_string(),
        node_major: try_stdout("node", &["--version"])
            .as_deref()
            .and_then(parse_node_major),
        has_c_compiler: try_stdout("cc", &["--version"]).is_some(),
    }
}

fn backends(fe: &Path) -> Result<BTreeSet<String>> {
    let help = capture(Command::new(fe).args(["build", "-h"]))?;
    Ok(parse_backends(&help.stdout))
}

// Backends are compiled into `fe` behind Cargo features, and its help text
// lists exactly the ones present, so the help is the source of truth for
// what a given binary supports.
fn parse_backends(help: &str) -> BTreeSet<String> {
    const MARKER: &str = "possible values:";
    help.split_once("--backend")
        .and_then(|(_, after_flag)| after_flag.split_once(MARKER))
        .and_then(|(_, values)| values.split_once(']'))
        .map(|(values, _)| {
            values
                .split(',')
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn parse_node_major(version: &str) -> Option<u32> {
    version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELP: &str = "\
      --contract <CONTRACT>   Build a specific contract by name
      --backend <BACKEND>     Backend to use for compilation. Defaults to the one for the `target` in `fe.toml`, or `sonatina` [possible values: sonatina, native, wasm]
  -O, --optimize <LEVEL>      Optimization level. [possible values: 0, 1, 2, s]
";

    #[test]
    fn reads_backends_from_help() {
        let backends: Vec<String> = parse_backends(HELP).into_iter().collect();
        assert_eq!(backends, ["native", "sonatina", "wasm"]);
    }

    #[test]
    fn ignores_possible_values_of_earlier_flags() {
        let help = "--color <COLOR> [possible values: auto, always]\n--backend <B> [possible values: sonatina]\n";
        let backends: Vec<String> = parse_backends(help).into_iter().collect();
        assert_eq!(backends, ["sonatina"]);
    }

    #[test]
    fn tolerates_unexpected_help() {
        assert!(parse_backends("no such flag").is_empty());
    }

    #[test]
    fn reads_node_major_version() {
        assert_eq!(parse_node_major("v24.16.0\n"), Some(24));
        assert_eq!(parse_node_major("garbage"), None);
    }
}
