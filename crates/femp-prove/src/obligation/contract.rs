//! `#[spec]`: for every argument the spec allows, its target returns a
//! result the spec ensures, without panicking. The task calls the spec
//! with arbitrary arguments; the spec_shape pass turns its `requires` into
//! assumptions and its `ensures` into assertions around the real call.

use femp_spec::{Contract, Domain};
use femp_vir::{Const, Cursor, Expr, FuncId, Function, Program, Terminator, Type};

use super::inputs::assume_domain;
use crate::passes::spec_shape::shape;
use crate::task::{Input, Summary, Task};

pub fn summary(spec: FuncId, target: FuncId, contract: &Contract, trusted: bool) -> Summary {
    Summary {
        spec,
        target,
        name: contract.name.clone(),
        trusted,
    }
}

/// Whether the spec has the shape every spec must have.
pub fn check_shape(
    program: &Program,
    spec: FuncId,
    target: FuncId,
    name: &str,
) -> Result<(), String> {
    shape(program.function(spec), target, name).map(|_| ())
}

pub fn task(
    program: &Program,
    spec: FuncId,
    target: FuncId,
    contract: &Contract,
) -> Result<Task, String> {
    let mut program = program.clone();
    let declared = program.function(spec);
    let span = declared.span.clone();
    let result = declared.result.clone();
    let types: Vec<Type> = declared
        .params
        .iter()
        .map(|param| declared.ty(*param).clone())
        .collect();
    let mut entry = Function::new(format!("prove {}", contract.name), Type::Unit, span);
    let mut inputs = Vec::new();
    let block = entry.entry;
    let mut cursor = Cursor::new(&mut entry, block);
    let mut args = Vec::new();
    for (param, ty) in contract.params.iter().zip(types) {
        let value = cursor.havoc(ty, Some(param.name.clone()));
        let domain = match param.ty {
            Some(prim) => {
                let domain = Domain::resolve(prim, &param.generator)
                    .map_err(|error| format!("parameter `{}`: {error}", param.name))?;
                assume_domain(&mut cursor, value, prim, &domain);
                Some((prim, domain))
            }
            None => None,
        };
        inputs.push(Input {
            name: param.name.clone(),
            value,
            domain,
        });
        args.push(value);
    }
    cursor.call(spec, args, result);
    let unit = cursor.def(Type::Unit, Expr::Const(Const::Unit));
    cursor.terminate(Terminator::Return(unit));
    let entry = program.push(entry);
    Ok(Task {
        inputs,
        proving: Some(summary(spec, target, contract, false)),
        timeout: contract
            .timeout
            .map(|seconds| std::time::Duration::from_secs(seconds as u64)),
        ..Task::new(program, entry)
    })
}
