//! Runs a VIR function on concrete values, as the compiled program would:
//! checked arithmetic panics outside its type, division truncates, and a
//! storage map holds zero at every key no one wrote. Tools use it to check
//! a claim of a state read back from a target, where the solver is not
//! needed. A value the model leaves open is the zero of its type.

use femp_core::property::{PanicKind, Ty, U256, Value};
use num_bigint::{BigInt, Sign};

use crate::program::{
    Arith, BinOp, Check, Const, Expr, FuncId, Function, Index, Intrinsic, Program, Proj, SpecOp,
    StmtKind, Terminator, UnOp, ValueId,
};
use crate::types::{Type, TypeDef, TypeTable};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Val {
    Bool(bool),
    Int(BigInt),
    Unit,
    Tuple(Vec<Val>),
    /// A struct, or an enum's variant, with its fields.
    Adt {
        variant: u16,
        fields: Vec<Val>,
    },
    Array(Vec<Val>),
    /// Every key holds `default` but those listed.
    Map {
        default: Box<Val>,
        entries: Vec<(Val, Val)>,
    },
}

/// Why a run did not return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Halt {
    Panic(PanicKind),
    /// A specification statement failed.
    Spec(SpecOp),
    /// An assertion a task added failed.
    Check(Check),
    /// The run took more steps than it may.
    Limit,
    /// The run reached code it cannot run, such as a value known only by
    /// the name of a function.
    Unsupported(String),
}

impl std::fmt::Display for Halt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Halt::Panic(kind) => write!(f, "it panics ({kind})"),
            Halt::Spec(op) => {
                let op = match op {
                    SpecOp::Requires => "requires",
                    SpecOp::Ensures => "ensures",
                    SpecOp::Invariant => "invariant",
                };
                write!(f, "a `{op}` statement fails")
            }
            Halt::Check(check) => write!(f, "{check}"),
            Halt::Limit => write!(f, "it runs longer than the interpreter allows"),
            Halt::Unsupported(what) => write!(f, "it reaches {what}"),
        }
    }
}

const STEP_LIMIT: u64 = 10_000_000;
const DEPTH_LIMIT: usize = 256;

impl Val {
    pub fn int(number: impl Into<BigInt>) -> Self {
        Self::Int(number.into())
    }

    /// A primitive value as the interpreter holds it.
    pub fn of(value: &Value) -> Self {
        match value.ty() {
            Ty::Bool => Self::Bool(value.as_bool().unwrap_or(false)),
            _ => Self::Int(number(value)),
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(flag) => Some(*flag),
            _ => None,
        }
    }

    /// The zero of `ty`: what storage no one wrote holds.
    pub fn zero(ty: &Type, table: &TypeTable) -> Self {
        match ty {
            Type::Bool => Self::Bool(false),
            Type::Int { .. } => Self::int(0),
            Type::Unit => Self::Unit,
            Type::Tuple(items) => {
                Self::Tuple(items.iter().map(|item| Self::zero(item, table)).collect())
            }
            Type::Array { elem, len } => Self::Array(vec![Self::zero(elem, table); *len]),
            Type::Map { value, .. } => Self::Map {
                default: Box::new(Self::zero(value, table)),
                entries: Vec::new(),
            },
            Type::Adt(id) => match table.get(*id) {
                TypeDef::Struct { fields, .. } => Self::Adt {
                    variant: 0,
                    fields: fields
                        .iter()
                        .map(|field| Self::zero(&field.ty, table))
                        .collect(),
                },
                TypeDef::Enum { variants, .. } => Self::Adt {
                    variant: 0,
                    fields: variants
                        .first()
                        .map(|variant| {
                            variant
                                .fields
                                .iter()
                                .map(|field| Self::zero(field, table))
                                .collect()
                        })
                        .unwrap_or_default(),
                },
            },
        }
    }

    fn field(&self, index: usize) -> Result<&Val, Halt> {
        match self {
            Self::Tuple(items) | Self::Adt { fields: items, .. } => items
                .get(index)
                .ok_or_else(|| Halt::Unsupported("a field that does not exist".to_string())),
            _ => Err(Halt::Unsupported(
                "a field of a value without fields".to_string(),
            )),
        }
    }

    fn field_mut(&mut self, index: usize) -> Result<&mut Val, Halt> {
        match self {
            Self::Tuple(items) | Self::Adt { fields: items, .. } => items
                .get_mut(index)
                .ok_or_else(|| Halt::Unsupported("a field that does not exist".to_string())),
            _ => Err(Halt::Unsupported(
                "a field of a value without fields".to_string(),
            )),
        }
    }

    /// The entry of a map at `key`.
    pub fn entry(&self, key: &Val) -> Option<&Val> {
        let Self::Map { default, entries } = self else {
            return None;
        };
        Some(
            entries
                .iter()
                .find(|(known, _)| known == key)
                .map_or(default.as_ref(), |(_, value)| value),
        )
    }

    /// Writes the entry of a map at `key`.
    pub fn set_entry(&mut self, key: Val, value: Val) {
        if let Self::Map { entries, .. } = self {
            match entries.iter_mut().find(|(known, _)| *known == key) {
                Some((_, held)) => *held = value,
                None => entries.push((key, value)),
            }
        }
    }
}

fn number(value: &Value) -> BigInt {
    let bits = value.bits();
    let unsigned = BigInt::from_bytes_be(Sign::Plus, &bits.to_be_bytes::<32>());
    if value.is_negative() {
        unsigned - (BigInt::from(1) << value.ty().bits())
    } else {
        unsigned
    }
}

/// The value of `number` as a primitive of `ty`, if it fits.
pub fn primitive(ty: Ty, val: &Val) -> Option<Value> {
    match (ty, val) {
        (Ty::Bool, Val::Bool(flag)) => Some(Value::boolean(*flag)),
        (Ty::Uint(_) | Ty::Int(_), Val::Int(number)) => {
            let (lo, hi) = range(ty.bits(), matches!(ty, Ty::Int(_)));
            if number < &lo || number > &hi {
                return None;
            }
            let modulus = BigInt::from(1) << ty.bits();
            let bits = ((number % &modulus) + &modulus) % &modulus;
            let (_, bytes) = bits.to_bytes_be();
            Some(Value::from_bits(ty, U256::from_be_slice(&bytes)))
        }
        _ => None,
    }
}

fn range(bits: u16, signed: bool) -> (BigInt, BigInt) {
    if signed {
        let half = BigInt::from(1) << (bits - 1);
        (-half.clone(), half - 1)
    } else {
        (BigInt::from(0), (BigInt::from(1) << bits) - 1)
    }
}

/// Runs `func` of `program` with `args`.
pub fn call(program: &Program, func: FuncId, args: Vec<Val>) -> Result<Val, Halt> {
    let mut machine = Machine {
        program,
        steps: 0,
        depth: 0,
    };
    machine.call(func, args)
}

struct Machine<'a> {
    program: &'a Program,
    steps: u64,
    depth: usize,
}

struct Frame<'a> {
    function: &'a Function,
    values: Vec<Option<Val>>,
    slots: Vec<Option<Val>>,
}

impl Frame<'_> {
    fn get(&self, value: ValueId) -> Result<Val, Halt> {
        self.values[value.index()]
            .clone()
            .ok_or_else(|| Halt::Unsupported(format!("{value} is read before it is defined")))
    }

    fn int(&self, value: ValueId) -> Result<BigInt, Halt> {
        match self.get(value)? {
            Val::Int(number) => Ok(number),
            Val::Bool(flag) => Ok(BigInt::from(u8::from(flag))),
            _ => Err(Halt::Unsupported(
                "arithmetic on a value that is no integer".to_string(),
            )),
        }
    }

    fn boolean(&self, value: ValueId) -> Result<bool, Halt> {
        self.get(value)?
            .as_bool()
            .ok_or_else(|| Halt::Unsupported("a condition that is no `bool`".to_string()))
    }
}

impl<'a> Machine<'a> {
    fn call(&mut self, func: FuncId, args: Vec<Val>) -> Result<Val, Halt> {
        if self.depth >= DEPTH_LIMIT {
            return Err(Halt::Limit);
        }
        self.depth += 1;
        let function = self.program.function(func);
        let mut frame = Frame {
            function,
            values: vec![None; function.values.len()],
            slots: vec![None; function.slots.len()],
        };
        for (param, arg) in function.params.iter().zip(args) {
            frame.values[param.index()] = Some(arg);
        }
        let result = self.run(&mut frame);
        self.depth -= 1;
        result
    }

    fn run(&mut self, frame: &mut Frame<'a>) -> Result<Val, Halt> {
        let function = frame.function;
        let mut block = function.entry;
        loop {
            let data = function.block(block);
            for stmt in &data.stmts {
                self.tick()?;
                self.stmt(frame, &stmt.kind)?;
            }
            let succ = match &data.term {
                Terminator::Return(value) => return frame.get(*value),
                Terminator::Fail(kind) => return Err(Halt::Panic(kind.clone())),
                Terminator::Unreachable => {
                    return Err(Halt::Unsupported(
                        "code the compiler deems unreachable".to_string(),
                    ));
                }
                Terminator::Stop => {
                    return Err(Halt::Unsupported("an execution cut off".to_string()));
                }
                Terminator::Goto(succ) => succ,
                Terminator::Branch {
                    cond,
                    then,
                    otherwise,
                } => {
                    if frame.boolean(*cond)? {
                        then
                    } else {
                        otherwise
                    }
                }
                Terminator::Switch {
                    value,
                    cases,
                    default,
                } => {
                    let Val::Adt { variant, .. } = frame.get(*value)? else {
                        return Err(Halt::Unsupported("a switch on no enum".to_string()));
                    };
                    cases
                        .iter()
                        .find(|(case, _)| *case == variant)
                        .map(|(_, succ)| succ)
                        .or(default.as_ref())
                        .ok_or_else(|| Halt::Unsupported("a variant without a case".to_string()))?
                }
            };
            self.tick()?;
            let args: Vec<Val> = succ
                .args
                .iter()
                .map(|arg| frame.get(*arg))
                .collect::<Result<_, _>>()?;
            let target = function.block(succ.block);
            for (param, arg) in target.params.iter().zip(args) {
                frame.values[param.index()] = Some(arg);
            }
            block = succ.block;
        }
    }

    fn tick(&mut self) -> Result<(), Halt> {
        self.steps += 1;
        if self.steps > STEP_LIMIT {
            return Err(Halt::Limit);
        }
        Ok(())
    }

    fn stmt(&mut self, frame: &mut Frame<'a>, stmt: &StmtKind) -> Result<(), Halt> {
        match stmt {
            StmtKind::Def { dst, expr } => {
                let value = self.expr(frame, *dst, expr)?;
                frame.values[dst.index()] = Some(value);
            }
            StmtKind::Store { slot, path, value } => {
                let new = frame.get(*value)?;
                let indexes = self.indexes(frame, path)?;
                let ty = frame.function.slots[slot.0 as usize].ty.clone();
                let held = frame.slots[slot.0 as usize]
                    .get_or_insert_with(|| Val::zero(&ty, &self.program.types));
                write(held, path, &indexes, new)?;
            }
            StmtKind::CheckIndex { index, len } => {
                let index = frame.int(*index)?;
                if index < BigInt::from(0) || index >= BigInt::from(*len) {
                    return Err(Halt::Panic(PanicKind::Bounds));
                }
            }
            StmtKind::Spec { op, cond } => {
                if !frame.boolean(*cond)? {
                    return Err(Halt::Spec(*op));
                }
            }
            StmtKind::Assume(cond) => {
                if !frame.boolean(*cond)? {
                    return Err(Halt::Unsupported(
                        "an execution its assumptions rule out".to_string(),
                    ));
                }
            }
            StmtKind::Assert { cond, check } => {
                if !frame.boolean(*cond)? {
                    return Err(Halt::Check(check.clone()));
                }
            }
        }
        Ok(())
    }

    /// The value of each index of `path`, read now.
    fn indexes(&self, frame: &Frame<'_>, path: &[Proj]) -> Result<Vec<Option<Val>>, Halt> {
        path.iter()
            .map(|projection| match projection {
                Proj::Index(Index::Value(value)) => frame.get(*value).map(Some),
                Proj::Index(Index::Const(index)) => Ok(Some(Val::int(*index as u64))),
                _ => Ok(None),
            })
            .collect()
    }

    fn expr(&mut self, frame: &mut Frame<'a>, dst: ValueId, expr: &Expr) -> Result<Val, Halt> {
        let ty = frame.function.ty(dst).clone();
        Ok(match expr {
            Expr::Use(value) => frame.get(*value)?,
            Expr::Const(constant) => match constant {
                Const::Bool(flag) => Val::Bool(*flag),
                Const::Int(value) => Val::Int(number(value)),
                Const::Unit => Val::Unit,
            },
            Expr::Unary { op, value } => match op {
                UnOp::Not => Val::Bool(!frame.boolean(*value)?),
                UnOp::Neg(arith) => fit(-frame.int(*value)?, &ty, *arith)?,
                UnOp::BitNot => {
                    let (bits, signed) = int_shape(&ty)?;
                    let number = frame.int(*value)?;
                    from_bits(!to_bits(&number, bits) & mask(bits), bits, signed)
                }
            },
            Expr::Binary { op, lhs, rhs } => self.binary(frame, &ty, *op, *lhs, *rhs)?,
            Expr::Cast { value, .. } => Val::Int(frame.int(*value)?),
            Expr::Make { ty: made, fields } => {
                let fields: Vec<Val> = fields
                    .iter()
                    .map(|field| frame.get(*field))
                    .collect::<Result<_, _>>()?;
                match made {
                    Type::Tuple(_) => Val::Tuple(fields),
                    Type::Array { .. } => Val::Array(fields),
                    _ => Val::Adt { variant: 0, fields },
                }
            }
            Expr::MakeEnum {
                variant, fields, ..
            } => Val::Adt {
                variant: *variant,
                fields: fields
                    .iter()
                    .map(|field| frame.get(*field))
                    .collect::<Result<_, _>>()?,
            },
            Expr::Project { value, path } => {
                let indexes = self.indexes(frame, path)?;
                read(&frame.get(*value)?, path, &indexes)?
            }
            Expr::Update { value, path, new } => {
                let indexes = self.indexes(frame, path)?;
                let mut whole = frame.get(*value)?;
                write(&mut whole, path, &indexes, frame.get(*new)?)?;
                whole
            }
            Expr::Load { slot, path } => {
                let indexes = self.indexes(frame, path)?;
                let held = frame.slots[slot.0 as usize].clone().ok_or_else(|| {
                    Halt::Unsupported("a slot read before it is written".to_string())
                })?;
                read(&held, path, &indexes)?
            }
            Expr::IsVariant { value, variant } => match frame.get(*value)? {
                Val::Adt { variant: held, .. } => Val::Bool(held == *variant),
                _ => return Err(Halt::Unsupported("the variant of no enum".to_string())),
            },
            Expr::Tag { value } => match frame.get(*value)? {
                Val::Adt { variant, .. } => Val::int(variant),
                _ => return Err(Halt::Unsupported("the tag of no enum".to_string())),
            },
            Expr::Repeat { value, len } => Val::Array(vec![frame.get(*value)?; *len]),
            Expr::Fill { value } => Val::Map {
                default: Box::new(frame.get(*value)?),
                entries: Vec::new(),
            },
            Expr::Call { func, args } => {
                let args: Vec<Val> = args
                    .iter()
                    .map(|arg| frame.get(*arg))
                    .collect::<Result<_, _>>()?;
                self.call(*func, args)?
            }
            Expr::Intrinsic { op, args } => {
                let (bits, signed) = int_shape(&ty)?;
                let arg = |index: usize| frame.int(args[index]);
                match op {
                    Intrinsic::Bitcast => {
                        from_bits(to_bits(&arg(0)?, 256) & mask(bits), bits, signed)
                    }
                    Intrinsic::SaturatingAdd => saturate(arg(0)? + arg(1)?, bits, signed),
                    Intrinsic::SaturatingSub => saturate(arg(0)? - arg(1)?, bits, signed),
                    Intrinsic::SaturatingMul => saturate(arg(0)? * arg(1)?, bits, signed),
                }
            }
            Expr::Havoc => Val::zero(&ty, &self.program.types),
            Expr::Apply { name, .. } => {
                return Err(Halt::Unsupported(format!("`{name}`, known only by name")));
            }
        })
    }

    fn binary(
        &mut self,
        frame: &Frame<'_>,
        ty: &Type,
        op: BinOp,
        lhs: ValueId,
        rhs: ValueId,
    ) -> Result<Val, Halt> {
        if matches!(op, BinOp::And | BinOp::Or | BinOp::Implies) {
            let (a, b) = (frame.boolean(lhs)?, frame.boolean(rhs)?);
            return Ok(Val::Bool(match op {
                BinOp::And => a && b,
                BinOp::Or => a || b,
                _ => !a || b,
            }));
        }
        if op.is_comparison() {
            let (a, b) = (frame.get(lhs)?, frame.get(rhs)?);
            let ordered = |ordering: std::cmp::Ordering| -> Result<bool, Halt> {
                let (Val::Int(a), Val::Int(b)) = (&a, &b) else {
                    return Err(Halt::Unsupported(
                        "an order on values that are no integers".to_string(),
                    ));
                };
                Ok(a.cmp(b) == ordering)
            };
            return Ok(Val::Bool(match op {
                BinOp::Eq => a == b,
                BinOp::Ne => a != b,
                BinOp::Lt => ordered(std::cmp::Ordering::Less)?,
                BinOp::Gt => ordered(std::cmp::Ordering::Greater)?,
                BinOp::Le => !ordered(std::cmp::Ordering::Greater)?,
                _ => !ordered(std::cmp::Ordering::Less)?,
            }));
        }
        let (a, b) = (frame.int(lhs)?, frame.int(rhs)?);
        let (bits, signed) = int_shape(ty)?;
        let zero = BigInt::from(0);
        Ok(match op {
            BinOp::Add(arith) => fit(a + b, ty, arith)?,
            BinOp::Sub(arith) => fit(a - b, ty, arith)?,
            BinOp::Mul(arith) => fit(a * b, ty, arith)?,
            BinOp::Div(arith) | BinOp::Rem(arith) => {
                if b == zero {
                    return Err(Halt::Panic(PanicKind::DivisionByZero));
                }
                // Truncating: the quotient rounds toward zero, and the
                // remainder has the sign of the dividend.
                let quotient = &a / &b;
                let result = if matches!(op, BinOp::Div(_)) {
                    quotient
                } else {
                    &a - &quotient * &b
                };
                fit(result, ty, arith)?
            }
            BinOp::Pow(arith) => {
                let exponent = u32::try_from(b)
                    .map_err(|_| Halt::Unsupported("an exponent beyond 32 bits".to_string()))?;
                fit(a.pow(exponent), ty, arith)?
            }
            BinOp::Shl => {
                let shift = usize::try_from(b).unwrap_or(usize::MAX).min(512);
                from_bits((to_bits(&a, bits) << shift) & mask(bits), bits, signed)
            }
            BinOp::Shr => {
                let shift = usize::try_from(b).unwrap_or(usize::MAX).min(512);
                if signed {
                    Val::Int(a >> shift)
                } else {
                    Val::Int(to_bits(&a, bits) >> shift)
                }
            }
            BinOp::BitAnd => from_bits(to_bits(&a, bits) & to_bits(&b, bits), bits, signed),
            BinOp::BitOr => from_bits(to_bits(&a, bits) | to_bits(&b, bits), bits, signed),
            BinOp::BitXor => from_bits(to_bits(&a, bits) ^ to_bits(&b, bits), bits, signed),
            _ => unreachable!("logical and comparison operators return above"),
        })
    }
}

fn int_shape(ty: &Type) -> Result<(u16, bool), Halt> {
    ty.int_shape()
        .ok_or_else(|| Halt::Unsupported("integer arithmetic on another type".to_string()))
}

fn mask(bits: u16) -> BigInt {
    (BigInt::from(1) << bits) - 1
}

/// The two's complement bits of `number` in `bits` bits.
fn to_bits(number: &BigInt, bits: u16) -> BigInt {
    let modulus = BigInt::from(1) << bits;
    ((number % &modulus) + &modulus) % &modulus
}

fn from_bits(bits_value: BigInt, bits: u16, signed: bool) -> Val {
    if signed && bits_value >= (BigInt::from(1) << (bits - 1)) {
        Val::Int(bits_value - (BigInt::from(1) << bits))
    } else {
        Val::Int(bits_value)
    }
}

fn saturate(number: BigInt, bits: u16, signed: bool) -> Val {
    let (lo, hi) = range(bits, signed);
    Val::Int(number.clamp(lo, hi))
}

/// `number` as a value of `ty`: a checked operation panics outside it, a
/// wrapping one keeps the low bits.
fn fit(number: BigInt, ty: &Type, arith: Arith) -> Result<Val, Halt> {
    let (bits, signed) = int_shape(ty)?;
    let (lo, hi) = range(bits, signed);
    match arith {
        Arith::Checked if number < lo || number > hi => Err(Halt::Panic(PanicKind::Overflow)),
        Arith::Wrapping => Ok(from_bits(to_bits(&number, bits), bits, signed)),
        _ => Ok(Val::Int(number)),
    }
}

fn read(value: &Val, path: &[Proj], indexes: &[Option<Val>]) -> Result<Val, Halt> {
    let mut current = value;
    for (projection, index) in path.iter().zip(indexes) {
        current = match projection {
            Proj::Field(field) | Proj::VariantField { field, .. } => {
                current.field(*field as usize)?
            }
            Proj::Index(_) => {
                let key = index.as_ref().expect("an index has a value");
                match current {
                    Val::Array(items) => {
                        let Val::Int(at) = key else {
                            return Err(Halt::Unsupported(
                                "an index that is no integer".to_string(),
                            ));
                        };
                        let at = usize::try_from(at.clone())
                            .map_err(|_| Halt::Panic(PanicKind::Bounds))?;
                        items.get(at).ok_or(Halt::Panic(PanicKind::Bounds))?
                    }
                    Val::Map { .. } => current.entry(key).expect("a map has every entry"),
                    _ => return Err(Halt::Unsupported("an index into no array".to_string())),
                }
            }
        };
    }
    Ok(current.clone())
}

fn write(value: &mut Val, path: &[Proj], indexes: &[Option<Val>], new: Val) -> Result<(), Halt> {
    let Some((first, rest)) = path.split_first() else {
        *value = new;
        return Ok(());
    };
    match first {
        Proj::Field(field) | Proj::VariantField { field, .. } => {
            write(value.field_mut(*field as usize)?, rest, &indexes[1..], new)
        }
        Proj::Index(_) => {
            let key = indexes[0].clone().expect("an index has a value");
            match value {
                Val::Array(items) => {
                    let Val::Int(at) = &key else {
                        return Err(Halt::Unsupported("an index that is no integer".to_string()));
                    };
                    let at =
                        usize::try_from(at.clone()).map_err(|_| Halt::Panic(PanicKind::Bounds))?;
                    let item = items.get_mut(at).ok_or(Halt::Panic(PanicKind::Bounds))?;
                    write(item, rest, &indexes[1..], new)
                }
                Val::Map { .. } => {
                    let mut entry = value.entry(&key).cloned().expect("a map has every entry");
                    write(&mut entry, rest, &indexes[1..], new)?;
                    value.set_entry(key, entry);
                    Ok(())
                }
                _ => Err(Halt::Unsupported("an index into no array".to_string())),
            }
        }
    }
}
