//! Finds the claims of a module by reading its Fe sources: `#[property]`
//! and `#[differential]` functions, `#[machine]` structs with their
//! `#[init]`, `#[command]`, `#[invariant]` and `#[observe]` methods,
//! `#[spec]` functions, and free `#[invariant]` functions of a contract's
//! storage. Attribute arguments carry the generator
//! specifications and limits, so a claim is declared in one place.

mod contracts;
mod lex;
mod parse;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use femp_core::ingot::SOURCE_DIR;
use femp_core::project::Module;
use femp_core::property::{
    Command, Effect, Location, Machine, Method, Observation, Param, Property, PropertyKind, Suite,
    Ty,
};
use femp_core::source_tree::ModuleTree;

use crate::claims::{Claims, Contract, MessageClaim, MessageKind, StorageClaim, StorageInvariant};
use contracts::{Candidate, PendingSpec};
use parse::{Attr, RawContract, RawFn, RawModule, RawStruct};

pub const DEFAULT_STEPS: &str = "1..=16";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryError {
    pub location: Location,
    pub message: String,
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.location, self.message)
    }
}

/// Reads every source of `module` and returns its claims, or every problem
/// found in their declarations.
pub fn discover(module: &Module) -> Result<Claims, Vec<DiscoveryError>> {
    let source_dir = module.dir.join(SOURCE_DIR);
    let tree = ModuleTree::read(&source_dir).map_err(|error| {
        vec![DiscoveryError {
            location: Location {
                file: source_dir.clone(),
                line: 0,
            },
            message: error.to_string(),
        }]
    })?;
    let mut sources = Vec::new();
    collect(&tree, Vec::new(), &mut sources);

    let mut builder = Builder {
        shared: module.is_shared(),
        errors: Vec::new(),
        suite: Suite {
            module: module.name.clone(),
            ingot: module.ingot.name.clone(),
            ..Suite::default()
        },
        specs: Vec::new(),
        candidates: Vec::new(),
        contracts: Vec::new(),
        deployed: Vec::new(),
        storage_invariants: Vec::new(),
        message_claims: Vec::new(),
    };
    for (path, file, source) in sources {
        let file = authored(module, &file);
        for raw in parse::parse_module(&lex::tokenize(source), path) {
            builder.module(&file, &raw);
        }
    }
    builder.resolve_specs();
    let storage = builder.resolve_storage();
    builder.check_names(&storage);
    if builder.errors.is_empty() {
        Ok(Claims {
            suite: builder.suite,
            contracts: builder.contracts,
            storage,
        })
    } else {
        Err(builder.errors)
    }
}

fn collect<'a>(
    tree: &'a ModuleTree,
    path: Vec<String>,
    out: &mut Vec<(Vec<String>, PathBuf, &'a str)>,
) {
    if let Some(file) = &tree.file {
        out.push((path.clone(), file.clone(), &tree.source));
    }
    for (name, child) in &tree.children {
        let mut child_path = path.clone();
        child_path.push(name.clone());
        collect(child, child_path, out);
    }
}

// Staged copies of a module point messages at the files the user wrote.
fn authored(module: &Module, file: &Path) -> PathBuf {
    file.strip_prefix(&module.dir)
        .ok()
        .map(|relative| module.source_dir.join(relative))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| file.to_path_buf())
}

struct Builder {
    shared: bool,
    errors: Vec<DiscoveryError>,
    suite: Suite,
    specs: Vec<PendingSpec>,
    /// Every function and method a spec may name as its target.
    candidates: Vec<Candidate>,
    contracts: Vec<Contract>,
    /// Every `contract` declared, with its module path and file.
    deployed: Vec<(RawContract, Vec<String>, PathBuf)>,
    /// Free `#[invariant]` functions, with the contract they name if any.
    storage_invariants: Vec<(StorageInvariant, Option<String>)>,
    /// `#[ensures]` and `#[reverts]` functions, likewise.
    message_claims: Vec<(MessageClaim, Option<String>)>,
}

impl Builder {
    fn error(&mut self, file: &Path, line: usize, message: impl Into<String>) {
        self.errors.push(DiscoveryError {
            location: Location {
                file: file.to_path_buf(),
                line,
            },
            message: message.into(),
        });
    }

    fn module(&mut self, file: &Path, raw: &RawModule) {
        self.note_candidates(file, raw);
        for contract in &raw.contracts {
            self.deployed
                .push((contract.clone(), raw.path.clone(), file.to_path_buf()));
        }
        for function in &raw.fns {
            if has(&function.attrs, "invariant") {
                self.storage_invariant(file, &raw.path, function);
                continue;
            }
            if let Some(kind) = message_kind(&function.attrs) {
                self.message_claim(file, &raw.path, function, kind);
                continue;
            }
            for attr in &function.attrs {
                if is_method_marker(&attr.name) {
                    self.error(
                        file,
                        attr.line,
                        format!(
                            "`#[{}]` belongs on a method of a `#[machine]` struct",
                            attr.name
                        ),
                    );
                }
            }
            if has(&function.attrs, "spec") {
                self.pending_spec(file, raw, function);
                continue;
            }
            if let Some(kind) = self.property_kind(file, function) {
                if raw.conditional {
                    self.error(
                        file,
                        function.line,
                        "properties cannot live in a module compiled for some targets only",
                    );
                }
                if let Some(property) = self.property(file, &raw.path, function, kind) {
                    self.suite.properties.push(property);
                }
            }
        }
        for method in raw.impls.iter().flat_map(|item| &item.methods) {
            if let Some(attr) = method.attrs.iter().find(|attr| attr.name == "spec") {
                self.error(
                    file,
                    attr.line,
                    "`#[spec]` belongs on a free function, not a method",
                );
            }
        }
        let machines: BTreeSet<&str> = raw
            .structs
            .iter()
            .filter(|item| has(&item.attrs, "machine"))
            .map(|item| item.name.as_str())
            .collect();
        for item in raw
            .impls
            .iter()
            .filter(|item| !machines.contains(item.type_name.as_str()))
        {
            for method in &item.methods {
                if let Some(attr) = method
                    .attrs
                    .iter()
                    .find(|attr| is_method_marker(&attr.name))
                {
                    self.error(
                        file,
                        attr.line,
                        format!(
                            "`#[{}]` is on `{}`, which is not a `#[machine]` struct",
                            attr.name, item.type_name
                        ),
                    );
                }
            }
        }
        for item in raw
            .structs
            .iter()
            .filter(|item| has(&item.attrs, "machine"))
        {
            if raw.conditional {
                self.error(
                    file,
                    item.line,
                    "machines cannot live in a module compiled for some targets only",
                );
            }
            let methods: Vec<&RawFn> = raw
                .impls
                .iter()
                .filter(|block| block.type_name == item.name)
                .flat_map(|block| &block.methods)
                .collect();
            if let Some(machine) = self.machine(file, &raw.path, item, &methods) {
                self.suite.machines.push(machine);
            }
        }
    }

    fn property_kind(&mut self, file: &Path, function: &RawFn) -> Option<PropertyKind> {
        let property = has(&function.attrs, "property");
        let differential = has(&function.attrs, "differential");
        match (property, differential) {
            (true, true) => {
                self.error(
                    file,
                    function.line,
                    "use either `#[property]` or `#[differential]`, not both",
                );
                None
            }
            (true, false) => Some(PropertyKind::Holds),
            (false, true) => Some(PropertyKind::Agrees),
            (false, false) => None,
        }
    }

    fn property(
        &mut self,
        file: &Path,
        path: &[String],
        function: &RawFn,
        kind: PropertyKind,
    ) -> Option<Property> {
        let marker = match kind {
            PropertyKind::Holds => "property",
            PropertyKind::Agrees => "differential",
            PropertyKind::Spec => "spec",
        };
        let options = self.options(file, &function.attrs, marker, &["cases"]);
        let cases = self.number(file, function.line, &options, "cases");
        self.entry_shape(file, function, marker);
        if function.has_self {
            self.error(
                file,
                function.line,
                format!("a `#[{marker}]` function cannot take `self`"),
            );
        }
        let params = self.params(file, function)?;
        let result = match (kind, function.result.as_deref().map(Ty::parse)) {
            (PropertyKind::Holds, Some(Some(Ty::Bool))) => Ty::Bool,
            (PropertyKind::Agrees | PropertyKind::Spec, Some(Some(ty))) => ty,
            (PropertyKind::Holds, _) => {
                self.error(file, function.line, "a `#[property]` must return `bool`");
                return None;
            }
            (PropertyKind::Agrees | PropertyKind::Spec, _) => {
                self.error(
                    file,
                    function.line,
                    "a `#[differential]` must return `bool` or a fixed-width integer",
                );
                return None;
            }
        };
        Some(Property {
            name: function.name.clone(),
            kind,
            path: path.to_vec(),
            params,
            result,
            cases,
            location: Location {
                file: file.to_path_buf(),
                line: function.line,
            },
        })
    }

    fn machine(
        &mut self,
        file: &Path,
        path: &[String],
        item: &RawStruct,
        methods: &[&RawFn],
    ) -> Option<Machine> {
        let options = self.options(file, &item.attrs, "machine", &["steps", "runs", "clock"]);
        if !item.public {
            self.error(
                file,
                item.line,
                format!("the `#[machine]` struct `{}` must be `pub`", item.name),
            );
        }
        let runs = self.number(file, item.line, &options, "runs");
        let mut init = Vec::new();
        let mut commands = Vec::new();
        let mut invariants = Vec::new();
        let mut observations = Vec::new();
        let returns_state = |method: &RawFn| matches!(method.result.as_deref(), Some(result) if result == "Self" || result == item.name);
        for method in methods {
            let markers: Vec<&Attr> = method
                .attrs
                .iter()
                .filter(|attr| is_method_marker(&attr.name))
                .collect();
            let Some(marker) = markers.first() else {
                continue;
            };
            if markers.len() > 1 {
                self.error(file, method.line, "a method takes one of `#[init]`, `#[command]`, `#[invariant]` and `#[observe]`");
                continue;
            }
            self.entry_shape(file, method, &marker.name);
            match marker.name.as_str() {
                "init" => {
                    if method.has_self || !returns_state(method) {
                        self.error(
                            file,
                            method.line,
                            format!("an `#[init]` takes no `self` and returns `{}`", item.name),
                        );
                    }
                    init.extend(self.method(file, method));
                }
                "command" => {
                    let options = self.options(file, &method.attrs, "command", &["weight"]);
                    let weight = self
                        .number(file, method.line, &options, "weight")
                        .unwrap_or(1);
                    if weight == 0 {
                        self.error(file, method.line, "a command's weight must be at least 1");
                    }
                    if !method.has_self || !returns_state(method) {
                        self.error(
                            file,
                            method.line,
                            format!("a `#[command]` takes `self` and returns `{}`", item.name),
                        );
                    }
                    if let Some(method) = self.method(file, method) {
                        commands.push(Command { method, weight });
                    }
                }
                "invariant" => {
                    if !method.has_self
                        || !method.params.is_empty()
                        || method.result.as_deref() != Some("bool")
                    {
                        self.error(
                            file,
                            method.line,
                            "an `#[invariant]` takes only `self` and returns `bool`",
                        );
                    }
                    invariants.extend(self.method(file, method));
                }
                _ => {
                    let ty = method.result.as_deref().and_then(Ty::parse);
                    if !method.has_self || !method.params.is_empty() || ty.is_none() {
                        self.error(file, method.line, "an `#[observe]` takes only `self` and returns `bool` or a fixed-width integer");
                    }
                    if let (Some(ty), Some(method)) = (ty, self.method(file, method)) {
                        observations.push(Observation { method, ty });
                    }
                }
            }
        }
        if init.len() != 1 {
            self.error(
                file,
                item.line,
                format!(
                    "the machine `{}` needs exactly one `#[init]` method",
                    item.name
                ),
            );
        }
        if commands.is_empty() {
            self.error(
                file,
                item.line,
                format!(
                    "the machine `{}` needs at least one `#[command]` method",
                    item.name
                ),
            );
        }
        Some(Machine {
            name: item.name.clone(),
            path: path.to_vec(),
            init: init.into_iter().next()?,
            commands,
            invariants,
            observations,
            steps: options
                .get("steps")
                .cloned()
                .unwrap_or_else(|| DEFAULT_STEPS.to_string()),
            runs,
            clock: options.get("clock").cloned(),
            reuse: false,
            location: Location {
                file: file.to_path_buf(),
                line: item.line,
            },
        })
    }

    fn method(&mut self, file: &Path, method: &RawFn) -> Option<Method> {
        let params = self.params(file, method)?;
        let effects = method
            .effects
            .iter()
            .map(|effect| Effect {
                ty: effect.ty.clone(),
                mutable: effect.mutable,
            })
            .collect();
        Some(Method {
            name: method.name.clone(),
            params,
            effects,
            location: Location {
                file: file.to_path_buf(),
                line: method.line,
            },
        })
    }

    // Rules shared by every entry point.
    fn entry_shape(&mut self, file: &Path, function: &RawFn, marker: &str) {
        if !function.public {
            self.error(
                file,
                function.line,
                format!(
                    "`{}` is marked `#[{marker}]` and must be `pub`",
                    function.name
                ),
            );
        }
        if function.generic {
            self.error(
                file,
                function.line,
                format!(
                    "`{}` is marked `#[{marker}]` and cannot be generic",
                    function.name
                ),
            );
        }
        if has(&function.attrs, "cfg") {
            self.error(
                file,
                function.line,
                format!(
                    "`{}` is marked `#[{marker}]` and cannot be conditionally compiled",
                    function.name
                ),
            );
        }
        if self.shared && !function.effects.is_empty() {
            self.error(
                file,
                function.line,
                format!(
                    "`{}` is in a shared module, so it cannot declare effects",
                    function.name
                ),
            );
        }
    }

    fn params(&mut self, file: &Path, function: &RawFn) -> Option<Vec<Param>> {
        let generators = self.generators(file, function);
        self.params_from(file, function, generators)
    }

    /// The `#[gen(..)]` specifications of a function, by parameter name.
    fn generators(&mut self, file: &Path, function: &RawFn) -> BTreeMap<String, String> {
        let mut generators = BTreeMap::new();
        for attr in function.attrs.iter().filter(|attr| attr.name == "gen") {
            for arg in &attr.args {
                match &arg.key {
                    Some(key) => {
                        generators.insert(key.clone(), arg.value.clone());
                    }
                    None => self.error(
                        file,
                        attr.line,
                        format!(
                            "`#[gen]` takes `parameter = \"spec\"` pairs, not `{}`",
                            arg.value
                        ),
                    ),
                }
            }
        }
        generators
    }

    fn params_from(
        &mut self,
        file: &Path,
        function: &RawFn,
        mut generators: BTreeMap<String, String>,
    ) -> Option<Vec<Param>> {
        let mut params = Vec::new();
        let mut valid = true;
        for raw in &function.params {
            match Ty::parse(&raw.ty) {
                Some(ty) => params.push(Param {
                    name: raw.name.clone(),
                    label: raw.label.clone(),
                    ty,
                    generator: generators
                        .remove(&raw.name)
                        .unwrap_or_else(|| "any".to_string()),
                }),
                None => {
                    valid = false;
                    self.error(
                        file,
                        function.line,
                        format!(
                            "parameter `{}` of `{}` is a `{}`; entry points take `bool`, `u8`..`u256` and `i8`..`i256`",
                            raw.name, function.name, raw.ty
                        ),
                    );
                }
            }
        }
        for unknown in generators.keys() {
            self.error(
                file,
                function.line,
                format!(
                    "`#[gen]` names `{unknown}`, which is not a parameter of `{}`",
                    function.name
                ),
            );
        }
        valid.then_some(params)
    }

    fn options(
        &mut self,
        file: &Path,
        attrs: &[Attr],
        marker: &str,
        known: &[&str],
    ) -> BTreeMap<String, String> {
        let mut options = BTreeMap::new();
        for attr in attrs.iter().filter(|attr| attr.name == marker) {
            for arg in &attr.args {
                match arg.key.as_deref() {
                    Some(key) if known.contains(&key) => {
                        options.insert(key.to_string(), arg.value.clone());
                    }
                    _ => self.error(
                        file,
                        attr.line,
                        format!(
                            "`#[{marker}]` takes {}; `{}` is not one of them",
                            if known.is_empty() {
                                "no options".to_string()
                            } else {
                                known.join(", ")
                            },
                            arg.key.clone().unwrap_or_else(|| arg.value.clone())
                        ),
                    ),
                }
            }
        }
        options
    }

    fn number(
        &mut self,
        file: &Path,
        line: usize,
        options: &BTreeMap<String, String>,
        key: &str,
    ) -> Option<u32> {
        let text = options.get(key)?;
        match text.replace('_', "").parse() {
            Ok(number) => Some(number),
            Err(_) => {
                self.error(
                    file,
                    line,
                    format!("`{key}` must be a whole number, not `{text}`"),
                );
                None
            }
        }
    }

    fn storage_invariant(&mut self, file: &Path, path: &[String], function: &RawFn) {
        let options = self.options(file, &function.attrs, "invariant", &["of"]);
        if function.has_self
            || function.generic
            || function.result.as_deref() != Some("bool")
            || !function.effects.is_empty()
        {
            self.error(
                file,
                function.line,
                "an `#[invariant]` of a contract takes its fields by name and returns `bool`, \
                 without `self`, type parameters or effects",
            );
            return;
        }
        self.storage_invariants.push((
            StorageInvariant {
                name: function.name.clone(),
                path: path.to_vec(),
                location: Location {
                    file: file.to_path_buf(),
                    line: function.line,
                },
            },
            options.get("of").cloned(),
        ));
    }

    fn message_claim(&mut self, file: &Path, path: &[String], function: &RawFn, kind: MessageKind) {
        let marker = match kind {
            MessageKind::Ensures => "ensures",
            MessageKind::Reverts => "reverts",
        };
        let options = self.options(file, &function.attrs, marker, &["message", "of"]);
        let Some(message) = options.get("message").cloned() else {
            self.error(
                file,
                function.line,
                format!("`#[{marker}]` names its message: `#[{marker}(message = \"Name\")]`"),
            );
            return;
        };
        if function.has_self
            || function.generic
            || function.result.as_deref() != Some("bool")
            || !function.effects.is_empty()
        {
            self.error(
                file,
                function.line,
                format!(
                    "a `#[{marker}]` function takes storage fields, message fields and the \
                     context by name and returns `bool`, without `self`, type parameters or effects"
                ),
            );
            return;
        }
        self.message_claims.push((
            MessageClaim {
                name: function.name.clone(),
                path: path.to_vec(),
                kind,
                message,
                location: Location {
                    file: file.to_path_buf(),
                    line: function.line,
                },
            },
            options.get("of").cloned(),
        ));
    }

    /// The contract a free invariant or message claim describes: the one it
    /// names, or the only contract of the module.
    fn contract_for(
        &mut self,
        location: &Location,
        named: &Option<String>,
    ) -> Option<(RawContract, Vec<String>, PathBuf)> {
        let candidates: Vec<&(RawContract, Vec<String>, PathBuf)> = self
            .deployed
            .iter()
            .filter(|(contract, _, _)| named.as_ref().is_none_or(|name| *name == contract.name))
            .collect();
        let message = match (candidates.as_slice(), named) {
            ([only], _) => return Some((*only).clone()),
            ([], Some(name)) => format!("there is no contract `{name}` in this module"),
            ([], None) => {
                "an `#[invariant]` outside a `#[machine]`, `#[ensures]` and `#[reverts]` \
                 describe a contract, but this module declares no contract"
                    .to_string()
            }
            (_, _) => "this module declares several contracts; name one with \
                 `of = \"Name\"`"
                .to_string(),
        };
        let (file, line) = (location.file.clone(), location.line);
        self.error(&file, line, message);
        None
    }

    fn claim_entry(
        claims: &mut Vec<StorageClaim>,
        (contract, path, file): (RawContract, Vec<String>, PathBuf),
    ) -> &mut StorageClaim {
        if let Some(index) = claims.iter().position(|claim| claim.name == contract.name) {
            return &mut claims[index];
        }
        claims.push(StorageClaim {
            name: contract.name.clone(),
            path,
            invariants: Vec::new(),
            claims_invariants: true,
            messages: Vec::new(),
            location: Location {
                file,
                line: contract.line,
            },
        });
        claims.last_mut().expect("just pushed")
    }

    /// Groups the free invariants and message claims by the contract each
    /// describes.
    fn resolve_storage(&mut self) -> Vec<StorageClaim> {
        let mut claims: Vec<StorageClaim> = Vec::new();
        for (invariant, named) in std::mem::take(&mut self.storage_invariants) {
            if let Some(contract) = self.contract_for(&invariant.location, &named) {
                Self::claim_entry(&mut claims, contract)
                    .invariants
                    .push(invariant);
            }
        }
        for (message, named) in std::mem::take(&mut self.message_claims) {
            if let Some(contract) = self.contract_for(&message.location, &named) {
                Self::claim_entry(&mut claims, contract)
                    .messages
                    .push(message);
            }
        }
        claims
    }

    fn check_names(&mut self, storage: &[StorageClaim]) {
        let mut seen: BTreeMap<String, Location> = BTreeMap::new();
        let entries: Vec<(String, Location)> = self
            .suite
            .properties
            .iter()
            .map(|p| (p.name.clone(), p.location.clone()))
            .chain(
                self.suite
                    .machines
                    .iter()
                    .map(|m| (m.name.clone(), m.location.clone())),
            )
            .chain(
                self.contracts
                    .iter()
                    .map(|c| (c.name.clone(), c.location.clone())),
            )
            .chain(storage.iter().map(|c| (c.name.clone(), c.location.clone())))
            .chain(
                storage
                    .iter()
                    .flat_map(|c| &c.messages)
                    .map(|m| (m.name.clone(), m.location.clone())),
            )
            .collect();
        for (name, location) in entries {
            if let Some(first) = seen.get(&name) {
                let message = format!(
                    "`{name}` is also declared at {first}; entry point names must be unique within a module"
                );
                self.error(&location.file, location.line, message);
            } else {
                seen.insert(name, location);
            }
        }
    }
}

fn has(attrs: &[Attr], name: &str) -> bool {
    attrs.iter().any(|attr| attr.name == name)
}

fn message_kind(attrs: &[Attr]) -> Option<MessageKind> {
    if has(attrs, "ensures") {
        Some(MessageKind::Ensures)
    } else if has(attrs, "reverts") {
        Some(MessageKind::Reverts)
    } else {
        None
    }
}

fn is_method_marker(name: &str) -> bool {
    matches!(name, "init" | "command" | "invariant" | "observe")
}

#[cfg(test)]
mod tests;
