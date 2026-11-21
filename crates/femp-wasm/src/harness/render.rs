//! Renders the WebAssembly property harness: one `#[unsafe(export)]` function
//! per property and per pure machine, reading decimal arguments from
//! `std::io::Stdin` and printing results to `std::io::Stdout` so values of any
//! width (including `u128`, `i128`, `u256`, and `i256`) cross the boundary.

use femp_core::codegen::fe::{self, SourceWriter};
use femp_core::property::{Machine, Method, Param, Property, Suite, Ty};
use serde_json::{Value as Json, json};

pub fn machine_fits(machine: &Machine) -> bool {
    !machine.uses_effects()
}

pub fn export_name(id: usize) -> String {
    format!("femp_entry_{id}")
}

const READS: &str = "uses (input: mut Stdin)";
const READS_AND_PRINTS: &str = "uses (input: mut Stdin, out: mut Stdout)";

pub fn render(suite: &Suite) -> String {
    let mut out = fe::writer();
    out.line(fe::HEADER);
    let has_entries =
        !suite.properties.is_empty() || suite.machines.iter().any(machine_fits);
    if !has_entries {
        out.line("#[unsafe(export)]");
        out.line("pub fn femp_noop() {}");
        return out.finish();
    }
    out.line("use core::MemBuffer");
    out.line("use core::text::{parse_int, parse_uint}");
    out.line("use std::io::{Stdin, StdinReader, Stdout}");
    out.blank();
    readers(&mut out);
    for (index, property) in suite.properties.iter().enumerate() {
        render_property(&mut out, suite, suite.property_id(index), property);
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
    let properties = suite.properties.iter().enumerate().map(|(index, property)| {
        json!({
            "kind": "property",
            "export": export_name(suite.property_id(index)),
            "params": property.params.len(),
        })
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

fn readers(out: &mut SourceWriter) {
    out.block(
        format!("fn femp_line(_ reader: mut StdinReader) -> Option<MemBuffer> {READS} {{"),
        "}",
        |out| {
            out.line("var text = MemBuffer::empty()");
            out.line("if reader.read_line(mut text) {");
            out.indented(|out| {
                out.line("Option::Some(text)");
            });
            out.line("} else {");
            out.indented(|out| {
                out.line("Option::None");
            });
            out.line("}");
        },
    );
    out.blank();
    for (name, parse, ty) in [
        ("femp_uint", "parse_uint", "u256"),
        ("femp_int", "parse_int", "i256"),
    ] {
        out.block(
            format!("fn {name}(_ reader: mut StdinReader) -> {ty} {READS} {{"),
            "}",
            |out| {
                out.block("match femp_line(mut reader) {", "}", |out| {
                    out.block(
                        format!("Option::Some(text) => match {parse}(text) {{"),
                        "},",
                        |out| {
                            out.line("Option::Some(value) => value,");
                            out.line("Option::None => core::panic(),");
                        },
                    );
                    out.line("Option::None => core::panic(),");
                });
            },
        );
        out.blank();
    }
}

fn read(ty: Ty) -> String {
    match ty {
        Ty::Bool => "femp_uint(mut reader) == 1".to_string(),
        Ty::Uint(256) => "femp_uint(mut reader)".to_string(),
        Ty::Int(256) => "femp_int(mut reader)".to_string(),
        Ty::Uint(_) => "femp_uint(mut reader).downcast_unchecked()".to_string(),
        Ty::Int(_) => "femp_int(mut reader).downcast_unchecked()".to_string(),
    }
}

fn read_args(out: &mut SourceWriter, params: &[Param]) -> Vec<String> {
    let locals = fe::locals(params.len());
    for (local, param) in locals.iter().zip(params) {
        out.line(format!(
            "let {local}: {} = {}",
            fe::type_name(param.ty),
            read(param.ty)
        ));
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

fn render_property(out: &mut SourceWriter, suite: &Suite, id: usize, property: &Property) {
    out.line("#[unsafe(export)]");
    out.block(
        format!("pub fn {}() {READS_AND_PRINTS} {{", export_name(id)),
        "}",
        |out| {
            out.line("var reader = StdinReader::new()");
            let locals = read_args(out, &property.params);
            let target = suite.item_path(&property.path, &property.name);
            out.line(format!(
                "println!({target}({}))",
                fe::arguments(&property.params, &locals)
            ));
        },
    );
    out.blank();
}

fn render_machine(out: &mut SourceWriter, suite: &Suite, id: usize, machine: &Machine) {
    let ty = suite.item_path(&machine.path, &machine.name);
    out.block(
        format!("fn femp_record_{id}(_ machine: {ty}) uses (out: mut Stdout) {{"),
        "}",
        |out| {
            out.line("println!(\"@\")");
            for invariant in &machine.invariants {
                out.line(format!("println!({})", call(invariant, &[])));
            }
            for observation in &machine.observations {
                out.line(format!("println!({})", call(&observation.method, &[])));
            }
        },
    );
    out.blank();
    out.line("#[unsafe(export)]");
    out.block(
        format!("pub fn {}() {READS_AND_PRINTS} {{", export_name(id)),
        "}",
        |out| {
            out.line("var reader = StdinReader::new()");
            let locals = read_args(out, &machine.init.params);
            out.line(format!(
                "var machine = {ty}::{}({})",
                machine.init.name,
                fe::arguments(&machine.init.params, &locals)
            ));
            out.line(format!("femp_record_{id}(machine)"));
            out.line("let steps = femp_uint(mut reader)");
            out.line("var step: u256 = 0");
            out.block("while step < steps {", "}", |out| {
                out.line("let command = femp_uint(mut reader)");
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
    fn wide_types_are_exported() {
        let suite = fixtures::suite();
        assert!(machine_fits(&suite.machines[0]));
        assert!(render(&suite).contains("pub fn femp_entry_2()"));
    }
}
