//! EVM builds: every contract of an ingot, with its creation and runtime
//! bytecode, its ABI and its metadata.

mod abi;
mod metadata;

use std::collections::BTreeSet;

use femp_core::environment::OptLevel;
use hir::hir_def::HirIngot;

use crate::error::CompileError;
use crate::guard::{generate, opt_level};
use crate::names::file_name;
use crate::session::Session;

/// One contract as the compiler writes it, named after its artifact.
pub struct Contract {
    /// The file stem of the contract's artifacts.
    pub name: String,
    /// The creation code, as lowercase hex without a prefix.
    pub bytecode: String,
    /// The code that stays deployed, as lowercase hex.
    pub runtime: String,
    /// The JSON ABI; absent when the contract has no entry in it.
    pub abi: Option<String>,
    pub metadata: Option<String>,
}

pub struct EvmBuild {
    pub contracts: Vec<Contract>,
    pub warnings: Vec<String>,
}

const RUNTIME_LIMIT: usize = 24_576;
const INITCODE_LIMIT: usize = 49_152;

impl Session {
    pub fn build_evm(&self, optimize: OptLevel) -> Result<EvmBuild, CompileError> {
        let db = &self.db;
        let ingot = self.ingot();
        let root = ingot.root_mod(db);
        let exported: Vec<String> = mir::exported_funcs(db, root)
            .iter()
            .map(|func| {
                format!(
                    "Error: `#[unsafe(export)]` requires `--backend native` or `--backend wasm`, but `{}` is exported",
                    func.scope().pretty_path(db).unwrap_or_default()
                )
            })
            .collect();
        if !exported.is_empty() {
            return Err(CompileError::Rejected(exported.join("\n")));
        }

        let names = contract_names(self)?;
        if names.is_empty() {
            return Err(CompileError::Failed(
                "the ingot declares no contract to build".to_string(),
            ));
        }
        let level = opt_level(optimize);
        let bytecode = generate("Sonatina", || {
            codegen::emit_ingot_sonatina_bytecode(db, ingot, level, None)
        })?;
        let abis = mir::ingot_contracts(db, ingot);
        let metadata = metadata::render(db, ingot, &names, &level.to_string())
            .map_err(CompileError::Failed)?;

        let mut warnings = Vec::new();
        let mut contracts = Vec::new();
        for name in &names {
            let code = bytecode.get(name).ok_or_else(|| {
                CompileError::Failed(format!("Sonatina emitted no bytecode for `{name}`"))
            })?;
            warnings.extend(size_warnings(name, code.runtime.len(), code.deploy.len()));
            let abi = match abis
                .iter()
                .find(|declared| declared.name == *name)
                .and_then(|declared| declared.contract())
            {
                Some(contract) => {
                    let result = abi::generate_contract_abi(db, contract).map_err(|error| {
                        CompileError::Failed(format!("cannot generate the ABI of {name}: {error}"))
                    })?;
                    warnings.extend(result.warnings);
                    (result.entry_count > 0).then_some(result.json)
                }
                None => None,
            };
            contracts.push(Contract {
                name: file_name(name),
                bytecode: hex::encode(&code.deploy),
                runtime: hex::encode(&code.runtime),
                abi,
                metadata: metadata.get(name).cloned(),
            });
        }
        Ok(EvmBuild {
            contracts,
            warnings,
        })
    }
}

/// The `Sol` ABI type a contract's messages are encoded with.
pub(crate) fn sol_abi_ty<'db>(
    db: &'db driver::DriverDataBase,
    contract: hir::hir_def::Contract<'db>,
) -> Result<hir::analysis::ty::ty_def::TyId<'db>, String> {
    abi::resolve_sol_abi_ty(db, contract.scope())
}

// The objects the ingot's modules deploy: its contracts, and the root
// module's `main`.
fn contract_names(session: &Session) -> Result<Vec<String>, CompileError> {
    let db = &session.db;
    let mut names = BTreeSet::new();
    for &top_mod in session.ingot().all_modules(db) {
        let package = generate("MIR", || {
            mir::build_ingot_module_runtime_package(db, top_mod)
        })?;
        if let Some(package) = package {
            names.extend(
                package
                    .root_objects(db)
                    .iter()
                    .map(|object| object.name(db).clone()),
            );
        }
    }
    Ok(names.into_iter().collect())
}

fn size_warnings(name: &str, runtime: usize, initcode: usize) -> Vec<String> {
    [
        ("runtime code", runtime, RUNTIME_LIMIT, "EIP-170"),
        (
            "initcode (before constructor arguments)",
            initcode,
            INITCODE_LIMIT,
            "EIP-3860",
        ),
    ]
    .into_iter()
    .filter(|(_, size, limit, _)| size > limit)
    .map(|(kind, size, limit, eip)| {
        format!(
            "contract `{name}` has {size} bytes of {kind}, exceeding the {eip} limit of {limit} \
             bytes; deployment will fail on chains enforcing this limit"
        )
    })
    .collect()
}
