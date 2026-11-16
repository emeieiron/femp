//! SMT sorts for VIR types. Integers are mathematical integers with range
//! facts; structs, enums, tuples and `()` are datatypes; arrays are SMT
//! arrays indexed by integers, of which only the first `len` cells matter.

use std::collections::BTreeSet;
use std::fmt::Write;

use femp_vir::{Type, TypeDef, TypeId, TypeTable};

pub struct Sorts<'a> {
    pub table: &'a TypeTable,
    tuples: Vec<Vec<Type>>,
    ints: BTreeSet<(u16, bool)>,
}

impl<'a> Sorts<'a> {
    pub fn new(table: &'a TypeTable) -> Self {
        let mut sorts = Self {
            table,
            tuples: Vec::new(),
            ints: BTreeSet::new(),
        };
        for id in table.ids() {
            match table.get(id) {
                TypeDef::Struct { fields, .. } => {
                    for field in fields {
                        sorts.note(&field.ty);
                    }
                }
                TypeDef::Enum { variants, .. } => {
                    for variant in variants {
                        for field in &variant.fields {
                            sorts.note(field);
                        }
                    }
                }
            }
        }
        sorts
    }

    /// Records the tuples and integer widths a type needs.
    pub fn note(&mut self, ty: &Type) {
        match ty {
            Type::Int { bits, signed } => {
                self.ints.insert((*bits, *signed));
            }
            Type::Tuple(items) => {
                for item in items {
                    self.note(item);
                }
                if !self.tuples.contains(items) {
                    self.tuples.push(items.clone());
                }
            }
            Type::Array { elem, .. } => self.note(elem),
            Type::Bool | Type::Unit | Type::Adt(_) => {}
        }
    }

    fn tuple_index(&self, items: &[Type]) -> usize {
        self.tuples
            .iter()
            .position(|tuple| tuple == items)
            .expect("tuples are noted before they are named")
    }

    pub fn sort(&self, ty: &Type) -> String {
        match ty {
            Type::Bool => "Bool".to_string(),
            Type::Int { .. } => "Int".to_string(),
            Type::Unit => "Unit".to_string(),
            Type::Tuple(items) => format!("Tup{}", self.tuple_index(items)),
            Type::Adt(id) => format!("T{}", id.0),
            Type::Array { elem, .. } => format!("(Array Int {})", self.sort(elem)),
        }
    }

    /// The constructor of a struct, tuple or `()`, and of an enum variant.
    pub fn constructor(&self, ty: &Type, variant: Option<u16>) -> String {
        match (ty, variant) {
            (Type::Adt(id), Some(variant)) => format!("T{}.v{variant}", id.0),
            (Type::Adt(id), None) => format!("T{}.mk", id.0),
            (Type::Tuple(items), _) => format!("Tup{}.mk", self.tuple_index(items)),
            _ => "Unit.mk".to_string(),
        }
    }

    pub fn selector(&self, ty: &Type, variant: Option<u16>, field: u16) -> String {
        match (ty, variant) {
            (Type::Adt(id), Some(variant)) => format!("T{}.v{variant}.{field}", id.0),
            (Type::Adt(id), None) => format!("T{}.{field}", id.0),
            (Type::Tuple(items), _) => format!("Tup{}.{field}", self.tuple_index(items)),
            _ => unreachable!("only products have fields"),
        }
    }

    pub fn tester(&self, ty: &Type, variant: u16) -> String {
        format!("(_ is {})", self.constructor(ty, Some(variant)))
    }

    pub fn wf_name(ty: &Type) -> Option<String> {
        match ty {
            Type::Int { bits, signed } => {
                Some(format!("wf.{}{bits}", if *signed { 'i' } else { 'u' }))
            }
            _ => None,
        }
    }

    /// Whether every integer inside a value of `ty` lies in its type.
    pub fn wf(&self, ty: &Type, term: &str) -> Option<String> {
        match ty {
            Type::Bool | Type::Unit => None,
            Type::Int { .. } => Some(format!("({} {term})", Self::wf_name(ty)?)),
            Type::Tuple(items) => self.wf_fields(ty, None, items, term),
            Type::Adt(id) => {
                let needs = self.needs_wf(ty);
                needs.then(|| format!("(wf.T{} {term})", id.0))
            }
            Type::Array { elem, len } => {
                if !self.needs_wf(elem) {
                    return None;
                }
                let body = self.wf(elem, &format!("(select {term} i)"))?;
                Some(format!(
                    "(forall ((i Int)) (=> (and (<= 0 i) (< i {len})) {body}))"
                ))
            }
        }
    }

    fn wf_fields(
        &self,
        ty: &Type,
        variant: Option<u16>,
        fields: &[Type],
        term: &str,
    ) -> Option<String> {
        let parts: Vec<String> = fields
            .iter()
            .enumerate()
            .filter_map(|(index, field)| {
                let selected = format!("({} {term})", self.selector(ty, variant, index as u16));
                self.wf(field, &selected)
            })
            .collect();
        match parts.len() {
            0 => None,
            1 => parts.into_iter().next(),
            _ => Some(format!("(and {})", parts.join(" "))),
        }
    }

    pub fn needs_wf(&self, ty: &Type) -> bool {
        match ty {
            Type::Bool | Type::Unit => false,
            Type::Int { .. } => true,
            Type::Tuple(items) => items.iter().any(|item| self.needs_wf(item)),
            Type::Array { elem, .. } => self.needs_wf(elem),
            Type::Adt(id) => match self.table.get(*id) {
                TypeDef::Struct { fields, .. } => fields.iter().any(|f| self.needs_wf(&f.ty)),
                TypeDef::Enum { variants, .. } => variants
                    .iter()
                    .any(|variant| variant.fields.iter().any(|f| self.needs_wf(f))),
            },
        }
    }

    /// The datatype declarations, then the range predicates of integers and
    /// the well-formedness predicates of datatypes, dependencies first.
    pub fn declarations(&self) -> String {
        let mut out = String::new();
        let mut names = vec!["(Unit 0)".to_string()];
        let mut bodies = vec!["((Unit.mk))".to_string()];
        for (index, items) in self.tuples.iter().enumerate() {
            let ty = Type::Tuple(items.clone());
            names.push(format!("(Tup{index} 0)"));
            bodies.push(format!("({})", self.product(&ty, None, items)));
        }
        for id in self.table.ids() {
            let ty = Type::Adt(id);
            names.push(format!("(T{} 0)", id.0));
            let body = match self.table.get(id) {
                TypeDef::Struct { fields, .. } => {
                    let fields: Vec<Type> = fields.iter().map(|field| field.ty.clone()).collect();
                    format!("({})", self.product(&ty, None, &fields))
                }
                TypeDef::Enum { variants, .. } => {
                    let variants: Vec<String> = variants
                        .iter()
                        .enumerate()
                        .map(|(index, variant)| {
                            self.product(&ty, Some(index as u16), &variant.fields)
                        })
                        .collect();
                    format!("({})", variants.join(" "))
                }
            };
            bodies.push(body);
        }
        let _ = writeln!(
            out,
            "(declare-datatypes ({}) ({}))",
            names.join(" "),
            bodies.join(" ")
        );
        for (bits, signed) in &self.ints {
            let ty = Type::Int {
                bits: *bits,
                signed: *signed,
            };
            let (lo, hi) = super::ints::bounds(*bits, *signed);
            let _ = writeln!(
                out,
                "(define-fun {} ((x Int)) Bool (and (<= {} x) (<= x {})))",
                Self::wf_name(&ty).unwrap_or_default(),
                super::ints::literal(&lo),
                super::ints::literal(&hi)
            );
        }
        let mut done = BTreeSet::new();
        for id in self.table.ids() {
            self.define_wf(id, &mut done, &mut out);
        }
        out
    }

    fn product(&self, ty: &Type, variant: Option<u16>, fields: &[Type]) -> String {
        let constructor = self.constructor(ty, variant);
        if fields.is_empty() {
            return format!("({constructor})");
        }
        let fields: Vec<String> = fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                format!(
                    "({} {})",
                    self.selector(ty, variant, index as u16),
                    self.sort(field)
                )
            })
            .collect();
        format!("({constructor} {})", fields.join(" "))
    }

    fn define_wf(&self, id: TypeId, done: &mut BTreeSet<TypeId>, out: &mut String) {
        if !done.insert(id) {
            return;
        }
        let ty = Type::Adt(id);
        if !self.needs_wf(&ty) {
            return;
        }
        let mut deps = Vec::new();
        collect_adts(self.table, &ty, &mut deps);
        for dep in deps.into_iter().filter(|dep| *dep != id) {
            self.define_wf(dep, done, out);
        }
        let body = match self.table.get(id) {
            TypeDef::Struct { fields, .. } => {
                let fields: Vec<Type> = fields.iter().map(|field| field.ty.clone()).collect();
                self.wf_fields(&ty, None, &fields, "x")
                    .unwrap_or_else(|| "true".to_string())
            }
            TypeDef::Enum { variants, .. } => {
                let parts: Vec<String> = variants
                    .iter()
                    .enumerate()
                    .filter_map(|(index, variant)| {
                        let inner =
                            self.wf_fields(&ty, Some(index as u16), &variant.fields, "x")?;
                        Some(format!(
                            "(=> ({} x) {inner})",
                            self.tester(&ty, index as u16)
                        ))
                    })
                    .collect();
                if parts.is_empty() {
                    "true".to_string()
                } else {
                    format!("(and {})", parts.join(" "))
                }
            }
        };
        let _ = writeln!(out, "(define-fun wf.T{} ((x T{})) Bool {body})", id.0, id.0);
    }
}

/// The structs and enums a type's values contain, directly or nested.
fn collect_adts(table: &TypeTable, ty: &Type, out: &mut Vec<TypeId>) {
    match ty {
        Type::Adt(id) => {
            if out.contains(id) {
                return;
            }
            out.push(*id);
            match table.get(*id) {
                TypeDef::Struct { fields, .. } => {
                    for field in fields {
                        collect_adts(table, &field.ty, out);
                    }
                }
                TypeDef::Enum { variants, .. } => {
                    for variant in variants {
                        for field in &variant.fields {
                            collect_adts(table, field, out);
                        }
                    }
                }
            }
        }
        Type::Tuple(items) => {
            for item in items {
                collect_adts(table, item, out);
            }
        }
        Type::Array { elem, .. } => collect_adts(table, elem, out),
        Type::Bool | Type::Int { .. } | Type::Unit => {}
    }
}
