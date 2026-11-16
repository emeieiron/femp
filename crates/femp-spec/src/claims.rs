use std::fmt;

use femp_core::property::{Location, Param, Property, PropertyKind, Suite, Ty};

/// Everything one module claims: the entry points property testing runs and
/// proving checks, and the `#[spec]` functions that describe other functions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Claims {
    pub suite: Suite,
    pub contracts: Vec<Contract>,
    /// The deployed contracts whose storage `#[invariant]`s describe.
    pub storage: Vec<StorageClaim>,
}

impl Claims {
    pub fn is_empty(&self) -> bool {
        self.suite.is_empty() && self.contracts.is_empty() && self.storage.is_empty()
    }

    /// The entry points property testing runs: the suite, and the specs it
    /// can run as properties after the suite's own.
    pub fn testable(&self) -> Suite {
        let mut suite = self.suite.clone();
        suite
            .properties
            .extend(self.contracts.iter().filter_map(Contract::runnable));
        suite
    }

    /// Keeps the claims whose name contains `pattern`.
    pub fn retain_matching(&mut self, pattern: &str) {
        self.suite
            .properties
            .retain(|property| property.name.contains(pattern));
        self.suite
            .machines
            .retain(|machine| machine.name.contains(pattern));
        self.contracts
            .retain(|contract| contract.name.contains(pattern));
        self.storage.retain(|claim| claim.name.contains(pattern));
    }
}

/// A `contract` and the `#[invariant]` functions that state what its
/// storage keeps after every message it accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageClaim {
    /// The contract's name.
    pub name: String,
    /// The module path of the contract inside its ingot.
    pub path: Vec<String>,
    pub invariants: Vec<StorageInvariant>,
    pub location: Location,
}

/// A free function of the contract's fields, named as its parameters, that
/// returns `bool`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageInvariant {
    pub name: String,
    pub path: Vec<String>,
    pub location: Location,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractMode {
    /// Proved, then used in place of the target's body at call sites.
    Prove,
    /// Assumed without proof and used at call sites; reported as trusted.
    Trusted,
}

/// A `#[spec]` function: it takes the target's arguments, states what it
/// `requires`, calls the target once and states what it `ensures`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    pub name: String,
    /// The module path of the spec function inside its ingot.
    pub path: Vec<String>,
    pub target: TargetRef,
    pub mode: ContractMode,
    /// Calls to the target keep its body instead of using the spec.
    pub transparent: bool,
    /// Seconds the solver may spend on each of the spec's obligations.
    pub timeout: Option<u32>,
    pub params: Vec<SpecParam>,
    /// The result type, when it is a primitive.
    pub result: Option<Ty>,
    pub public: bool,
    pub location: Location,
}

impl Contract {
    /// The spec as an entry point property testing can run: a public
    /// function of primitives to a primitive.
    pub fn runnable(&self) -> Option<Property> {
        if !self.public {
            return None;
        }
        let params = self
            .params
            .iter()
            .map(|param| {
                Some(Param {
                    name: param.name.clone(),
                    label: param.label.clone(),
                    ty: param.ty?,
                    generator: param.generator.clone(),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Property {
            name: self.name.clone(),
            kind: PropertyKind::Spec,
            path: self.path.clone(),
            params,
            result: self.result?,
            cases: None,
            location: self.location.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecParam {
    pub name: String,
    pub label: Option<String>,
    /// The type as written.
    pub ty_text: String,
    /// The type when it is a primitive that crosses harness boundaries.
    pub ty: Option<Ty>,
    pub generator: String,
}

/// The function a spec describes: a free function or an inherent method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetRef {
    pub path: Vec<String>,
    /// The type whose `impl` block declares the method.
    pub owner: Option<String>,
    pub name: String,
    pub location: Location,
}

impl fmt::Display for TargetRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.owner {
            Some(owner) => write!(f, "{owner}::{}", self.name),
            None => f.write_str(&self.name),
        }
    }
}
