//! Fe types as VIR types. Views and shared borrows read a value that cannot
//! change while they live, so they lower to the value's own type.

use std::collections::HashMap;

use femp_vir::{Field, Type, TypeDef, TypeId, TypeTable, Variant};
use hir::analysis::HirAnalysisDb;
use hir::analysis::semantic::{SemanticInstance, VariantIndex};
use hir::analysis::ty::adt_def::AdtRef;
use hir::analysis::ty::ty_def::{CapabilityKind, PrimTy, TyId};
use hir::hir_def::{EnumVariant, Partial};

use crate::lower::evm::{CONTEXT, is_evm};

pub(crate) struct Types<'db> {
    pub table: TypeTable,
    lowered: HashMap<TyId<'db>, Type>,
    word_bits: u16,
    evm: Option<TypeId>,
}

impl<'db> Types<'db> {
    pub fn new(word_bits: u16) -> Self {
        Self {
            table: TypeTable::default(),
            lowered: HashMap::new(),
            word_bits,
            evm: None,
        }
    }

    pub fn lower(
        &mut self,
        db: &'db dyn HirAnalysisDb,
        instance: SemanticInstance<'db>,
        ty: TyId<'db>,
    ) -> Result<Type, String> {
        if let Some(found) = self.lowered.get(&ty) {
            return Ok(found.clone());
        }
        let lowered = self.compute(db, instance, ty)?;
        self.lowered.insert(ty, lowered.clone());
        Ok(lowered)
    }

    fn compute(
        &mut self,
        db: &'db dyn HirAnalysisDb,
        instance: SemanticInstance<'db>,
        ty: TyId<'db>,
    ) -> Result<Type, String> {
        let ty = instance.normalized_ty(db, ty);
        if ty.is_never(db) {
            return Ok(Type::Unit);
        }
        if let Some((kind, target)) = ty.as_capability(db) {
            return match kind {
                CapabilityKind::View | CapabilityKind::Ref => self.lower(db, instance, target),
                // The modeled context cannot change, so a mutable borrow of it
                // is the context itself.
                CapabilityKind::Mut if is_evm(db, target) => self.lower(db, instance, target),
                CapabilityKind::Mut => Err(format!(
                    "`{}` is a mutable borrow, which the prover does not support yet",
                    ty.pretty_print(db)
                )),
            };
        }
        if let Some(prim) = ty.as_prim(db) {
            return self.primitive(db, instance, ty, prim);
        }
        if is_evm(db, ty) {
            return Ok(self.evm());
        }
        if let Some(def) = ty.adt_def(db) {
            let name = ty.pretty_print(db).to_string();
            let placeholder = self.table.push(TypeDef::Struct {
                name: name.clone(),
                fields: Vec::new(),
            });
            self.lowered.insert(ty, Type::Adt(placeholder));
            let def = match def.adt_ref(db) {
                AdtRef::Struct(item) => {
                    let names: Vec<String> = item
                        .hir_fields(db)
                        .data(db)
                        .iter()
                        .map(|field| match field.name {
                            Partial::Present(name) => name.data(db).to_string(),
                            Partial::Absent => "_".to_string(),
                        })
                        .collect();
                    let mut fields = Vec::new();
                    for (index, field_ty) in
                        instance.normalized_field_types(db, ty).iter().enumerate()
                    {
                        fields.push(Field {
                            name: names
                                .get(index)
                                .cloned()
                                .unwrap_or_else(|| index.to_string()),
                            ty: self.lower(db, instance, *field_ty)?,
                        });
                    }
                    TypeDef::Struct { name, fields }
                }
                AdtRef::Enum(item) => {
                    let mut variants = Vec::new();
                    for index in 0..def.fields(db).len() {
                        let variant_name = EnumVariant::new(item, index)
                            .name(db)
                            .unwrap_or("_")
                            .to_string();
                        let mut fields = Vec::new();
                        for field_ty in instance
                            .normalized_enum_variant_field_tys(db, ty, VariantIndex(index as u16))
                            .iter()
                        {
                            fields.push(self.lower(db, instance, *field_ty)?);
                        }
                        variants.push(Variant {
                            name: variant_name,
                            fields,
                        });
                    }
                    TypeDef::Enum { name, variants }
                }
            };
            self.table.defs[placeholder.0 as usize] = def;
            return Ok(Type::Adt(placeholder));
        }
        Err(format!(
            "values of type `{}` are not supported",
            ty.pretty_print(db)
        ))
    }

    /// The EVM context, as a struct of the values `Ctx` reads.
    pub fn evm(&mut self) -> Type {
        if let Some(found) = self.evm {
            return Type::Adt(found);
        }
        let id = self.table.push(TypeDef::Struct {
            name: "Evm".to_string(),
            fields: CONTEXT
                .iter()
                .map(|(_, name)| Field {
                    name: name.trim_start_matches("ctx.").to_string(),
                    ty: Type::uint(256),
                })
                .collect(),
        });
        self.evm = Some(id);
        Type::Adt(id)
    }

    fn primitive(
        &mut self,
        db: &'db dyn HirAnalysisDb,
        instance: SemanticInstance<'db>,
        ty: TyId<'db>,
        prim: PrimTy,
    ) -> Result<Type, String> {
        let word = self.word_bits;
        Ok(match prim {
            PrimTy::Bool => Type::Bool,
            PrimTy::U8 => Type::uint(8),
            PrimTy::U16 => Type::uint(16),
            PrimTy::U32 => Type::uint(32),
            PrimTy::U64 => Type::uint(64),
            PrimTy::U128 => Type::uint(128),
            PrimTy::U256 => Type::uint(256),
            PrimTy::Usize => Type::uint(word),
            PrimTy::I8 => Type::int(8),
            PrimTy::I16 => Type::int(16),
            PrimTy::I32 => Type::int(32),
            PrimTy::I64 => Type::int(64),
            PrimTy::I128 => Type::int(128),
            PrimTy::I256 => Type::int(256),
            PrimTy::Isize => Type::int(word),
            PrimTy::Tuple(0) => Type::Unit,
            PrimTy::Tuple(_) => {
                let mut items = Vec::new();
                for item in ty.generic_args(db).to_vec() {
                    items.push(self.lower(db, instance, item)?);
                }
                Type::Tuple(items)
            }
            PrimTy::Array => {
                let elem = *ty
                    .generic_args(db)
                    .first()
                    .ok_or_else(|| "an array type without an element type".to_string())?;
                let len = ty.array_len(db).ok_or_else(|| {
                    format!(
                        "the array type `{}` has no known length",
                        ty.pretty_print(db)
                    )
                })?;
                Type::Array {
                    elem: Box::new(self.lower(db, instance, elem)?),
                    len,
                }
            }
            PrimTy::View | PrimTy::BorrowRef => {
                let inner = *ty
                    .generic_args(db)
                    .first()
                    .ok_or_else(|| "a borrow without a target type".to_string())?;
                self.lower(db, instance, inner)?
            }
            PrimTy::BorrowMut => {
                return Err(format!(
                    "`{}` is a mutable borrow, which the prover does not support yet",
                    ty.pretty_print(db)
                ));
            }
            PrimTy::String | PrimTy::Ptr(_) => {
                return Err(format!(
                    "values of type `{}` are not supported",
                    ty.pretty_print(db)
                ));
            }
        })
    }
}
