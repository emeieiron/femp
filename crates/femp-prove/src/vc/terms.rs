//! The SMT term of each VIR expression.

use femp_vir::{
    Arith, BinOp, Const, Expr, Function, Index, Intrinsic, Proj, Type, TypeDef, UnOp, ValueId,
};

use super::sorts::Sorts;
use super::{bits, ints};
use crate::task::NotSupported;

/// What defining a value asserts: that it equals a term, that each of
/// several facts holds (arrays are built cell by cell), or only that it is
/// well formed.
pub enum Definition {
    Equals(String),
    Facts(Vec<String>),
    Arbitrary,
    /// Equals the term, which may be any well-formed value.
    Unknown(String),
}

pub struct Terms<'a> {
    pub sorts: &'a Sorts<'a>,
    pub function: &'a Function,
    /// The uninterpreted function of each name `Expr::Apply` uses.
    pub applied: &'a std::collections::BTreeMap<String, String>,
    /// The integer constants of the function, by the value they define.
    pub constants: std::collections::BTreeMap<ValueId, femp_core::property::Value>,
    /// The values shifted left by a known amount, whose low bits are zero.
    pub shifted: std::collections::BTreeMap<ValueId, u16>,
    /// Whether round trips through bit vectors give way to their bounds.
    pub approximate: bool,
}

/// The values of `function` shifted left by a known amount below their
/// width, with that amount.
pub fn shifted(
    function: &Function,
    constants: &std::collections::BTreeMap<ValueId, femp_core::property::Value>,
) -> std::collections::BTreeMap<ValueId, u16> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.stmts)
        .filter_map(|stmt| match &stmt.kind {
            femp_vir::StmtKind::Def {
                dst,
                expr:
                    Expr::Binary {
                        op: BinOp::Shl,
                        lhs,
                        rhs,
                    },
            } => {
                let (bits, _) = function.ty(*lhs).int_shape()?;
                let amount = constants.get(rhs)?.bits();
                (amount < femp_core::property::U256::from(bits)).then(|| (*dst, amount.to::<u16>()))
            }
            _ => None,
        })
        .collect()
}

/// The values of `function` that integer constants define.
pub fn constants(
    function: &Function,
) -> std::collections::BTreeMap<ValueId, femp_core::property::Value> {
    function
        .blocks
        .iter()
        .flat_map(|block| &block.stmts)
        .filter_map(|stmt| match &stmt.kind {
            femp_vir::StmtKind::Def {
                dst,
                expr: Expr::Const(Const::Int(value)),
            } => Some((*dst, *value)),
            _ => None,
        })
        .collect()
}

pub fn name(value: ValueId) -> String {
    format!("v{}", value.0)
}

fn internal(message: &str) -> NotSupported {
    NotSupported::new(
        femp_vir::Span::default(),
        format!("internal: {message} reached the encoder"),
    )
}

impl Terms<'_> {
    fn ty(&self, value: ValueId) -> &Type {
        self.function.ty(value)
    }

    pub fn define(&self, dst: ValueId, expr: &Expr) -> Result<Definition, NotSupported> {
        if let Expr::Binary { op, lhs, rhs } = expr
            && let Some(facts) = self.bounded(dst, *op, *lhs, *rhs)?
        {
            return Ok(Definition::Facts(facts));
        }
        let term = match expr {
            Expr::Use(value) => name(*value),
            Expr::Const(constant) => match constant {
                Const::Bool(flag) => flag.to_string(),
                Const::Int(value) => ints::value(value),
                Const::Unit => "Unit.mk".to_string(),
            },
            Expr::Unary { op, value } => self.unary(*op, *value)?,
            Expr::Binary { op, lhs, rhs } => self.binary(*op, *lhs, *rhs)?,
            Expr::Cast { value, to } => match (self.ty(*value), to) {
                (Type::Bool, Type::Int { .. }) => format!("(ite {} 1 0)", name(*value)),
                (Type::Int { .. }, Type::Int { .. }) => name(*value),
                (from, to) if from == to => name(*value),
                _ => {
                    return Err(NotSupported::new(
                        femp_vir::Span::default(),
                        "a conversion between these types is not supported",
                    ));
                }
            },
            Expr::Make { ty, fields } => match ty {
                Type::Array { .. } => {
                    return Ok(Definition::Facts(
                        fields
                            .iter()
                            .enumerate()
                            .map(|(index, field)| {
                                format!("(= (select {} {index}) {})", name(dst), name(*field))
                            })
                            .collect(),
                    ));
                }
                _ => self.construct(ty, None, fields)?,
            },
            Expr::MakeEnum {
                ty,
                variant,
                fields,
            } => self.construct(ty, Some(*variant), fields)?,
            Expr::Project { value, path } => {
                self.project(self.ty(*value).clone(), name(*value), path)?
            }
            Expr::Update { value, path, new } => {
                self.update(self.ty(*value).clone(), name(*value), path, &name(*new))?
            }
            Expr::IsVariant { value, variant } => {
                format!(
                    "({} {})",
                    self.sorts.tester(self.ty(*value), *variant),
                    name(*value)
                )
            }
            Expr::Tag { value } => self.tag(*value)?,
            Expr::Repeat { value, len } => {
                return Ok(Definition::Facts(vec![format!(
                    "(forall ((i Int)) (=> (and (<= 0 i) (< i {len})) (= (select {} i) {})))",
                    name(dst),
                    name(*value)
                )]));
            }
            Expr::Fill { value } => format!(
                "((as const {}) {})",
                self.sorts.sort(self.ty(dst)),
                name(*value)
            ),
            Expr::Intrinsic { op, args } => self.intrinsic(dst, *op, args)?,
            Expr::Havoc => return Ok(Definition::Arbitrary),
            Expr::Apply {
                name: applied,
                args,
            } => {
                let function = self
                    .applied
                    .get(applied)
                    .ok_or_else(|| internal("an undeclared function"))?;
                let term = if args.is_empty() {
                    function.clone()
                } else {
                    let args: Vec<String> = args.iter().map(|arg| name(*arg)).collect();
                    format!("({function} {})", args.join(" "))
                };
                return Ok(Definition::Unknown(term));
            }
            Expr::Load { .. } => return Err(internal("a slot load")),
            Expr::Call { .. } => return Err(internal("a call")),
        };
        Ok(Definition::Equals(term))
    }

    fn construct(
        &self,
        ty: &Type,
        variant: Option<u16>,
        fields: &[ValueId],
    ) -> Result<String, NotSupported> {
        let constructor = self.sorts.constructor(ty, variant);
        Ok(if fields.is_empty() {
            constructor
        } else {
            let fields: Vec<String> = fields.iter().map(|field| name(*field)).collect();
            format!("({constructor} {})", fields.join(" "))
        })
    }

    fn field_types(&self, ty: &Type, variant: Option<u16>) -> Result<Vec<Type>, NotSupported> {
        self.sorts
            .table
            .fields(ty, variant)
            .ok_or_else(|| internal("a projection of a value without fields"))
    }

    fn index(index: &Index) -> String {
        match index {
            Index::Const(index) => index.to_string(),
            Index::Value(value) => name(*value),
        }
    }

    fn project(
        &self,
        mut ty: Type,
        mut term: String,
        path: &[Proj],
    ) -> Result<String, NotSupported> {
        for projection in path {
            let (next, selected) = match projection {
                Proj::Field(field) => {
                    let fields = self.field_types(&ty, None)?;
                    let selector = self.sorts.selector(&ty, None, *field);
                    (
                        fields[*field as usize].clone(),
                        format!("({selector} {term})"),
                    )
                }
                Proj::VariantField { variant, field } => {
                    let fields = self.field_types(&ty, Some(*variant))?;
                    let selector = self.sorts.selector(&ty, Some(*variant), *field);
                    (
                        fields[*field as usize].clone(),
                        format!("({selector} {term})"),
                    )
                }
                Proj::Index(index) => match &ty {
                    Type::Array { elem: item, .. } | Type::Map { value: item, .. } => (
                        (**item).clone(),
                        format!("(select {term} {})", Self::index(index)),
                    ),
                    _ => return Err(internal("an index into a non-array")),
                },
            };
            ty = next;
            term = selected;
        }
        Ok(term)
    }

    fn update(
        &self,
        ty: Type,
        term: String,
        path: &[Proj],
        new: &str,
    ) -> Result<String, NotSupported> {
        let Some((first, rest)) = path.split_first() else {
            return Ok(new.to_string());
        };
        Ok(match first {
            Proj::Field(field) | Proj::VariantField { field, .. } => {
                let variant = match first {
                    Proj::VariantField { variant, .. } => Some(*variant),
                    _ => None,
                };
                let fields = self.field_types(&ty, variant)?;
                let mut parts = Vec::new();
                for (index, field_ty) in fields.iter().enumerate() {
                    let selector = self.sorts.selector(&ty, variant, index as u16);
                    let current = format!("({selector} {term})");
                    parts.push(if index == *field as usize {
                        self.update(field_ty.clone(), current, rest, new)?
                    } else {
                        current
                    });
                }
                let rebuilt = format!(
                    "({} {})",
                    self.sorts.constructor(&ty, variant),
                    parts.join(" ")
                );
                match variant {
                    Some(variant) => format!(
                        "(ite ({} {term}) {rebuilt} {term})",
                        self.sorts.tester(&ty, variant)
                    ),
                    None => rebuilt,
                }
            }
            Proj::Index(index) => {
                let (Type::Array { elem, .. } | Type::Map { value: elem, .. }) = &ty else {
                    return Err(internal("an index into a non-array"));
                };
                let at = Self::index(index);
                let inner =
                    self.update((**elem).clone(), format!("(select {term} {at})"), rest, new)?;
                format!("(store {term} {at} {inner})")
            }
        })
    }

    fn tag(&self, value: ValueId) -> Result<String, NotSupported> {
        let ty = self.ty(value);
        let Type::Adt(id) = ty else {
            return Err(internal("the tag of a non-enum"));
        };
        let TypeDef::Enum { variants, .. } = self.sorts.table.get(*id) else {
            return Err(internal("the tag of a struct"));
        };
        let mut term = (variants.len().saturating_sub(1)).to_string();
        for index in (0..variants.len().saturating_sub(1)).rev() {
            term = format!(
                "(ite ({} {}) {index} {term})",
                self.sorts.tester(ty, index as u16),
                name(value)
            );
        }
        Ok(term)
    }

    fn unary(&self, op: UnOp, value: ValueId) -> Result<String, NotSupported> {
        let term = name(value);
        let ty = self.ty(value);
        Ok(match (op, ty.int_shape()) {
            (UnOp::Not, _) if *ty == Type::Bool => format!("(not {term})"),
            (UnOp::Neg(Arith::Wrapping), Some((bits, signed))) => {
                ints::wrap(bits, signed, &format!("(- {term})"))
            }
            (UnOp::Neg(_), Some(_)) => format!("(- {term})"),
            (UnOp::BitNot | UnOp::Not, Some((bits, signed))) => {
                if signed {
                    format!("(- (- 1) {term})")
                } else {
                    format!("(- {} {term})", ints::bounds(bits, false).1)
                }
            }
            _ => return Err(internal("a unary operation on this type")),
        })
    }

    /// An operation that takes a round trip through bit vectors: the round
    /// trip, or in an approximate encoding only that the result is well
    /// formed, and for unsigned values the bounds every bit pattern keeps.
    fn bounded(
        &self,
        dst: ValueId,
        op: BinOp,
        lhs: ValueId,
        rhs: ValueId,
    ) -> Result<Option<Vec<String>>, NotSupported> {
        if *self.ty(lhs) == Type::Bool || self.ty(lhs).int_shape().is_none() {
            return Ok(None);
        }
        let term = self.binary(op, lhs, rhs)?;
        if !term.contains("int2bv") {
            return Ok(None);
        }
        let result = name(dst);
        let (a, b) = (name(lhs), name(rhs));
        let mut facts = if self.approximate {
            self.sorts.wf(self.ty(dst), &result).into_iter().collect()
        } else {
            vec![format!("(= {result} {term})")]
        };
        if matches!(self.ty(lhs).int_shape(), Some((_, true))) {
            return Ok(Some(facts));
        }
        facts.extend(bits::bounds(op, &result, &a, &b));
        for (shifted, other) in [(lhs, &b), (rhs, &a)] {
            if let Some(zeros) = self.shifted.get(&shifted)
                && let Some(fact) = bits::disjoint(op, &result, other, &name(shifted), *zeros)
            {
                facts.push(fact);
            }
        }
        Ok(Some(facts))
    }

    fn binary(&self, op: BinOp, lhs: ValueId, rhs: ValueId) -> Result<String, NotSupported> {
        let (a, b) = (name(lhs), name(rhs));
        let ty = self.ty(lhs);
        if *ty == Type::Bool {
            return Ok(match op {
                BinOp::Eq => format!("(= {a} {b})"),
                BinOp::Ne => format!("(not (= {a} {b}))"),
                BinOp::And | BinOp::BitAnd => format!("(and {a} {b})"),
                BinOp::Or | BinOp::BitOr => format!("(or {a} {b})"),
                BinOp::Implies => format!("(=> {a} {b})"),
                BinOp::BitXor => format!("(xor {a} {b})"),
                _ => return Err(internal("an arithmetic operation on booleans")),
            });
        }
        let Some((bits, signed)) = ty.int_shape() else {
            return match op {
                BinOp::Eq => Ok(format!("(= {a} {b})")),
                BinOp::Ne => Ok(format!("(not (= {a} {b}))")),
                _ => Err(internal("an operation on aggregates")),
            };
        };
        let wrapped = |arith: Arith, term: String| match arith {
            Arith::Wrapping => ints::wrap(bits, signed, &term),
            Arith::Exact | Arith::Checked => term,
        };
        let known = |value: ValueId| self.constants.get(&value).map(|value| value.bits());
        if matches!(op, BinOp::Shl | BinOp::Shr)
            && let Some(amount) = known(rhs)
            && let Some(term) = bits::shift(op, bits, signed, &a, amount)
        {
            return Ok(term);
        }
        if matches!(op, BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor) {
            let found = match (known(lhs), known(rhs)) {
                (_, Some(mask)) => bits::with_known(op, bits, signed, &a, mask),
                (Some(mask), None) => bits::with_known(op, bits, signed, &b, mask),
                (None, None) => None,
            };
            if let Some(term) = found {
                return Ok(term);
            }
        }
        let bitwise = |operator: &str| {
            ints::from_bits(
                bits,
                signed,
                &format!(
                    "({operator} {} {})",
                    ints::to_bits(bits, &a),
                    ints::to_bits(bits, &b)
                ),
            )
        };
        Ok(match op {
            BinOp::Add(arith) => wrapped(arith, format!("(+ {a} {b})")),
            BinOp::Sub(arith) => wrapped(arith, format!("(- {a} {b})")),
            BinOp::Mul(arith) => wrapped(arith, format!("(* {a} {b})")),
            BinOp::Div(arith) => {
                let quotient = if signed {
                    format!("(tdiv {a} {b})")
                } else {
                    format!("(div {a} {b})")
                };
                wrapped(arith, quotient)
            }
            BinOp::Rem(_) => {
                if signed {
                    format!("(trem {a} {b})")
                } else {
                    format!("(mod {a} {b})")
                }
            }
            BinOp::Pow(_) => return Err(internal("a power")),
            BinOp::Shl => bitwise("bvshl"),
            BinOp::Shr => bitwise(if signed { "bvashr" } else { "bvlshr" }),
            BinOp::BitAnd => bitwise("bvand"),
            BinOp::BitOr => bitwise("bvor"),
            BinOp::BitXor => bitwise("bvxor"),
            BinOp::Eq => format!("(= {a} {b})"),
            BinOp::Ne => format!("(not (= {a} {b}))"),
            BinOp::Lt => format!("(< {a} {b})"),
            BinOp::Le => format!("(<= {a} {b})"),
            BinOp::Gt => format!("(> {a} {b})"),
            BinOp::Ge => format!("(>= {a} {b})"),
            BinOp::And | BinOp::Or | BinOp::Implies => {
                return Err(internal("a logical operation on integers"));
            }
        })
    }

    fn intrinsic(
        &self,
        dst: ValueId,
        op: Intrinsic,
        args: &[ValueId],
    ) -> Result<String, NotSupported> {
        let Some((bits, signed)) = self.ty(dst).int_shape() else {
            return Err(internal("an integer intrinsic with a non-integer result"));
        };
        let arg = |index: usize| {
            args.get(index)
                .map(|value| name(*value))
                .ok_or_else(|| internal("an intrinsic without its arguments"))
        };
        let saturate = |term: String| {
            let (lo, hi) = ints::bounds(bits, signed);
            let (lo, hi) = (ints::literal(&lo), ints::literal(&hi));
            format!("(ite (< {term} {lo}) {lo} (ite (> {term} {hi}) {hi} {term}))")
        };
        Ok(match op {
            Intrinsic::Bitcast => ints::wrap(bits, signed, &arg(0)?),
            Intrinsic::SaturatingAdd => saturate(format!("(+ {} {})", arg(0)?, arg(1)?)),
            Intrinsic::SaturatingSub => saturate(format!("(- {} {})", arg(0)?, arg(1)?)),
            Intrinsic::SaturatingMul => saturate(format!("(* {} {})", arg(0)?, arg(1)?)),
        })
    }
}
