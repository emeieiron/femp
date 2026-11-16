//! Compile-time constants as instructions: scalars directly, aggregates
//! built from their parts.

use fe_hir::analysis::semantic::{SConst, SemConstId, SemConstScalar, SemConstValue};
use femp_core::property::{Ty, Value};
use femp_vir::{Const, Expr, Span, Type, ValueId};

use super::{BodyLowering, Lowering};

impl<'db> BodyLowering<'_, 'db> {
    pub(super) fn constant(
        &mut self,
        constant: &SConst<'db>,
        ty: &Type,
        span: &Span,
    ) -> Lowering<ValueId> {
        match constant {
            SConst::Value(value) => self.const_value(value.value(), ty, span),
            _ => Err(self.unsupported(span, "a constant the compiler did not evaluate")),
        }
    }

    fn const_value(&mut self, id: SemConstId<'db>, ty: &Type, span: &Span) -> Lowering<ValueId> {
        let db = self.lx.db;
        let expr = match id.value(db) {
            SemConstValue::Unit => Expr::Const(Const::Unit),
            SemConstValue::Scalar { value, .. } => match value {
                SemConstScalar::Bool(value) => Expr::Const(Const::Bool(*value)),
                SemConstScalar::Int { value } => {
                    let Some((bits, signed)) = ty.int_shape() else {
                        return Err(
                            self.unsupported(span, "an integer constant of a non-integer type")
                        );
                    };
                    let prim = if signed {
                        Ty::Int(bits)
                    } else {
                        Ty::Uint(bits)
                    };
                    let parsed = Value::parse(prim, &value.to_string()).ok_or_else(|| {
                        self.unsupported(
                            span,
                            format!("the constant {value} does not fit `{prim}`"),
                        )
                    })?;
                    Expr::Const(Const::Int(parsed))
                }
                SemConstScalar::Bytes(_) => {
                    return Err(self.unsupported(span, "byte string constants are not supported"));
                }
            },
            SemConstValue::Tuple { elems, .. }
            | SemConstValue::Struct { fields: elems, .. }
            | SemConstValue::Array { elems, .. } => {
                let parts = self.part_types(ty, None, elems.len(), span)?;
                let mut fields = Vec::new();
                for (elem, part) in elems.iter().zip(parts) {
                    fields.push(self.const_value(*elem, &part, span)?);
                }
                Expr::Make {
                    ty: ty.clone(),
                    fields,
                }
            }
            SemConstValue::Enum {
                variant, fields, ..
            } => {
                let parts = self.part_types(ty, Some(variant.0), fields.len(), span)?;
                let mut values = Vec::new();
                for (field, part) in fields.iter().zip(parts) {
                    values.push(self.const_value(*field, &part, span)?);
                }
                Expr::MakeEnum {
                    ty: ty.clone(),
                    variant: variant.0,
                    fields: values,
                }
            }
            SemConstValue::Description(_) => {
                return Err(self.unsupported(span, "a constant that describes a type"));
            }
        };
        Ok(self.fresh(ty.clone(), expr, span.clone()))
    }

    fn part_types(
        &self,
        ty: &Type,
        variant: Option<u16>,
        count: usize,
        span: &Span,
    ) -> Lowering<Vec<Type>> {
        if let Type::Array { elem, len } = ty
            && *len == count
        {
            return Ok(vec![(**elem).clone(); count]);
        }
        self.lx
            .types
            .table
            .fields(ty, variant)
            .filter(|parts| parts.len() == count)
            .ok_or_else(|| self.unsupported(span, "a constant whose shape does not match its type"))
    }
}
