//! `#[property]`: for every input in the `#[gen]` domains, the function
//! returns `true` without panicking.

use femp_core::property::Property;
use femp_vir::{Check, Const, Cursor, Expr, FuncId, Function, Program, Terminator, Type};

use super::inputs::choose;
use crate::task::Task;

pub const RETURNS_FALSE: &str = "returns false";

pub fn task(program: &Program, func: FuncId, property: &Property) -> Result<Task, String> {
    let mut program = program.clone();
    let span = program.function(func).span.clone();
    let mut entry = Function::new(format!("prove {}", property.name), Type::Unit, span);
    let mut inputs = Vec::new();
    let mut assumed = Vec::new();
    let block = entry.entry;
    let mut cursor = Cursor::new(&mut entry, block);
    let args = choose(&mut cursor, &property.params, &mut inputs, &mut assumed)?;
    let result = cursor.call(func, args, Type::Bool);
    cursor.assert(result, Check::Claim(RETURNS_FALSE.to_string()));
    let unit = cursor.def(Type::Unit, Expr::Const(Const::Unit));
    cursor.terminate(Terminator::Return(unit));
    let entry = program.push(entry);
    Ok(Task {
        inputs,
        assumed,
        ..Task::new(program, entry)
    })
}
