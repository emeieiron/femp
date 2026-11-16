use std::fmt;

/// An index into a program's [`TypeTable`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    Bool,
    Int {
        bits: u16,
        signed: bool,
    },
    /// The empty tuple, and what a function without a result returns.
    Unit,
    Tuple(Vec<Type>),
    /// A struct or enum declared in the table.
    Adt(TypeId),
    Array {
        elem: Box<Type>,
        len: usize,
    },
}

impl Type {
    pub fn uint(bits: u16) -> Self {
        Self::Int {
            bits,
            signed: false,
        }
    }

    pub fn int(bits: u16) -> Self {
        Self::Int { bits, signed: true }
    }

    pub fn is_int(&self) -> bool {
        matches!(self, Self::Int { .. })
    }

    pub fn int_shape(&self) -> Option<(u16, bool)> {
        match self {
            Self::Int { bits, signed } => Some((*bits, *signed)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Field {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<Type>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeDef {
    Struct {
        name: String,
        fields: Vec<Field>,
    },
    Enum {
        name: String,
        variants: Vec<Variant>,
    },
}

impl TypeDef {
    pub fn name(&self) -> &str {
        match self {
            Self::Struct { name, .. } | Self::Enum { name, .. } => name,
        }
    }
}

/// The structs and enums a program uses, monomorphized: `Option<u8>` and
/// `Option<u64>` are two entries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct TypeTable {
    pub defs: Vec<TypeDef>,
}

impl TypeTable {
    pub fn get(&self, id: TypeId) -> &TypeDef {
        &self.defs[id.0 as usize]
    }

    pub fn push(&mut self, def: TypeDef) -> TypeId {
        self.defs.push(def);
        TypeId(self.defs.len() as u32 - 1)
    }

    pub fn ids(&self) -> impl Iterator<Item = TypeId> {
        (0..self.defs.len() as u32).map(TypeId)
    }

    /// The type of the fields of a product: a struct, a tuple, or one variant
    /// of an enum.
    pub fn fields(&self, ty: &Type, variant: Option<u16>) -> Option<Vec<Type>> {
        match (ty, variant) {
            (Type::Tuple(items), None) => Some(items.clone()),
            (Type::Unit, None) => Some(Vec::new()),
            (Type::Adt(id), variant) => match (self.get(*id), variant) {
                (TypeDef::Struct { fields, .. }, None) => {
                    Some(fields.iter().map(|field| field.ty.clone()).collect())
                }
                (TypeDef::Enum { variants, .. }, Some(variant)) => {
                    variants.get(variant as usize).map(|v| v.fields.clone())
                }
                _ => None,
            },
            _ => None,
        }
    }

    pub fn display<'a>(&'a self, ty: &'a Type) -> impl fmt::Display + 'a {
        TypeName { table: self, ty }
    }
}

struct TypeName<'a> {
    table: &'a TypeTable,
    ty: &'a Type,
}

impl fmt::Display for TypeName<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ty {
            Type::Bool => f.write_str("bool"),
            Type::Int { bits, signed } => write!(f, "{}{bits}", if *signed { 'i' } else { 'u' }),
            Type::Unit => f.write_str("()"),
            Type::Tuple(items) => {
                f.write_str("(")?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}", self.table.display(item))?;
                }
                f.write_str(")")
            }
            Type::Adt(id) => f.write_str(self.table.get(*id).name()),
            Type::Array { elem, len } => write!(f, "[{}; {len}]", self.table.display(elem)),
        }
    }
}
