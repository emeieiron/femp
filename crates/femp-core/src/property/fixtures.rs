//! A small suite that exercises every shape a harness renders, for the
//! adapters' tests.

use std::path::PathBuf;

use super::{
    Command, Location, Machine, Method, Observation, Param, Property, PropertyKind, Suite, Ty,
};

fn location() -> Location {
    Location {
        file: PathBuf::from("src/lib.fe"),
        line: 1,
    }
}

fn param(label: Option<&str>, name: &str, ty: Ty) -> Param {
    Param {
        name: name.to_string(),
        label: label.map(str::to_string),
        ty,
        generator: "any".to_string(),
    }
}

fn method(name: &str, params: Vec<Param>) -> Method {
    Method {
        name: name.to_string(),
        params,
        effects: Vec::new(),
        location: location(),
    }
}

fn property(
    name: &str,
    kind: PropertyKind,
    path: &[&str],
    params: Vec<Param>,
    result: Ty,
) -> Property {
    Property {
        name: name.to_string(),
        kind,
        path: path.iter().map(|segment| segment.to_string()).collect(),
        params,
        result,
        cases: None,
        location: location(),
    }
}

pub fn suite() -> Suite {
    Suite {
        module: "core".to_string(),
        ingot: "demo_core".to_string(),
        properties: vec![
            property(
                "holds",
                PropertyKind::Holds,
                &["props"],
                vec![
                    param(Some("a"), "a", Ty::Uint(64)),
                    param(None, "b", Ty::Bool),
                ],
                Ty::Bool,
            ),
            property(
                "negate",
                PropertyKind::Agrees,
                &["props"],
                vec![param(Some("x"), "x", Ty::Int(64))],
                Ty::Int(64),
            ),
            property(
                "wide",
                PropertyKind::Agrees,
                &[],
                vec![param(Some("x"), "x", Ty::Uint(256))],
                Ty::Uint(256),
            ),
        ],
        machines: vec![Machine {
            name: "Counter".to_string(),
            path: vec!["model".to_string()],
            init: method("new", vec![param(Some("start"), "start", Ty::Uint(8))]),
            commands: vec![
                Command {
                    method: method("add", vec![param(Some("amount"), "amount", Ty::Uint(64))]),
                    weight: 2,
                },
                Command {
                    method: method("flip", vec![param(None, "sign", Ty::Int(8))]),
                    weight: 1,
                },
            ],
            invariants: vec![method("small", Vec::new())],
            observations: vec![
                Observation {
                    method: method("value", Vec::new()),
                    ty: Ty::Uint(64),
                },
                Observation {
                    method: method("sign", Vec::new()),
                    ty: Ty::Int(8),
                },
            ],
            steps: "1..=8".to_string(),
            runs: None,
            clock: None,
            reuse: false,
            location: location(),
        }],
    }
}
