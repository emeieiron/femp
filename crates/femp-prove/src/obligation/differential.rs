//! `#[differential]`: every target computes the same outcome. Targets that
//! lower to the same program agree by construction; two that differ are
//! run side by side on the same inputs, each with its panics captured, and
//! must both return the same value or both panic.

use femp_core::property::Property;
use femp_vir::{
    BinOp, Check, Const, Cursor, Expr, FuncId, Function, Program, Proj, Terminator, Type,
};

use super::inputs::choose;
use crate::passes::{self, capture};
use crate::task::{NotSupported, Task};

/// One target's function, reduced to straight-line code with its panics
/// captured, alone in its program.
#[derive(Debug, Clone)]
pub struct Side {
    pub program: Program,
}

pub fn side(program: &Program, func: FuncId) -> Result<Side, NotSupported> {
    let mut task = Task::new(program.clone(), func);
    passes::run(&mut task, &passes::reduction(), &mut |_, _| {})?;
    let mut function = task.entry().clone();
    capture::capture(&mut function);
    Ok(Side {
        program: Program {
            target: task.program.target.clone(),
            types: task.program.types.clone(),
            functions: vec![function],
        },
    })
}

pub fn task(first: &Side, second: &Side, property: &Property, claim: &str) -> Result<Task, String> {
    let mut types = first.program.types.clone();
    let offset = types.append(&second.program.types);
    let left = first.program.functions[0].clone();
    let mut right = second.program.functions[0].clone();
    right.shift_types(offset);
    let left_ty = left.result.clone();
    let right_ty = right.result.clone();
    let span = left.span.clone();
    let mut program = Program {
        target: format!("{}+{}", first.program.target, second.program.target),
        types,
        functions: vec![left, right],
    };

    let mut entry = Function::new(format!("compare {}", property.name), Type::Unit, span);
    let mut inputs = Vec::new();
    let mut assumed = Vec::new();
    let block = entry.entry;
    let mut cursor = Cursor::new(&mut entry, block);
    let args = choose(&mut cursor, &property.params, &mut inputs, &mut assumed)?;
    let outcome = |cursor: &mut Cursor<'_>, func: u32, ty: Type| {
        let value_ty = match &ty {
            Type::Tuple(items) => items[1].clone(),
            _ => Type::Unit,
        };
        let outcome = cursor.call(FuncId(func), args.clone(), ty);
        let panicked = cursor.def(
            Type::Bool,
            Expr::Project {
                value: outcome,
                path: vec![Proj::Field(0)],
            },
        );
        let value = cursor.def(
            value_ty,
            Expr::Project {
                value: outcome,
                path: vec![Proj::Field(1)],
            },
        );
        (panicked, value)
    };
    let (left_panicked, left_value) = outcome(&mut cursor, 0, left_ty);
    let (right_panicked, right_value) = outcome(&mut cursor, 1, right_ty);
    let both_panic = cursor.binary(BinOp::And, left_panicked, right_panicked);
    let left_returned = cursor.not(left_panicked);
    let right_returned = cursor.not(right_panicked);
    let both_return = cursor.binary(BinOp::And, left_returned, right_returned);
    let same = cursor.binary(BinOp::Eq, left_value, right_value);
    let returned_alike = cursor.binary(BinOp::And, both_return, same);
    let agree = cursor.binary(BinOp::Or, both_panic, returned_alike);
    cursor.assert(agree, Check::Claim(claim.to_string()));
    let unit = cursor.def(Type::Unit, Expr::Const(Const::Unit));
    cursor.terminate(Terminator::Return(unit));
    let entry = program.push(entry);
    Ok(Task {
        inputs,
        assumed,
        ..Task::new(program, entry)
    })
}
