//! Where a contract keeps its storage on the EVM, from the compiler's own
//! layout, so that a target's storage can be read back as the state the
//! prover reasons about: each scalar at its slot and byte lane, an enum's
//! tag, and a storage map's entries at `keccak256(key ++ slot)` for the
//! keys asked about.

use femp_vir::eval::Val;
use femp_vir::{Type, TypeDef, TypeTable};
use hir::core::semantic::{
    ContractLayoutEntryKind, ContractLayoutPathSegment, ContractLayoutValue,
};
use hir::hir_def::Contract;
use num_bigint::{BigInt, Sign};
use tiny_keccak::{Hasher, Keccak};

use crate::error::Unsupported;
use crate::session::Session;

/// A 256-bit word, big-endian.
pub type Word = [u8; 32];

#[derive(Debug, Clone)]
pub struct StorageLayout {
    leaves: Vec<Leaf>,
}

#[derive(Debug, Clone)]
struct Leaf {
    path: Vec<Step>,
    slot: Word,
    /// The bytes of the slot the value takes, from the low-order end.
    lane: Option<(u8, u8)>,
    kind: Kind,
}

#[derive(Debug, Clone, Copy)]
enum Step {
    Field(usize),
    /// A field of an enum's variant, which only that variant has.
    Variant(u16, usize),
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Scalar,
    Tag,
    Map,
}

impl Session {
    /// The storage layout of the contract `name` in the module at `path`.
    pub fn storage_layout(
        &self,
        path: &[String],
        name: &str,
    ) -> Result<StorageLayout, Unsupported> {
        let contract = self.contract_named(path, name).ok_or_else(|| {
            Unsupported::new(
                Default::default(),
                format!("the contract `{name}` was not found"),
            )
        })?;
        layout(self, contract)
    }

    pub(crate) fn contract_named(&self, path: &[String], name: &str) -> Option<Contract<'_>> {
        let root = crate::lower::ContractRoot {
            path: path.to_vec(),
            name: name.to_string(),
            invariants: Vec::new(),
            messages: Vec::new(),
        };
        self.find_contract(&root)
    }
}

fn layout(session: &Session, contract: Contract<'_>) -> Result<StorageLayout, Unsupported> {
    let db = &session.db;
    let unsupported = |message: String| Unsupported::new(Default::default(), message);
    let report = contract.layout_report(db).ok_or_else(|| {
        unsupported("the compiler laid out no storage for the contract".to_string())
    })?;
    let mut fields: Vec<_> = contract.fields(db).values().cloned().collect();
    fields.sort_by_key(|field| field.index);
    let mut leaves = Vec::new();
    for entry in &report.entries {
        let field = fields
            .iter()
            .position(|field| field.name == entry.path.field)
            .ok_or_else(|| unsupported("storage of no field".to_string()))?;
        let mut path = vec![Step::Field(field)];
        let mut variant = None;
        for segment in &entry.path.segments {
            match segment {
                ContractLayoutPathSegment::Member { index, .. } => {
                    path.push(match variant.take() {
                        Some(variant) => Step::Variant(variant, *index as usize),
                        None => Step::Field(*index as usize),
                    })
                }
                ContractLayoutPathSegment::TupleElement(index) => {
                    path.push(Step::Field(*index as usize));
                }
                ContractLayoutPathSegment::Variant { index, .. } => variant = Some(*index as u16),
                ContractLayoutPathSegment::EnumTag => {}
                ContractLayoutPathSegment::ArrayElement { .. } => {
                    return Err(unsupported(format!(
                        "reading `{}` back from storage, an array, is not supported yet",
                        entry.path.display(db)
                    )));
                }
            }
        }
        let ContractLayoutValue::Scalar(slot) = &entry.value else {
            return Err(unsupported(format!(
                "reading `{}` back from storage is not supported yet",
                entry.path.display(db)
            )));
        };
        let mut word = [0u8; 32];
        let bytes = slot.data(db).to_bytes_be();
        word[32 - bytes.len().min(32)..].copy_from_slice(&bytes[bytes.len().saturating_sub(32)..]);
        leaves.push(Leaf {
            path,
            slot: word,
            lane: entry.lane.map(|lane| (lane.byte_offset, lane.byte_width)),
            kind: match entry.kind {
                ContractLayoutEntryKind::InlineField => Kind::Scalar,
                ContractLayoutEntryKind::EnumTag => Kind::Tag,
                ContractLayoutEntryKind::Collection => Kind::Map,
            },
        });
    }
    // Tags first, so that a variant's fields go into the variant it has.
    leaves.sort_by_key(|leaf| !matches!(leaf.kind, Kind::Tag));
    Ok(StorageLayout { leaves })
}

impl StorageLayout {
    /// The slots to read to know the storage, with the entries of every map
    /// at each of `keys` of its key type.
    pub fn slots(&self, state: &Type, table: &TypeTable, keys: &[Val]) -> Vec<Word> {
        let mut slots = Vec::new();
        for leaf in &self.leaves {
            match leaf.kind {
                Kind::Scalar | Kind::Tag => slots.push(leaf.slot),
                Kind::Map => {
                    let Some(Type::Map { key, .. }) = type_at(state, table, &leaf.path) else {
                        continue;
                    };
                    for candidate in keys.iter().filter(|candidate| fits(candidate, &key, table)) {
                        slots.push(entry_slot(candidate, &leaf.slot));
                    }
                }
            }
        }
        slots.sort();
        slots.dedup();
        slots
    }

    /// The storage as the prover's state, from the words `read` gives for
    /// the slots `slots` named.
    pub fn decode(
        &self,
        state: &Type,
        table: &TypeTable,
        keys: &[Val],
        read: &dyn Fn(&Word) -> Word,
    ) -> Option<Val> {
        let mut value = Val::zero(state, table);
        for leaf in &self.leaves {
            let ty = type_at(state, table, &leaf.path)?;
            match leaf.kind {
                Kind::Scalar => {
                    let word = lane(read(&leaf.slot), leaf.lane);
                    *place(&mut value, &leaf.path)? = scalar(&word, &ty, table)?;
                }
                Kind::Tag => {
                    let tag = lane(read(&leaf.slot), leaf.lane);
                    let tag = u16::try_from(BigInt::from_bytes_be(Sign::Plus, &tag)).ok()?;
                    let Type::Adt(id) = &ty else {
                        return None;
                    };
                    let TypeDef::Enum { variants, .. } = table.get(*id) else {
                        return None;
                    };
                    let fields = variants
                        .get(tag as usize)?
                        .fields
                        .iter()
                        .map(|field| Val::zero(field, table))
                        .collect();
                    *place(&mut value, &leaf.path)? = Val::Adt {
                        variant: tag,
                        fields,
                    };
                }
                Kind::Map => {
                    let Type::Map { key, value: held } = &ty else {
                        return None;
                    };
                    let map = place(&mut value, &leaf.path)?;
                    for candidate in keys.iter().filter(|candidate| fits(candidate, key, table)) {
                        let word = read(&entry_slot(candidate, &leaf.slot));
                        map.set_entry(candidate.clone(), scalar(&word, held, table)?);
                    }
                }
            }
        }
        Some(value)
    }
}

fn type_at(state: &Type, table: &TypeTable, path: &[Step]) -> Option<Type> {
    let mut ty = state.clone();
    for step in path {
        ty = match (step, &ty) {
            (Step::Field(index), Type::Tuple(items)) => items.get(*index)?.clone(),
            (Step::Field(index), Type::Adt(id)) => match table.get(*id) {
                TypeDef::Struct { fields, .. } => fields.get(*index)?.ty.clone(),
                TypeDef::Enum { .. } => return None,
            },
            (Step::Variant(variant, index), Type::Adt(id)) => match table.get(*id) {
                TypeDef::Enum { variants, .. } => {
                    variants.get(*variant as usize)?.fields.get(*index)?.clone()
                }
                TypeDef::Struct { .. } => return None,
            },
            _ => return None,
        };
    }
    Some(ty)
}

/// The part of `value` at `path`, which for a variant's field exists only
/// when the enum has that variant.
fn place<'v>(value: &'v mut Val, path: &[Step]) -> Option<&'v mut Val> {
    let mut current = value;
    for step in path {
        current = match (step, current) {
            (Step::Field(index), Val::Tuple(items) | Val::Adt { fields: items, .. }) => {
                items.get_mut(*index)?
            }
            (
                Step::Variant(variant, index),
                Val::Adt {
                    variant: held,
                    fields,
                },
            ) if held == variant => fields.get_mut(*index)?,
            _ => return None,
        };
    }
    Some(current)
}

fn lane(word: Word, lane: Option<(u8, u8)>) -> Word {
    let Some((offset, width)) = lane else {
        return word;
    };
    let mut out = [0u8; 32];
    let (offset, width) = (offset as usize, width as usize);
    let end = 32 - offset;
    let start = end.saturating_sub(width);
    out[32 - (end - start)..].copy_from_slice(&word[start..end]);
    out
}

/// A scalar of `ty` held in the low bytes of `word`: an integer, which a
/// signed type sign-extends, a `bool`, or an `Address`.
fn scalar(word: &Word, ty: &Type, table: &TypeTable) -> Option<Val> {
    let unsigned = BigInt::from_bytes_be(Sign::Plus, word);
    Some(match ty {
        Type::Bool => Val::Bool(unsigned != BigInt::from(0)),
        Type::Int { bits, signed } => {
            let modulus = BigInt::from(1) << *bits;
            let low = unsigned % &modulus;
            if *signed && low >= (BigInt::from(1) << (*bits - 1)) {
                Val::Int(low - modulus)
            } else {
                Val::Int(low)
            }
        }
        Type::Adt(id) => match table.get(*id) {
            TypeDef::Struct { fields, .. } if fields.len() == 1 => Val::Adt {
                variant: 0,
                fields: vec![scalar(word, &fields[0].ty, table)?],
            },
            _ => return None,
        },
        _ => return None,
    })
}

fn fits(key: &Val, ty: &Type, table: &TypeTable) -> bool {
    match (key, ty) {
        (Val::Int(_), Type::Int { .. }) | (Val::Bool(_), Type::Bool) => true,
        (Val::Adt { fields, .. }, Type::Adt(id)) => match table.get(*id) {
            TypeDef::Struct {
                fields: declared, ..
            } => {
                declared.len() == fields.len()
                    && fields
                        .iter()
                        .zip(declared)
                        .all(|(field, declared)| fits(field, &declared.ty, table))
            }
            TypeDef::Enum { .. } => false,
        },
        (Val::Tuple(items), Type::Tuple(types)) => {
            items.len() == types.len()
                && items
                    .iter()
                    .zip(types)
                    .all(|(item, ty)| fits(item, ty, table))
        }
        _ => false,
    }
}

/// The slot of a map's entry: `keccak256` of the key's words, then the
/// map's own slot.
fn entry_slot(key: &Val, map: &Word) -> Word {
    let mut preimage = Vec::new();
    encode(key, &mut preimage);
    preimage.extend_from_slice(map);
    let mut hasher = Keccak::v256();
    hasher.update(&preimage);
    let mut out = [0u8; 32];
    hasher.finalize(&mut out);
    out
}

/// A key as the words `StorageKey` writes: one per scalar, two's complement.
fn encode(key: &Val, out: &mut Vec<u8>) {
    match key {
        Val::Int(number) => {
            let modulus = BigInt::from(1) << 256;
            let bits: BigInt = ((number % &modulus) + &modulus) % &modulus;
            let (_, bytes) = bits.to_bytes_be();
            let mut word = [0u8; 32];
            word[32 - bytes.len()..].copy_from_slice(&bytes);
            out.extend_from_slice(&word);
        }
        Val::Bool(flag) => {
            let mut word = [0u8; 32];
            word[31] = u8::from(*flag);
            out.extend_from_slice(&word);
        }
        Val::Adt { fields: items, .. } | Val::Tuple(items) => {
            for item in items {
                encode(item, out);
            }
        }
        _ => {}
    }
}
