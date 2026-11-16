//! What the prover models of the EVM a contract runs on. The block and
//! transaction context is one value of `std::evm::Evm`: a handler receives
//! it, passes it to whatever reads the context, and `Ctx` reads its fields.
//! Reverting ends the transaction, and events are not modeled.

use common::ingot::IngotKind;
use driver::DriverDataBase;
use hir::analysis::ty::adt_def::AdtRef;
use hir::analysis::ty::ty_def::TyId;
use hir::hir_def::{Func, ItemKind, Partial};

/// The context the modeled `Evm` value holds: the extern in `std::evm::ops`
/// that reads each field, and the name the field has in reports.
pub const CONTEXT: [(&str, &str); 14] = [
    ("timestamp", "ctx.timestamp"),
    ("number", "ctx.block_number"),
    ("caller", "ctx.caller"),
    ("callvalue", "ctx.value"),
    ("origin", "ctx.origin"),
    ("address", "ctx.address"),
    ("coinbase", "ctx.coinbase"),
    ("gasprice", "ctx.gasprice"),
    ("prevrandao", "ctx.prevrandao"),
    ("gaslimit", "ctx.gaslimit"),
    ("chainid", "ctx.chainid"),
    ("basefee", "ctx.basefee"),
    ("selfbalance", "ctx.selfbalance"),
    ("blobbasefee", "ctx.blobbasefee"),
];

/// The field of the value a call sends, which is zero unless the handler
/// is `#[payable]`.
pub const VALUE: usize = 3;

/// Reads that change while a transaction runs or depend on other accounts.
/// Each read is any value, which proves more than the EVM guarantees.
const UNCONSTRAINED: [&str; 6] = [
    "gas",
    "balance",
    "blockhash",
    "blobhash",
    "extcodesize",
    "extcodehash",
];

/// How a library function is modeled instead of lowered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    /// Ends the transaction, undoing what it did.
    Revert,
    /// Emits an event, which leaves storage alone.
    Emit,
    /// Reads a field of the context.
    Context(usize),
    /// Reads something the model leaves open.
    Unconstrained,
}

pub fn model(db: &DriverDataBase, func: Func<'_>) -> Option<Model> {
    let path = std_path(db, func)?;
    match path.as_str() {
        "evm::effects::revert"
        | "evm::effects::revert_error"
        | "evm::effects::revert_bytes"
        | "evm::effects::revert_empty" => Some(Model::Revert),
        "evm::effects::Log::emit" => Some(Model::Emit),
        _ => {
            let name = path.strip_prefix("evm::ops::")?;
            if let Some(index) = CONTEXT.iter().position(|(op, _)| *op == name) {
                Some(Model::Context(index))
            } else {
                UNCONSTRAINED
                    .contains(&name)
                    .then_some(Model::Unconstrained)
            }
        }
    }
}

/// The path of a function of the standard library inside it, with the trait
/// or type of its `impl` before its name.
fn std_path(db: &DriverDataBase, func: Func<'_>) -> Option<String> {
    if func.top_mod(db).ingot(db).kind(db) != IngotKind::Std {
        return None;
    }
    let Partial::Present(name) = func.name(db) else {
        return None;
    };
    let name = name.data(db).to_string();
    let module = func.top_mod(db).scope().pretty_path(db).unwrap_or_default();
    let module = module.split_once("::").map_or("", |(_, rest)| rest);
    let owner = match func.scope().parent_item(db) {
        Some(ItemKind::Trait(item)) => match item.name(db) {
            Partial::Present(owner) => Some(owner.data(db).to_string()),
            Partial::Absent => None,
        },
        _ => None,
    };
    Some(
        [module.to_string()]
            .into_iter()
            .filter(|segment| !segment.is_empty())
            .chain(owner)
            .chain(std::iter::once(name))
            .collect::<Vec<_>>()
            .join("::"),
    )
}

/// Whether `ty` is `std::evm::Evm`, the capability a contract's code runs
/// with.
pub fn is_evm(db: &dyn hir::analysis::HirAnalysisDb, ty: TyId<'_>) -> bool {
    let Some(def) = ty.adt_def(db) else {
        return false;
    };
    let AdtRef::Struct(item) = def.adt_ref(db) else {
        return false;
    };
    item.top_mod(db).ingot(db).kind(db) == IngotKind::Std
        && matches!(item.name(db), Partial::Present(name) if name.data(db) == "Evm")
}
