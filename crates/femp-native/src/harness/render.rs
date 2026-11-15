//! Renders the native harness: a `main` that reads requests from standard
//! input, calls the suite's entry points and prints their results, one
//! value per line (see `femp_harness::wire`).

use femp_core::codegen::fe::{self, SourceWriter};
use femp_core::property::{Machine, Method, Property, Suite, Ty};

pub fn render(suite: &Suite) -> String {
    let mut out = fe::writer();
    out.line(fe::HEADER);
    out.line("use core::MemBuffer");
    out.line("use core::text::{parse_int, parse_uint}");
    out.line("use std::io::{Stdin, StdinReader, Stdout}");
    out.blank();
    readers(&mut out);
    for (index, property) in suite.properties.iter().enumerate() {
        render_property(&mut out, suite, suite.property_id(index), property);
    }
    for (index, machine) in suite.machines.iter().enumerate() {
        render_machine(&mut out, suite, suite.machine_id(index), machine);
    }
    render_main(&mut out, suite);
    out.finish()
}

const READS: &str = "uses (input: mut Stdin)";
const READS_AND_PRINTS: &str = "uses (input: mut Stdin, out: mut Stdout)";

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

// Declares one local per parameter, read from the request in order.
fn read_args(out: &mut SourceWriter, params: &[femp_core::property::Param]) -> Vec<String> {
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

fn call(method: &Method, receiver: &str, locals: &[String]) -> String {
    format!(
        "{receiver}.{}({})",
        method.name,
        fe::arguments(&method.params, locals)
    )
}

fn render_property(out: &mut SourceWriter, suite: &Suite, id: usize, property: &Property) {
    out.block(
        format!("fn femp_entry_{id}(_ reader: mut StdinReader) {READS_AND_PRINTS} {{"),
        "}",
        |out| {
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
                out.line(format!("println!({})", call(invariant, "machine", &[])));
            }
            for observation in &machine.observations {
                out.line(format!(
                    "println!({})",
                    call(&observation.method, "machine", &[])
                ));
            }
        },
    );
    out.blank();
    out.block(
        format!("fn femp_entry_{id}(_ reader: mut StdinReader) {READS_AND_PRINTS} {{"),
        "}",
        |out| {
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
                    let open = if index == 0 {
                        format!("if command == {index} {{")
                    } else {
                        format!("}} else if command == {index} {{")
                    };
                    out.line(open);
                    out.indented(|out| {
                        let locals = read_args(out, &command.method.params);
                        out.line(format!(
                            "machine = {}",
                            call(&command.method, "machine", &locals)
                        ));
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

fn render_main(out: &mut SourceWriter, suite: &Suite) {
    let count = suite.properties.len() + suite.machines.len();
    out.block(
        format!("pub fn main() -> i32 {READS_AND_PRINTS} {{"),
        "}",
        |out| {
            out.line("var reader = StdinReader::new()");
            out.block("while true {", "}", |out| {
                out.block("let entry = match femp_line(mut reader) {", "}", |out| {
                    out.block(
                        "Option::Some(text) => match parse_uint(text) {",
                        "},",
                        |out| {
                            out.line("Option::Some(value) => value,");
                            out.line("Option::None => {");
                            out.line("    return 2");
                            out.line("},");
                        },
                    );
                    out.line("Option::None => {");
                    out.line("    return 0");
                    out.line("},");
                });
                for id in 0..count {
                    let open = if id == 0 {
                        format!("if entry == {id} {{")
                    } else {
                        format!("}} else if entry == {id} {{")
                    };
                    out.line(open);
                    out.line(format!("    femp_entry_{id}(mut reader)"));
                }
                if count == 0 {
                    out.line("return 3");
                } else {
                    out.line("} else {");
                    out.line("    return 3");
                    out.line("}");
                }
            });
            out.line("0");
        },
    );
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use femp_core::property::fixtures;

    use super::*;

    const FIXTURE: &str = "tests/fixtures/harness.fe";

    #[test]
    fn renders_the_fixture_suite() {
        let rendered = render(&fixtures::suite());
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE);
        if std::env::var_os("FEMP_UPDATE_FIXTURES").is_some() {
            std::fs::write(&path, &rendered).unwrap();
        }
        assert_eq!(rendered, std::fs::read_to_string(path).unwrap());
    }
}
