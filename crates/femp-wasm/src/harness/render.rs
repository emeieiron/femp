//! Renders the WebAssembly harness: one export per property, and per
//! machine an export that runs a whole sequence read from a host file and
//! logs each record. Entry points whose types do not fit the module
//! boundary get no export.

use femp_core::codegen::fe::{self, SourceWriter};
use femp_core::property::{Machine, Method, Param, Property, Suite, Ty};
use serde_json::{Value as Json, json};

/// Every type a WebAssembly export or a 64-bit host word can carry.
pub fn fits(ty: Ty) -> bool {
    ty.bits() <= 64
}

pub fn property_fits(property: &Property) -> bool {
    fits(property.result) && property.params.iter().all(|param| fits(param.ty))
}

pub fn machine_fits(machine: &Machine) -> bool {
    !machine.uses_effects()
        && machine
            .methods()
            .all(|method| method.params.iter().all(|param| fits(param.ty)))
        && machine
            .observations
            .iter()
            .all(|observation| fits(observation.ty))
}

pub fn export_name(id: usize) -> String {
    format!("femp_entry_{id}")
}

pub fn render(suite: &Suite) -> String {
    let mut out = fe::writer();
    out.line(fe::HEADER);
    if !suite.machines.is_empty() {
        out.line("use std::wasm::{HostText, Log}");
        out.line("use std::wasm::files::{Files, Handle, close, open, read_byte}");
        out.blank();
        helpers(&mut out);
    }
    for (index, property) in suite.properties.iter().enumerate() {
        if property_fits(property) {
            render_property(&mut out, suite, suite.property_id(index), property);
        }
    }
    for (index, machine) in suite.machines.iter().enumerate() {
        if machine_fits(machine) {
            render_machine(&mut out, suite, suite.machine_id(index), machine);
        }
    }
    out.finish()
}

/// The runner's table of entry points, in id order.
pub fn entries(suite: &Suite) -> Json {
    let properties = suite
        .properties
        .iter()
        .enumerate()
        .map(|(index, property)| {
            if property_fits(property) {
                json!({
                    "kind": "property",
                    "export": export_name(suite.property_id(index)),
                    "params": property.params.len(),
                })
            } else {
                json!({ "kind": "unsupported" })
            }
        });
    let machines = suite.machines.iter().enumerate().map(|(index, machine)| {
        if machine_fits(machine) {
            json!({
                "kind": "machine",
                "export": export_name(suite.machine_id(index)),
                "init": machine.init.params.len(),
                "commands": machine
                    .commands
                    .iter()
                    .map(|command| command.method.params.len())
                    .collect::<Vec<_>>(),
            })
        } else {
            json!({ "kind": "unsupported" })
        }
    });
    Json::Array(properties.chain(machines).collect())
}

fn helpers(out: &mut SourceWriter) {
    out.block(
        "fn femp_word(_ handle: Handle, _ index: u32) -> u64 uses (files: mut Files) {",
        "}",
        |out| {
            out.line("var value: u64 = 0");
            out.line("var byte_index: u32 = 0");
            out.block("while byte_index < 8 {", "}", |out| {
                out.block(
                    "match read_byte(handle, index * 8 + byte_index) {",
                    "}",
                    |out| {
                        out.block("Result::Ok(byte) => {", "},", |out| {
                            out.line(
                                "value = value | ((byte as u64) << ((byte_index * 8) as u64))",
                            );
                        });
                        out.line("Result::Err(_) => core::panic(),");
                    },
                );
                out.line("byte_index += 1");
            });
            out.line("value");
        },
    );
    out.blank();
    // Signed values cross as their 64-bit two's-complement pattern.
    out.block("fn femp_signed(_ word: u64) -> i64 {", "}", |out| {
        out.line("if word > 9223372036854775807 {");
        out.indented(|out| {
            out.line("let wide: i128 = (word as i128) - 18446744073709551616");
            out.line("wide.downcast_unchecked()");
        });
        out.line("} else {");
        out.indented(|out| {
            out.line("word.downcast_unchecked()");
        });
        out.line("}");
    });
    out.blank();
    out.block("fn femp_bits(_ value: i64) -> u64 {", "}", |out| {
        out.line("if value < 0 {");
        out.indented(|out| {
            out.line("let wide: i128 = (value as i128) + 18446744073709551616");
            out.line("wide.downcast_unchecked()");
        });
        out.line("} else {");
        out.indented(|out| {
            out.line("value.downcast_unchecked()");
        });
        out.line("}");
    });
    out.blank();
}

fn params(params: &[Param], locals: &[String]) -> String {
    params
        .iter()
        .zip(locals)
        .map(|(param, local)| format!("{local}: {}", fe::type_name(param.ty)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_property(out: &mut SourceWriter, suite: &Suite, id: usize, property: &Property) {
    let locals = fe::locals(property.params.len());
    out.block(
        format!(
            "pub fn {}({}) -> {} {{",
            export_name(id),
            params(&property.params, &locals),
            fe::type_name(property.result)
        ),
        "}",
        |out| {
            out.line(format!(
                "{}({})",
                suite.item_path(&property.path, &property.name),
                fe::arguments(&property.params, &locals)
            ));
        },
    );
    out.blank();
}

fn from_word(ty: Ty, word: &str) -> String {
    match ty {
        Ty::Bool => format!("{word} == 1"),
        Ty::Uint(64) => word.to_string(),
        Ty::Uint(_) => format!("{word}.downcast_unchecked()"),
        Ty::Int(64) => format!("femp_signed({word})"),
        Ty::Int(_) => format!("femp_signed({word}).downcast_unchecked()"),
    }
}

fn to_word(ty: Ty, value: &str) -> String {
    match ty {
        Ty::Bool => format!("if {value} {{ 1 }} else {{ 0 }}"),
        Ty::Uint(64) => value.to_string(),
        Ty::Uint(_) => format!("{value} as u64"),
        Ty::Int(64) => format!("femp_bits({value})"),
        Ty::Int(_) => format!("femp_bits({value} as i64)"),
    }
}

fn read_args(out: &mut SourceWriter, params: &[Param]) -> Vec<String> {
    let locals = fe::locals(params.len());
    for (local, param) in locals.iter().zip(params) {
        out.line(format!(
            "let {local}: {} = {}",
            fe::type_name(param.ty),
            from_word(param.ty, "femp_word(handle, cursor)")
        ));
        out.line("cursor += 1");
    }
    locals
}

fn call(method: &Method, locals: &[String]) -> String {
    format!(
        "machine.{}({})",
        method.name,
        fe::arguments(&method.params, locals)
    )
}

fn render_machine(out: &mut SourceWriter, suite: &Suite, id: usize, machine: &Machine) {
    let ty = suite.item_path(&machine.path, &machine.name);
    out.block(
        format!("fn femp_record_{id}(_ machine: {ty}) uses (log: mut Log) {{"),
        "}",
        |out| {
            out.line("log.log_i32(value: 0)");
            for invariant in &machine.invariants {
                out.line(format!(
                    "log.log_u64(value: {})",
                    to_word(Ty::Bool, &call(invariant, &[]))
                ));
            }
            for observation in &machine.observations {
                out.line(format!(
                    "log.log_u64(value: {})",
                    to_word(observation.ty, &call(&observation.method, &[]))
                ));
            }
        },
    );
    out.blank();
    out.block(
        format!(
            "pub fn {}(script: HostText) -> u32 uses (files: mut Files, log: mut Log) {{",
            export_name(id)
        ),
        "}",
        |out| {
            out.block("let handle = match open(script.copy()) {", "}", |out| {
                out.line("Result::Ok(handle) => handle,");
                out.line("Result::Err(_) => core::panic(),");
            });
            out.line("var cursor: u32 = 0");
            let locals = read_args(out, &machine.init.params);
            out.line(format!(
                "var machine = {ty}::{}({})",
                machine.init.name,
                fe::arguments(&machine.init.params, &locals)
            ));
            out.line(format!("femp_record_{id}(machine)"));
            out.line("let steps = femp_word(handle, cursor)");
            out.line("cursor += 1");
            out.line("var step: u64 = 0");
            out.block("while step < steps {", "}", |out| {
                out.line("let command = femp_word(handle, cursor)");
                out.line("cursor += 1");
                for (index, command) in machine.commands.iter().enumerate() {
                    out.line(if index == 0 {
                        format!("if command == {index} {{")
                    } else {
                        format!("}} else if command == {index} {{")
                    });
                    out.indented(|out| {
                        let locals = read_args(out, &command.method.params);
                        out.line(format!("machine = {}", call(&command.method, &locals)));
                    });
                }
                out.line("} else {");
                out.indented(|out| {
                    out.line("core::panic()");
                });
                out.line("}");
                out.line(format!("femp_record_{id}(machine)"));
                out.line("step += 1");
            });
            out.line("close(handle)");
            out.line("0");
        },
    );
    out.blank();
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use femp_core::property::fixtures;

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
        check_fixture(
            "harness_entries.json",
            &serde_json::to_string_pretty(&entries(&fixtures::suite())).unwrap(),
        );
    }

    #[test]
    fn wide_types_get_no_export() {
        let suite = fixtures::suite();
        assert!(property_fits(&suite.properties[0]));
        assert!(!property_fits(&suite.properties[2]));
        assert!(machine_fits(&suite.machines[0]));
        assert!(!render(&suite).contains("femp_entry_2"));
    }
}
