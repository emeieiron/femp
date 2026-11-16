//! The JSON ABI of a contract, as `fe build --emit abi` writes it. The
//! generator is the Fe CLI's own (Apache-2.0), kept unchanged so that the
//! files femp writes are the ones the compiler writes.

use std::collections::HashSet;

use common::ingot::IngotKind;
use driver::DriverDataBase;
use hir::{
    analysis::{
        name_resolution::{PathRes, resolve_path},
        ty::{
            abi_ty::{self, AbiComponent, AbiTypeDesc, ParsedFunctionSignature},
            adt_def::AdtRef,
            binder::Binder,
            corelib::{RuntimeBuiltinFuncKind, runtime_builtin_func_kind},
            normalize::normalize_from_assumptions,
            trait_def::TraitInstId,
            trait_resolution::{PredicateListId, TraitSolveCx},
            ty_check::{BodyOwner, check_body},
            ty_def::{BorrowKind, TyBase, TyData, TyId},
        },
    },
    hir_def::{
        Contract, FieldDefListId, Func, GenericParamOwner, IdentId, PathId, Struct, Trait,
        scope_graph::ScopeId,
    },
};
use serde::Serialize;
use tiny_keccak::{Hasher, Keccak};

#[derive(Serialize)]
pub struct AbiEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Vec<AbiParam>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<AbiParam>>,
    #[serde(rename = "stateMutability", skip_serializing_if = "Option::is_none")]
    pub state_mutability: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub anonymous: Option<bool>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct AbiParam {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<AbiParam>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NamedAbiParamDesc {
    name: String,
    indexed: Option<bool>,
    desc: AbiTypeDesc,
}

fn component_to_param(component: AbiComponent) -> AbiParam {
    AbiParam {
        name: component.name,
        ty: component.ty,
        indexed: None,
        components: component
            .components
            .map(|components| components.into_iter().map(component_to_param).collect()),
    }
}

impl NamedAbiParamDesc {
    fn into_param(self) -> AbiParam {
        AbiParam {
            name: self.name,
            ty: self.desc.abi_type,
            indexed: self.indexed,
            components: self
                .desc
                .components
                .map(|components| components.into_iter().map(component_to_param).collect()),
        }
    }
}

pub struct AbiResult {
    pub json: String,
    pub entry_count: usize,
    pub warnings: Vec<String>,
}

enum RecvArmAbiEmission {
    Emit(AbiEntry),
    Skip(String),
}

/// Generate an Ethereum-compatible JSON ABI for `contract`.
pub fn generate_contract_abi(
    db: &DriverDataBase,
    contract: Contract<'_>,
) -> Result<AbiResult, String> {
    let mut entries = Vec::new();
    let mut warnings = Vec::new();

    if let Some(init) = contract.init(db) {
        let inputs = init_params_to_abi_params(db, contract)?
            .into_iter()
            .map(NamedAbiParamDesc::into_param)
            .collect();
        let state_mutability = if init.is_payable(db) {
            "payable"
        } else {
            "nonpayable"
        };
        entries.push(AbiEntry {
            entry_type: "constructor".to_string(),
            name: None,
            inputs: Some(inputs),
            outputs: None,
            state_mutability: Some(state_mutability.to_string()),
            anonymous: None,
        });
    }

    let sol_ty = resolve_sol_abi_ty(db, contract.scope())?;
    for recv in contract.recv_views(db) {
        for arm_view in recv.arms(db) {
            match recv_arm_to_abi_entry(db, arm_view, sol_ty) {
                Ok(RecvArmAbiEmission::Emit(entry)) => entries.push(entry),
                Ok(RecvArmAbiEmission::Skip(warning)) => warnings.push(warning),
                Err(e) => return Err(e),
            }
        }
    }

    for struct_ in collect_contract_event_structs(db, contract, false) {
        entries.push(event_struct_to_abi_entry(db, struct_)?);
    }

    for struct_ in collect_contract_event_structs(db, contract, true) {
        let mut entry = event_struct_to_abi_entry(db, struct_)?;
        entry.entry_type = "error".to_string();
        entry.anonymous = None;
        for input in entry.inputs.iter_mut().flatten() {
            input.indexed = None;
        }
        // Solidity's built-in errors are implicit. Declaring them as custom
        // errors makes consumers such as ethers reject the entire ABI.
        if let Some([input]) = entry.inputs.as_deref()
            && matches!(
                (entry.name.as_deref(), input.ty.as_str()),
                (Some("Panic"), "uint256") | (Some("Error"), "string")
            )
        {
            continue;
        }
        entries.push(entry);
    }

    let entry_count = entries.len();
    let json = serde_json::to_string_pretty(&entries)
        .map_err(|e| format!("JSON serialization error: {e}"))?;
    Ok(AbiResult {
        json,
        entry_count,
        warnings,
    })
}

fn recv_arm_to_abi_entry(
    db: &DriverDataBase,
    arm_view: hir::semantic::RecvArmView<'_>,
    sol_ty: TyId<'_>,
) -> Result<RecvArmAbiEmission, String> {
    let arm = arm_view
        .arm(db)
        .ok_or_else(|| "missing recv arm during ABI generation".to_string())?;
    if arm_view.is_fallback(db) {
        return Ok(RecvArmAbiEmission::Emit(AbiEntry {
            entry_type: "fallback".to_string(),
            name: None,
            inputs: None,
            outputs: None,
            state_mutability: Some(if arm.is_payable(db) {
                "payable".to_string()
            } else {
                "nonpayable".to_string()
            }),
            anonymous: None,
        }));
    }

    let variant_ty = arm_view.variant_ty(db);
    let variant_struct = struct_from_ty(db, variant_ty).ok_or_else(|| {
        format!(
            "recv arm type `{}` is not a struct",
            variant_ty.pretty_print(db)
        )
    })?;
    let variant_name = variant_struct
        .name(db)
        .to_opt()
        .map_or_else(|| "<unknown>".to_string(), |name| name.data(db).to_string());
    if !variant_has_canonical_json_abi_shape(db, variant_struct) {
        return Ok(RecvArmAbiEmission::Skip(format!(
            "skipping recv arm `{variant_name}`: ABI shape is not compiler-known for manual `MsgVariant` impls; only `msg`-generated variants are emitted"
        )));
    }
    let abi_info = arm_view.abi_info(db, sol_ty);
    let Some(selector_signature) = abi_info.selector_signature.as_deref() else {
        return Ok(RecvArmAbiEmission::Skip(format!(
            "skipping recv arm `{variant_name}`: selector signature is unknown; \
             use `#[selector = sol(\"name(types)\")]` to include it in the ABI"
        )));
    };

    let input_descs = struct_ty_to_abi_param_descs(db, variant_struct, variant_ty, |_| None)?;
    // A tuple return is a parameter list (Solidity multi-value `returns`), so
    // each element becomes its own output, matching its runtime encoding.
    let output_tys = match abi_info.ret_ty {
        Some(ret_ty) if ret_ty.is_tuple(db) => ret_ty.field_types(db),
        Some(ret_ty) => vec![ret_ty],
        None => Vec::new(),
    };
    let outputs = output_tys
        .into_iter()
        .map(|ty| {
            Ok(NamedAbiParamDesc {
                name: String::new(),
                indexed: None,
                desc: semantic_ty_to_abi_desc(db, ty)?,
            }
            .into_param())
        })
        .collect::<Result<Vec<_>, String>>()?;

    let selector_value = abi_info.selector_value.ok_or_else(|| {
        format!(
            "cannot emit JSON ABI for `{selector_signature}`: selector value could not be resolved"
        )
    })?;
    let parsed_signature = parse_function_signature(selector_signature)?;
    ensure_selector_matches_signature(
        selector_signature,
        &parsed_signature,
        &input_descs,
        selector_value,
    )?;

    Ok(RecvArmAbiEmission::Emit(AbiEntry {
        entry_type: "function".to_string(),
        name: Some(parsed_signature.name),
        inputs: Some(
            input_descs
                .into_iter()
                .map(NamedAbiParamDesc::into_param)
                .collect(),
        ),
        outputs: Some(outputs),
        state_mutability: Some(derive_state_mutability(db, arm_view)),
        anonymous: None,
    }))
}

fn event_struct_to_abi_entry(db: &DriverDataBase, struct_: Struct<'_>) -> Result<AbiEntry, String> {
    let event_name = struct_
        .name(db)
        .to_opt()
        .map(|name| name.data(db).to_string())
        .ok_or_else(|| "event struct is missing a name".to_string())?;
    let field_tys: Vec<_> = struct_
        .field_tys(db)
        .into_iter()
        .map(|ty| ty.instantiate_identity())
        .collect();
    let inputs = named_field_param_descs(db, struct_.hir_fields(db), &field_tys, |field| {
        Some(field.is_event_indexed)
    })?
    .into_iter()
    .map(NamedAbiParamDesc::into_param)
    .collect();

    Ok(AbiEntry {
        entry_type: "event".to_string(),
        name: Some(event_name),
        inputs: Some(inputs),
        outputs: None,
        state_mutability: None,
        anonymous: Some(false),
    })
}

fn collect_contract_event_structs<'db>(
    db: &'db DriverDataBase,
    contract: hir::hir_def::Contract<'db>,
    errors: bool,
) -> Vec<Struct<'db>> {
    let mut events = Vec::new();
    let mut seen = HashSet::new();
    let mut visited_funcs = HashSet::new();
    let emit_traits = resolve_event_emit_traits(db, contract.scope());
    let error_func = hir::analysis::ty::corelib::resolve_lib_func_path(
        db,
        contract.scope(),
        "std::evm::effects::revert_error",
    );

    if contract.init(db).is_some() {
        let (_, typed_body) = hir::analysis::ty::ty_check::check_contract_init_body(db, contract);
        collect_typed_body_event_structs(
            db,
            typed_body,
            &emit_traits,
            errors,
            error_func,
            &mut events,
            &mut seen,
            &mut visited_funcs,
        );
    }

    for recv in contract.recv_views(db) {
        for arm in recv.arms(db) {
            let (_, typed_body) = hir::analysis::ty::ty_check::check_contract_recv_arm_body(
                db,
                contract,
                recv.index(db),
                arm.index(db),
            );
            collect_typed_body_event_structs(
                db,
                typed_body,
                &emit_traits,
                errors,
                error_func,
                &mut events,
                &mut seen,
                &mut visited_funcs,
            );
        }
    }

    events
}

#[allow(clippy::too_many_arguments)]
fn collect_typed_body_event_structs<'db>(
    db: &'db DriverDataBase,
    typed_body: &hir::analysis::ty::ty_check::TypedBody<'db>,
    emit_traits: &EventEmitTraits<'db>,
    errors: bool,
    error_func: Option<Func<'db>>,
    out: &mut Vec<Struct<'db>>,
    seen: &mut HashSet<Struct<'db>>,
    visited_funcs: &mut HashSet<VisitedFuncBody<'db>>,
) {
    let Some(body) = typed_body.body() else {
        return;
    };

    for (expr_id, partial_expr) in body.exprs(db).iter() {
        let hir::hir_def::Partial::Present(_) = partial_expr else {
            continue;
        };
        if errors {
            if let Some(struct_) = reverted_error_struct(db, typed_body, body, expr_id, error_func)
                && seen.insert(struct_)
            {
                out.push(struct_);
            }
        } else if let Some(struct_) =
            emitted_event_struct(db, typed_body, body, expr_id, emit_traits)
        {
            push_event_struct(db, out, seen, struct_);
        }

        // Operators also have semantic callees, even without call syntax.
        if let Some(callable) = typed_body.callable_expr(expr_id)
            && let hir::hir_def::CallableDef::Func(func) = callable.callable_def
        {
            let mut target = VisitedFuncBody::from_callable(func, callable);
            if let Some(method) = callable.selected_method(
                db,
                TraitSolveCx::new(db, body.scope()).with_assumptions(typed_body.assumptions()),
            ) && let Some(impl_func) = method.body()
                && let Ok(body_args) = method.complete_body_args(db, callable.generic_args())
            {
                target.func = impl_func;
                target.generic_args = body_args.into_values();
            }
            // The body the call runs: a `mut?` function's at the call's mode.
            let owner = BodyOwner::func_at(db, target.func, callable.mode());
            target.mode = owner.mode();
            if target.func.body(db).is_none() || !visited_funcs.insert(target.clone()) {
                continue;
            }
            let (_, func_typed_body) = check_body(db, owner);
            let func_typed_body =
                instantiate_callable_typed_body(db, func_typed_body.clone(), &target);
            collect_typed_body_event_structs(
                db,
                &func_typed_body,
                emit_traits,
                errors,
                error_func,
                out,
                seen,
                visited_funcs,
            );
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct VisitedFuncBody<'db> {
    func: Func<'db>,
    /// The mode of the body of `func`, a `mut?` function, the call runs.
    mode: Option<BorrowKind>,
    generic_args: Vec<TyId<'db>>,
    trait_inst: Option<TraitInstId<'db>>,
}

impl<'db> VisitedFuncBody<'db> {
    fn from_callable(
        func: Func<'db>,
        callable: &hir::analysis::ty::ty_check::Callable<'db>,
    ) -> Self {
        Self {
            func,
            mode: callable.mode(),
            generic_args: callable.generic_args().to_vec(),
            trait_inst: callable.trait_inst(),
        }
    }
}

fn instantiate_callable_typed_body<'db>(
    db: &'db DriverDataBase,
    typed_body: hir::analysis::ty::ty_check::TypedBody<'db>,
    target: &VisitedFuncBody<'db>,
) -> hir::analysis::ty::ty_check::TypedBody<'db> {
    let mut typed_body = Binder::bind(GenericParamOwner::Func(target.func), typed_body)
        .instantiate(db, &target.generic_args);
    if let Some(trait_inst) = target.trait_inst {
        typed_body = normalize_from_assumptions(
            db,
            typed_body,
            target.func.scope(),
            PredicateListId::new(db, vec![trait_inst]),
        );
    }
    typed_body
}

struct EventEmitTraits<'db> {
    log_trait: Option<Trait<'db>>,
    event_trait: Option<Trait<'db>>,
}

fn resolve_event_emit_traits<'db>(
    db: &'db DriverDataBase,
    scope: ScopeId<'db>,
) -> EventEmitTraits<'db> {
    EventEmitTraits {
        log_trait: resolve_lib_trait_path(db, scope, "std::evm::effects::Log"),
        event_trait: resolve_lib_trait_path(db, scope, "std::evm::event::Event"),
    }
}

fn emitted_event_struct<'db>(
    db: &'db DriverDataBase,
    typed_body: &hir::analysis::ty::ty_check::TypedBody<'db>,
    body: hir::hir_def::Body<'db>,
    expr_id: hir::hir_def::ExprId,
    emit_traits: &EventEmitTraits<'db>,
) -> Option<Struct<'db>> {
    let hir::hir_def::Partial::Present(hir::hir_def::Expr::MethodCall(
        receiver,
        method_name,
        _,
        args,
    )) = expr_id.data(db, body)
    else {
        return None;
    };
    let method_name = method_name.to_opt()?;
    if method_name.data(db) != "emit" {
        return None;
    }

    let callable = typed_body.callable_expr(expr_id)?;
    let hir::hir_def::CallableDef::Func(func) = callable.callable_def else {
        return None;
    };
    if func.name(db).to_opt()? != method_name {
        return None;
    }

    let trait_def = callable.trait_inst()?.def(db);
    if emit_traits
        .log_trait
        .is_some_and(|log_trait| trait_def == log_trait)
    {
        let event_expr = args.first()?.expr;
        let struct_ = struct_from_ty(db, typed_body.expr_ty(db, event_expr))?;
        return is_event_struct(db, struct_).then_some(struct_);
    }
    if emit_traits
        .event_trait
        .is_some_and(|event_trait| trait_def == event_trait)
    {
        let struct_ = struct_from_ty(db, typed_body.expr_ty(db, *receiver))?;
        return is_event_struct(db, struct_).then_some(struct_);
    }

    None
}

fn reverted_error_struct<'db>(
    db: &'db DriverDataBase,
    typed_body: &hir::analysis::ty::ty_check::TypedBody<'db>,
    body: hir::hir_def::Body<'db>,
    expr_id: hir::hir_def::ExprId,
    error_func: Option<Func<'db>>,
) -> Option<Struct<'db>> {
    let hir::hir_def::Partial::Present(hir::hir_def::Expr::Call(_, args)) = expr_id.data(db, body)
    else {
        return None;
    };
    let callable = typed_body.callable_expr(expr_id)?;
    let hir::hir_def::CallableDef::Func(func) = callable.callable_def else {
        return None;
    };
    if Some(func) != error_func
        && runtime_builtin_func_kind(db, func) != Some(RuntimeBuiltinFuncKind::PanicWithValue)
    {
        return None;
    }
    let struct_ = struct_from_ty(db, typed_body.expr_ty(db, args.first()?.expr))?;
    matches!(
        hir::span::struct_ast(db, struct_),
        hir::span::HirOrigin::Desugared(hir::span::DesugaredOrigin::Error(_))
    )
    .then_some(struct_)
}

fn push_event_struct<'db>(
    db: &'db DriverDataBase,
    out: &mut Vec<Struct<'db>>,
    seen: &mut HashSet<Struct<'db>>,
    struct_: Struct<'db>,
) {
    if !seen.insert(struct_) || !is_event_struct(db, struct_) {
        return;
    }
    out.push(struct_);
}

fn is_event_struct(db: &DriverDataBase, struct_: Struct<'_>) -> bool {
    matches!(
        hir::span::struct_ast(db, struct_),
        hir::span::HirOrigin::Desugared(hir::span::DesugaredOrigin::Event(_))
    )
}

fn variant_has_canonical_json_abi_shape(db: &DriverDataBase, struct_: Struct<'_>) -> bool {
    matches!(
        hir::span::struct_ast(db, struct_),
        hir::span::HirOrigin::Desugared(hir::span::DesugaredOrigin::Msg(_))
    )
}

fn resolve_lib_trait_path<'db>(
    db: &'db DriverDataBase,
    scope: ScopeId<'db>,
    path: &str,
) -> Option<Trait<'db>> {
    let mut segments = path.split("::");
    let root = segments.next()?;

    let ingot = scope.ingot(db);
    let mut path = if (ingot.kind(db) == IngotKind::Std && root == "std")
        || (ingot.kind(db) == IngotKind::Core && root == "core")
    {
        PathId::from_ident(db, IdentId::make_ingot(db))
    } else {
        PathId::from_str(db, root)
    };

    for segment in segments {
        path = path.push_str(db, segment);
    }

    let assumptions = PredicateListId::empty_list(db);
    match resolve_path(db, path, scope, assumptions, true).ok()? {
        PathRes::Trait(inst) => Some(inst.def(db)),
        _ => None,
    }
}

fn init_params_to_abi_params(
    db: &DriverDataBase,
    contract: hir::hir_def::Contract<'_>,
) -> Result<Vec<NamedAbiParamDesc>, String> {
    let Some(init) = contract.init(db) else {
        return Ok(Vec::new());
    };

    let params: Vec<_> = init
        .params(db)
        .data(db)
        .iter()
        .filter(|param| !param.is_self_param(db))
        .collect();
    let param_tys = contract.init_args_ty(db).field_types(db);

    if params.len() != param_tys.len() {
        return Err(format!(
            "constructor parameter count mismatch: {} names vs {} semantic types",
            params.len(),
            param_tys.len()
        ));
    }

    params
        .into_iter()
        .zip(param_tys)
        .map(|(param, ty)| {
            let name = param
                .name()
                .map(|ident| ident.data(db).to_string())
                .unwrap_or_default();
            Ok(NamedAbiParamDesc {
                name,
                indexed: None,
                desc: semantic_ty_to_abi_desc(db, ty)?,
            })
        })
        .collect()
}

fn struct_ty_to_abi_param_descs(
    db: &DriverDataBase,
    struct_: Struct<'_>,
    ty: TyId<'_>,
    indexed: impl Fn(&hir::hir_def::FieldDef<'_>) -> Option<bool>,
) -> Result<Vec<NamedAbiParamDesc>, String> {
    let field_tys = ty.field_types(db);
    named_field_param_descs(db, struct_.hir_fields(db), &field_tys, indexed)
}

fn named_field_param_descs(
    db: &DriverDataBase,
    fields: FieldDefListId<'_>,
    field_tys: &[TyId<'_>],
    indexed: impl Fn(&hir::hir_def::FieldDef<'_>) -> Option<bool>,
) -> Result<Vec<NamedAbiParamDesc>, String> {
    let hir_fields = fields.data(db);
    if hir_fields.len() != field_tys.len() {
        return Err(format!(
            "field count mismatch: {} HIR fields vs {} semantic fields",
            hir_fields.len(),
            field_tys.len()
        ));
    }

    hir_fields
        .iter()
        .zip(field_tys.iter().copied())
        .map(|(field, ty)| {
            let name = field
                .name
                .to_opt()
                .map(|ident| ident.data(db).to_string())
                .unwrap_or_default();
            Ok(NamedAbiParamDesc {
                name,
                indexed: indexed(field),
                desc: semantic_ty_to_abi_desc(db, ty)?,
            })
        })
        .collect()
}

fn semantic_ty_to_abi_desc(db: &DriverDataBase, ty: TyId<'_>) -> Result<AbiTypeDesc, String> {
    abi_ty::semantic_ty_to_abi_desc(db, ty).map_err(|err| err.to_string())
}

fn struct_from_ty<'db>(db: &'db DriverDataBase, ty: TyId<'db>) -> Option<Struct<'db>> {
    if let Some((_, inner)) = ty.as_capability(db) {
        return struct_from_ty(db, inner);
    }
    match ty.base_ty(db).data(db) {
        TyData::TyBase(TyBase::Adt(adt)) => match adt.adt_ref(db) {
            AdtRef::Struct(struct_) => Some(struct_),
            AdtRef::Enum(_) => None,
        },
        TyData::QualifiedTy(trait_inst) => struct_from_ty(db, trait_inst.self_ty(db)),
        _ => None,
    }
}

fn parse_function_signature(signature: &str) -> Result<ParsedFunctionSignature, String> {
    abi_ty::parse_function_signature(signature)
        .map_err(|err| format!("cannot emit JSON ABI for `{signature}`: {err}"))
}

fn ensure_selector_matches_signature(
    source_signature: &str,
    parsed_signature: &ParsedFunctionSignature,
    inputs: &[NamedAbiParamDesc],
    actual_selector: u32,
) -> Result<(), String> {
    if parsed_signature.arg_types.len() != inputs.len() {
        return Err(format!(
            "cannot emit JSON ABI for `{source_signature}`: selector arity {} does not match semantic arity {}",
            parsed_signature.arg_types.len(),
            inputs.len()
        ));
    }

    for (selector_ty, input) in parsed_signature.arg_types.iter().zip(inputs) {
        if selector_ty != &input.desc.canonical_type {
            let mut message = format!(
                "cannot emit JSON ABI for `{source_signature}`: selector argument type `{selector_ty}` does not match semantic ABI type `{}`",
                input.desc.canonical_type
            );
            if let Some(suggestion) = abi_ty::suggested_fe_type_for_sol_type(selector_ty) {
                message.push_str(&format!(
                    "; `{suggestion}` decodes and encodes Solidity `{selector_ty}`"
                ));
            }
            return Err(message);
        }
    }

    let canonical_signature = canonical_function_signature(&parsed_signature.name, inputs);
    let expected_selector = selector_for_signature(&canonical_signature);
    if actual_selector != expected_selector {
        return Err(format!(
            "cannot emit JSON ABI for `{source_signature}`: non-canonical selector 0x{actual_selector:08x} does not match canonical signature `{canonical_signature}` (0x{expected_selector:08x})"
        ));
    }
    Ok(())
}

fn canonical_function_signature(fn_name: &str, inputs: &[NamedAbiParamDesc]) -> String {
    let mut signature = String::new();
    signature.push_str(fn_name);
    signature.push('(');
    for (idx, input) in inputs.iter().enumerate() {
        if idx > 0 {
            signature.push(',');
        }
        signature.push_str(&input.desc.canonical_type);
    }
    signature.push(')');
    signature
}

fn selector_for_signature(signature: &str) -> u32 {
    let mut hasher = Keccak::v256();
    let mut output = [0u8; 32];
    hasher.update(signature.as_bytes());
    hasher.finalize(&mut output);
    u32::from_be_bytes([output[0], output[1], output[2], output[3]])
}

pub(crate) fn resolve_sol_abi_ty<'db>(
    db: &'db DriverDataBase,
    scope: ScopeId<'db>,
) -> Result<TyId<'db>, String> {
    let ingot = scope.ingot(db);
    let std_root = if ingot.kind(db) == IngotKind::Std {
        IdentId::make_ingot(db)
    } else {
        IdentId::new(db, "std".to_string())
    };

    let sol_path = PathId::from_ident(db, std_root)
        .push_ident(db, IdentId::new(db, "abi".to_string()))
        .push_ident(db, IdentId::new(db, "Sol".to_string()));

    let assumptions = PredicateListId::empty_list(db);
    match resolve_path(db, sol_path, scope, assumptions, false) {
        Ok(PathRes::Ty(ty) | PathRes::TyAlias(_, ty)) => Ok(ty),
        Ok(other) => Err(format!(
            "expected `std::abi::Sol` to resolve to a type, got `{other:?}`"
        )),
        Err(err) => Err(format!("failed to resolve `std::abi::Sol`: {err:?}")),
    }
}

/// Derive ABI state mutability from the effective recv-arm effect requirement set.
fn derive_state_mutability(
    db: &DriverDataBase,
    arm_view: hir::semantic::RecvArmView<'_>,
) -> String {
    let arm = arm_view
        .arm(db)
        .expect("recv arm should exist during ABI generation");
    if arm.is_payable(db) {
        return "payable".to_string();
    }

    let effects = arm_view.effective_effect_requirements(db);
    if effects.is_empty() {
        "pure".to_string()
    } else if effects.iter().any(|effect| effect.is_mut) {
        "nonpayable".to_string()
    } else {
        "view".to_string()
    }
}
