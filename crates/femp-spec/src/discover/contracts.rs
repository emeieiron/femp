//! `#[spec]` functions. A spec names the function it describes with
//! `target = "Type::method"`, or by its own name: `claim_spec` describes
//! `claim`. Targets are resolved once every source of the module is read,
//! since a spec usually lives apart from the code it describes.

use std::path::{Path, PathBuf};

use femp_core::property::{Location, Ty};

use super::parse::{RawFn, RawModule};
use super::{Builder, has};
use crate::claims::{Contract, ContractMode, SpecParam, TargetRef};

const MARKER: &str = "spec";
const SUFFIX: &str = "_spec";
const FLAGS: [&str; 3] = ["prove", "trusted", "transparent"];
const KEYS: [&str; 2] = ["target", "timeout"];

pub(super) struct PendingSpec {
    file: PathBuf,
    path: Vec<String>,
    conditional: bool,
    function: RawFn,
}

pub(super) struct Candidate {
    path: Vec<String>,
    owner: Option<String>,
    name: String,
    location: Location,
}

impl Candidate {
    fn segments(&self) -> Vec<&str> {
        self.path
            .iter()
            .map(String::as_str)
            .chain(self.owner.as_deref())
            .chain(std::iter::once(self.name.as_str()))
            .collect()
    }
}

#[derive(Default)]
struct Options {
    flags: Vec<String>,
    target: Option<String>,
    timeout: Option<String>,
}

impl Builder {
    pub(super) fn note_candidates(&mut self, file: &Path, raw: &RawModule) {
        let location = |line| Location {
            file: file.to_path_buf(),
            line,
        };
        for function in raw.fns.iter().filter(|f| !has(&f.attrs, MARKER)) {
            self.candidates.push(Candidate {
                path: raw.path.clone(),
                owner: None,
                name: function.name.clone(),
                location: location(function.line),
            });
        }
        for item in &raw.impls {
            for method in &item.methods {
                self.candidates.push(Candidate {
                    path: raw.path.clone(),
                    owner: Some(item.type_name.clone()),
                    name: method.name.clone(),
                    location: location(method.line),
                });
            }
        }
    }

    pub(super) fn pending_spec(&mut self, file: &Path, raw: &RawModule, function: &RawFn) {
        self.specs.push(PendingSpec {
            file: file.to_path_buf(),
            path: raw.path.clone(),
            conditional: raw.conditional,
            function: function.clone(),
        });
    }

    pub(super) fn resolve_specs(&mut self) {
        for spec in std::mem::take(&mut self.specs) {
            if let Some(contract) = self.contract(&spec) {
                self.contracts.push(contract);
            }
        }
    }

    fn contract(&mut self, spec: &PendingSpec) -> Option<Contract> {
        let (file, function) = (spec.file.as_path(), &spec.function);
        let line = function.line;
        for other in ["property", "differential"] {
            if has(&function.attrs, other) {
                self.error(
                    file,
                    line,
                    format!("use either `#[spec]` or `#[{other}]`, not both"),
                );
            }
        }
        if spec.conditional {
            self.error(
                file,
                line,
                "specs cannot live in a module compiled for some targets only",
            );
        }
        self.spec_shape(file, function);
        let options = self.spec_options(file, function);
        let trusted = options.flags.iter().any(|flag| flag == "trusted");
        let transparent = options.flags.iter().any(|flag| flag == "transparent");
        if trusted && options.flags.iter().any(|flag| flag == "prove") {
            self.error(
                file,
                line,
                "a spec is either `prove` or `trusted`, not both",
            );
        }
        if trusted && transparent {
            self.error(
                file,
                line,
                "a `trusted` spec stands in for its target, so it cannot be `transparent`",
            );
        }
        let timeout = options.timeout.as_ref().and_then(|text| {
            let parsed = text.replace('_', "").parse().ok().filter(|n: &u32| *n > 0);
            if parsed.is_none() {
                self.error(
                    file,
                    line,
                    format!("`timeout` must be a whole number of seconds, not `{text}`"),
                );
            }
            parsed
        });
        let target = self.target(spec, options.target.as_deref())?;
        let params = self.spec_params(file, function)?;
        Some(Contract {
            name: function.name.clone(),
            path: spec.path.clone(),
            target,
            mode: if trusted {
                ContractMode::Trusted
            } else {
                ContractMode::Prove
            },
            transparent,
            timeout,
            params,
            location: Location {
                file: file.to_path_buf(),
                line,
            },
        })
    }

    fn spec_shape(&mut self, file: &Path, function: &RawFn) {
        let name = &function.name;
        let line = function.line;
        if function.generic {
            self.error(file, line, format!("the spec `{name}` cannot be generic"));
        }
        if function.has_self {
            self.error(file, line, format!("the spec `{name}` cannot take `self`"));
        }
        if has(&function.attrs, "cfg") {
            self.error(
                file,
                line,
                format!("the spec `{name}` cannot be conditionally compiled"),
            );
        }
        if self.shared && !function.effects.is_empty() {
            self.error(
                file,
                line,
                format!("`{name}` is in a shared module, so it cannot declare effects"),
            );
        }
    }

    fn spec_options(&mut self, file: &Path, function: &RawFn) -> Options {
        let mut options = Options::default();
        for attr in function.attrs.iter().filter(|attr| attr.name == MARKER) {
            for arg in &attr.args {
                match arg.key.as_deref() {
                    None if FLAGS.contains(&arg.value.as_str()) => {
                        options.flags.push(arg.value.clone())
                    }
                    Some("target") => options.target = Some(arg.value.clone()),
                    Some("timeout") => options.timeout = Some(arg.value.clone()),
                    _ => self.error(
                        file,
                        attr.line,
                        format!(
                            "`#[spec]` takes {} and {}; `{}` is not one of them",
                            FLAGS.join(", "),
                            KEYS.map(|key| format!("{key} = ..")).join(", "),
                            arg.key.clone().unwrap_or_else(|| arg.value.clone())
                        ),
                    ),
                }
            }
        }
        options
    }

    fn target(&mut self, spec: &PendingSpec, written: Option<&str>) -> Option<TargetRef> {
        let (file, function) = (spec.file.as_path(), &spec.function);
        let wanted = match written {
            Some(text) => text.trim().to_string(),
            None => match function.name.strip_suffix(SUFFIX) {
                Some(stem) if !stem.is_empty() => stem.to_string(),
                _ => {
                    self.error(
                        file,
                        function.line,
                        format!(
                            "the spec `{}` names no target: add `target = \"..\"` or name it `<target>{SUFFIX}`",
                            function.name
                        ),
                    );
                    return None;
                }
            },
        };
        let segments: Vec<&str> = wanted.split("::").map(str::trim).collect();
        let mut matches: Vec<&Candidate> = self
            .candidates
            .iter()
            .filter(|candidate| candidate.segments().ends_with(&segments))
            .collect();
        // A spec of a method takes the receiver first, which tells methods
        // of the same name apart.
        if matches.len() > 1
            && let Some(first) = function.params.first()
        {
            let receivers: Vec<&Candidate> = matches
                .iter()
                .copied()
                .filter(|candidate| candidate.owner.as_deref() == Some(first.ty.as_str()))
                .collect();
            if receivers.len() == 1 {
                matches = receivers;
            }
        }
        match matches.as_slice() {
            [found] => Some(TargetRef {
                path: found.path.clone(),
                owner: found.owner.clone(),
                name: found.name.clone(),
                location: found.location.clone(),
            }),
            [] => {
                self.error(
                    file,
                    function.line,
                    format!(
                        "the spec `{}` describes `{wanted}`, which is not a function of this module",
                        function.name
                    ),
                );
                None
            }
            several => {
                let places: Vec<String> = several
                    .iter()
                    .map(|candidate| format!("`{}`", candidate.segments().join("::")))
                    .collect();
                self.error(
                    file,
                    function.line,
                    format!(
                        "the spec `{}` describes `{wanted}`, which could be {}; write `target = \"..\"` with more of its path",
                        function.name,
                        places.join(" or ")
                    ),
                );
                None
            }
        }
    }

    fn spec_params(&mut self, file: &Path, function: &RawFn) -> Option<Vec<SpecParam>> {
        let mut generators = self.generators(file, function);
        let mut params = Vec::new();
        for raw in &function.params {
            let ty = Ty::parse(&raw.ty);
            let generator = generators.remove(&raw.name);
            if ty.is_none() && generator.is_some() {
                self.error(
                    file,
                    function.line,
                    format!(
                        "`#[gen]` describes `{}`, a `{}`; only `bool` and fixed-width integer parameters take generators",
                        raw.name, raw.ty
                    ),
                );
            }
            params.push(SpecParam {
                name: raw.name.clone(),
                label: raw.label.clone(),
                ty_text: raw.ty.clone(),
                ty,
                generator: generator.unwrap_or_else(|| "any".to_string()),
            });
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
        Some(params)
    }
}
