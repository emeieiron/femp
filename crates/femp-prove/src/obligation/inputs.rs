use femp_core::property::{Param, Ty};
use femp_spec::Domain;
use femp_vir::{BinOp, Cursor, Type, ValueId};

use crate::task::Input;

pub fn vir_type(ty: Ty) -> Type {
    match ty {
        Ty::Bool => Type::Bool,
        Ty::Uint(bits) => Type::uint(bits),
        Ty::Int(bits) => Type::int(bits),
    }
}

/// Chooses an arbitrary value for each parameter and assumes it lies in the
/// parameter's domain. A custom generator cannot be read as a set, so the
/// whole type stands for it, which the task records as an assumption.
pub fn choose(
    cursor: &mut Cursor<'_>,
    params: &[Param],
    inputs: &mut Vec<Input>,
    assumed: &mut Vec<String>,
) -> Result<Vec<ValueId>, String> {
    let mut values = Vec::new();
    for param in params {
        let domain = Domain::resolve(param.ty, &param.generator)
            .map_err(|error| format!("parameter `{}`: {error}", param.name))?;
        let value = cursor.havoc(vir_type(param.ty), Some(param.name.clone()));
        assume_domain(cursor, value, param.ty, &domain);
        if let Domain::Custom { name, .. } = &domain {
            assumed.push(format!(
                "`{}` takes any `{}` in place of the generator `{name}`",
                param.name, param.ty
            ));
        }
        inputs.push(Input {
            name: param.name.clone(),
            value,
            domain: Some((param.ty, domain)),
        });
        values.push(value);
    }
    Ok(values)
}

pub fn assume_domain(cursor: &mut Cursor<'_>, value: ValueId, ty: Ty, domain: &Domain) {
    let vir = vir_type(ty);
    let cond = match domain {
        Domain::Any(_) | Domain::Custom { .. } => return,
        Domain::Range { lo, hi } => cursor.within(value, ty, *lo, *hi),
        Domain::Const(constant) => {
            let constant = constant_value(cursor, &vir, *constant);
            cursor.binary(BinOp::Eq, value, constant)
        }
        Domain::OneOf(choices) => {
            let tests: Vec<ValueId> = choices
                .iter()
                .map(|choice| {
                    let constant = constant_value(cursor, &vir, *choice);
                    cursor.binary(BinOp::Eq, value, constant)
                })
                .collect();
            cursor.any(&tests)
        }
    };
    cursor.assume(cond);
}

fn constant_value(
    cursor: &mut Cursor<'_>,
    ty: &Type,
    value: femp_core::property::Value,
) -> ValueId {
    match value.as_bool() {
        Some(flag) => cursor.boolean(flag),
        None => cursor.int(ty, value),
    }
}
