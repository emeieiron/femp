//! Renders the EVM harness: a contract with one message per entry point.
//! A property is one call. A machine keeps its state in storage; its
//! initializer and each command are transactions, and each invariant and
//! observation is a call made after them.

use femp_core::codegen::fe::{self, SourceWriter};
use femp_core::property::{Machine, Method, Param, Suite, Ty};

pub const CONTRACT: &str = "FempPropertyHarness";
const MESSAGE: &str = "FempHarness";
const MACHINES: &str = "femp_machines";

/// The effects a contract machine's methods may use, with the binding the
/// harness contract declares for each.
const EFFECTS: [(&str, &str); 4] = [
    ("Create", "femp_create"),
    ("Call", "femp_call"),
    ("Ctx", "femp_ctx"),
    ("Log", "femp_log"),
];

pub fn binding(effect: &str) -> Option<&'static str> {
    EFFECTS
        .iter()
        .find(|(ty, _)| *ty == effect)
        .map(|(_, binding)| *binding)
}

pub fn unsupported_effect(machine: &Machine) -> Option<&str> {
    machine
        .methods()
        .flat_map(|method| &method.effects)
        .map(|effect| effect.ty.as_str())
        .find(|ty| binding(ty).is_none())
}

/// One message of the harness: its Solidity signature, its variant name,
/// and the type of the value it returns.
pub struct Message {
    pub function: String,
    pub variant: String,
    pub params: Vec<Ty>,
    pub result: Ty,
}

impl Message {
    pub fn signature(&self) -> String {
        let params: Vec<String> = self.params.iter().map(|ty| ty.sol_name()).collect();
        format!("{}({})", self.function, params.join(","))
    }
}

fn types(params: &[Param]) -> Vec<Ty> {
    params.iter().map(|param| param.ty).collect()
}

pub fn property_message(suite: &Suite, index: usize) -> Message {
    let id = suite.property_id(index);
    let property = &suite.properties[index];
    Message {
        function: format!("femp_entry_{id}"),
        variant: format!("FempEntry{id}"),
        params: types(&property.params),
        result: property.result,
    }
}

pub fn init_message(suite: &Suite, index: usize) -> Message {
    let id = suite.machine_id(index);
    Message {
        function: format!("femp_init_{id}"),
        variant: format!("FempM{id}Init"),
        params: types(&suite.machines[index].init.params),
        result: Ty::Bool,
    }
}

pub fn command_message(suite: &Suite, index: usize, command: usize) -> Message {
    let id = suite.machine_id(index);
    Message {
        function: format!("femp_command_{id}_{command}"),
        variant: format!("FempM{id}Command{command}"),
        params: types(&suite.machines[index].commands[command].method.params),
        result: Ty::Bool,
    }
}

pub fn invariant_message(suite: &Suite, index: usize, invariant: usize) -> Message {
    let id = suite.machine_id(index);
    Message {
        function: format!("femp_invariant_{id}_{invariant}"),
        variant: format!("FempM{id}Invariant{invariant}"),
        params: Vec::new(),
        result: Ty::Bool,
    }
}

pub fn observation_message(suite: &Suite, index: usize, observation: usize) -> Message {
    let id = suite.machine_id(index);
    Message {
        function: format!("femp_observe_{id}_{observation}"),
        variant: format!("FempM{id}Observe{observation}"),
        params: Vec::new(),
        result: suite.machines[index].observations[observation].ty,
    }
}

pub fn renderable_machine(machine: &Machine) -> bool {
    unsupported_effect(machine).is_none()
}

pub fn render(suite: &Suite) -> String {
    let machines: Vec<usize> = (0..suite.machines.len())
        .filter(|index| renderable_machine(&suite.machines[*index]))
        .collect();
    let mut arms: Vec<(Message, Arm)> = (0..suite.properties.len())
        .map(|index| (property_message(suite, index), Arm::Property(index)))
        .collect();
    for &index in &machines {
        let machine = &suite.machines[index];
        arms.push((init_message(suite, index), Arm::Init(index)));
        for command in 0..machine.commands.len() {
            arms.push((
                command_message(suite, index, command),
                Arm::Command(index, command),
            ));
        }
        for invariant in 0..machine.invariants.len() {
            arms.push((
                invariant_message(suite, index, invariant),
                Arm::Invariant(index, invariant),
            ));
        }
        for observation in 0..machine.observations.len() {
            arms.push((
                observation_message(suite, index, observation),
                Arm::Observe(index, observation),
            ));
        }
    }

    let mut out = fe::writer();
    out.line(fe::HEADER);
    out.block(format!("msg {MESSAGE} {{"), "}", |out| {
        for (message, _) in &arms {
            let fields = fields(message.params.len(), &message.params);
            out.line(format!("#[selector = sol(\"{}\")]", message.signature()));
            out.line(format!(
                "{}{} -> {},",
                message.variant,
                if fields.is_empty() {
                    String::new()
                } else {
                    format!(" {{ {fields} }}")
                },
                fe::type_name(message.result)
            ));
        }
    });
    out.blank();
    if !machines.is_empty() {
        out.block("struct FempMachines {", "}", |out| {
            for &index in &machines {
                let machine = &suite.machines[index];
                out.line(format!(
                    "m{}: {},",
                    suite.machine_id(index),
                    suite.item_path(&machine.path, &machine.name)
                ));
            }
        });
        out.blank();
    }
    let effects = contract_effects(suite, &machines);
    let uses = if effects.is_empty() {
        String::new()
    } else {
        format!(" uses ({})", effects.join(", "))
    };
    out.block(format!("pub contract {CONTRACT}{uses} {{"), "}", |out| {
        if !machines.is_empty() {
            out.line(format!("mut {MACHINES}: FempMachines,"));
            out.blank();
        }
        out.block(format!("recv {MESSAGE} {{"), "}", |out| {
            for (index, (message, arm)) in arms.iter().enumerate() {
                if index > 0 {
                    out.blank();
                }
                render_arm(out, suite, message, arm);
            }
        });
    });
    out.finish()
}

enum Arm {
    Property(usize),
    Init(usize),
    Command(usize, usize),
    Invariant(usize, usize),
    Observe(usize, usize),
}

fn fields(count: usize, params: &[Ty]) -> String {
    fe::locals(count)
        .iter()
        .zip(params)
        .map(|(local, ty)| format!("{local}: {}", fe::type_name(*ty)))
        .collect::<Vec<_>>()
        .join(", ")
}

// The contract declares every effect its machines use, mutable when any
// method needs it so.
fn contract_effects(suite: &Suite, machines: &[usize]) -> Vec<String> {
    EFFECTS
        .iter()
        .filter_map(|(ty, binding)| {
            let uses: Vec<bool> = machines
                .iter()
                .flat_map(|index| suite.machines[*index].methods())
                .flat_map(|method| &method.effects)
                .filter(|effect| effect.ty == *ty)
                .map(|effect| effect.mutable)
                .collect();
            if uses.is_empty() {
                None
            } else if uses.contains(&true) {
                Some(format!("{binding}: mut {ty}"))
            } else {
                Some(format!("{binding}: {ty}"))
            }
        })
        .collect()
}

fn arm_uses(machines: Option<bool>, method: Option<&Method>) -> String {
    let mut uses = Vec::new();
    match machines {
        Some(true) => uses.push(format!("mut {MACHINES}")),
        Some(false) => uses.push(MACHINES.to_string()),
        None => {}
    }
    for effect in method.into_iter().flat_map(|method| &method.effects) {
        if let Some(binding) = binding(&effect.ty) {
            uses.push(if effect.mutable {
                format!("mut {binding}")
            } else {
                binding.to_string()
            });
        }
    }
    if uses.is_empty() {
        String::new()
    } else {
        format!(" uses ({})", uses.join(", "))
    }
}

fn render_arm(out: &mut SourceWriter, suite: &Suite, message: &Message, arm: &Arm) {
    let locals = fe::locals(message.params.len());
    let pattern = if locals.is_empty() {
        message.variant.clone()
    } else {
        format!("{} {{ {} }}", message.variant, locals.join(", "))
    };
    let result = fe::type_name(message.result);
    let state = |index: usize| format!("{MACHINES}.m{}", suite.machine_id(index));
    match arm {
        Arm::Property(index) => {
            let property = &suite.properties[*index];
            out.block(format!("{pattern} -> {result} {{"), "}", |out| {
                out.line(format!(
                    "{}({})",
                    suite.item_path(&property.path, &property.name),
                    fe::arguments(&property.params, &locals)
                ));
            });
        }
        Arm::Init(index) => {
            let machine = &suite.machines[*index];
            let method = &machine.init;
            out.block(
                format!(
                    "{pattern} -> {result}{} {{",
                    arm_uses(Some(true), Some(method))
                ),
                "}",
                |out| {
                    out.line(format!(
                        "{} = {}::{}({})",
                        state(*index),
                        suite.item_path(&machine.path, &machine.name),
                        method.name,
                        fe::arguments(&method.params, &locals)
                    ));
                    out.line("true");
                },
            );
        }
        Arm::Command(index, command) => {
            let method = &suite.machines[*index].commands[*command].method;
            out.block(
                format!(
                    "{pattern} -> {result}{} {{",
                    arm_uses(Some(true), Some(method))
                ),
                "}",
                |out| {
                    out.line(format!(
                        "{} = {}.{}({})",
                        state(*index),
                        state(*index),
                        method.name,
                        fe::arguments(&method.params, &locals)
                    ));
                    out.line("true");
                },
            );
        }
        Arm::Invariant(index, invariant) => {
            let method = &suite.machines[*index].invariants[*invariant];
            out.block(
                format!(
                    "{pattern} -> {result}{} {{",
                    arm_uses(Some(false), Some(method))
                ),
                "}",
                |out| {
                    out.line(format!("{}.{}()", state(*index), method.name));
                },
            );
        }
        Arm::Observe(index, observation) => {
            let method = &suite.machines[*index].observations[*observation].method;
            out.block(
                format!(
                    "{pattern} -> {result}{} {{",
                    arm_uses(Some(false), Some(method))
                ),
                "}",
                |out| {
                    out.line(format!("{}.{}()", state(*index), method.name));
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use femp_core::property::{Effect, fixtures};

    use super::*;

    fn check_fixture(name: &str, rendered: &str) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        if std::env::var_os("FEMP_UPDATE_FIXTURES").is_some() {
            std::fs::write(&path, rendered).unwrap();
        }
        assert_eq!(rendered, std::fs::read_to_string(path).unwrap());
    }

    #[test]
    fn renders_the_fixture_suite() {
        check_fixture("harness.fe", &render(&fixtures::suite()));
    }

    #[test]
    fn declares_the_effects_contract_machines_use() {
        let mut suite = fixtures::suite();
        suite.properties.clear();
        let machine = &mut suite.machines[0];
        machine.init.effects.push(Effect {
            ty: "Create".into(),
            mutable: true,
        });
        machine.invariants[0].effects.push(Effect {
            ty: "Call".into(),
            mutable: false,
        });
        let rendered = render(&suite);
        assert!(
            rendered.contains("uses (femp_create: mut Create, femp_call: Call) {"),
            "{rendered}"
        );
        assert!(rendered.contains("-> bool uses (mut femp_machines, mut femp_create) {"));
        assert!(rendered.contains("-> bool uses (femp_machines, femp_call) {"));

        suite.machines[0].commands[0].method.effects.push(Effect {
            ty: "Evm".into(),
            mutable: true,
        });
        assert_eq!(unsupported_effect(&suite.machines[0]), Some("Evm"));
        assert!(!render(&suite).contains("FempMachines"));
    }

    #[test]
    fn signatures_use_solidity_types() {
        let suite = fixtures::suite();
        assert_eq!(
            property_message(&suite, 0).signature(),
            "femp_entry_0(uint64,bool)"
        );
        assert_eq!(
            command_message(&suite, 0, 1).signature(),
            "femp_command_3_1(int8)"
        );
        assert_eq!(observation_message(&suite, 0, 1).result, Ty::Int(8));
    }
}
