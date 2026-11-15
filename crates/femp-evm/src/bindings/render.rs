use femp_core::codegen::typescript::{GENERATED_HEADER, SourceWriter, string_literal};
use femp_core::naming::{camel_case, pascal_case};

use super::ts_types::{param_name, struct_type_of, ts_type, tuple_type};
use crate::abi::{AbiEntry, AbiParam};

pub struct Contract<'a> {
    pub name: &'a str,
    pub entries: Vec<AbiEntry>,
    pub abi: &'a str,
    pub bytecode: &'a str,
    pub runtime_bytecode: &'a str,
}

pub fn render(contract: &Contract<'_>) -> String {
    let value_prefix = camel_case(contract.name);
    let type_prefix = pascal_case(contract.name);
    let mut out = SourceWriter::new();
    out.line(GENERATED_HEADER).blank();

    constants(&mut out, &value_prefix, contract);
    constructor(&mut out, &type_prefix, &contract.entries);
    functions(&mut out, &type_prefix, &contract.entries);
    records(&mut out, &type_prefix, "Events", "event", &contract.entries);
    records(&mut out, &type_prefix, "Errors", "error", &contract.entries);
    out.finish()
}

fn constants(out: &mut SourceWriter, prefix: &str, contract: &Contract<'_>) {
    let abi: Vec<&str> = contract.abi.trim().lines().collect();
    for (index, line) in abi.iter().enumerate() {
        let prefix = if index == 0 {
            format!("export const {prefix}Abi = ")
        } else {
            String::new()
        };
        let suffix = if index + 1 == abi.len() {
            " as const;"
        } else {
            ""
        };
        out.line(format!("{prefix}{line}{suffix}"));
    }
    out.blank();
    out.line(format!(
        "export const {prefix}Bytecode = {} as const;",
        string_literal(contract.bytecode)
    ));
    out.line(format!(
        "export const {prefix}DeployedBytecode = {} as const;",
        string_literal(contract.runtime_bytecode)
    ));
}

fn constructor(out: &mut SourceWriter, prefix: &str, entries: &[AbiEntry]) {
    let inputs = entries
        .iter()
        .find(|entry| entry.is("constructor"))
        .map(|entry| entry.inputs.as_slice())
        .unwrap_or_default();
    out.blank();
    out.line(format!(
        "export type {prefix}ConstructorArgs = {};",
        tuple_type(inputs)
    ));
}

fn functions(out: &mut SourceWriter, prefix: &str, entries: &[AbiEntry]) {
    out.blank();
    out.block(
        format!("export interface {prefix}Functions {{"),
        "}",
        |out| {
            for function in entries.iter().filter(|entry| entry.is("function")) {
                if let Some(mutability) = &function.state_mutability {
                    out.line(format!("/** {mutability} */"));
                }
                out.line(format!(
                    "{}({}): {};",
                    function.name,
                    parameters(&function.inputs),
                    return_type(&function.outputs)
                ));
            }
        },
    );
}

fn records(out: &mut SourceWriter, prefix: &str, suffix: &str, kind: &str, entries: &[AbiEntry]) {
    out.blank();
    out.block(
        format!("export interface {prefix}{suffix} {{"),
        "}",
        |out| {
            for entry in entries.iter().filter(|entry| entry.is(kind)) {
                out.line(format!(
                    "{}: {};",
                    entry.name,
                    struct_type_of(&entry.inputs)
                ));
            }
        },
    );
}

fn parameters(inputs: &[AbiParam]) -> String {
    inputs
        .iter()
        .enumerate()
        .map(|(index, input)| format!("{}: {}", param_name(input, index), ts_type(input)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn return_type(outputs: &[AbiParam]) -> String {
    match outputs {
        [] => "void".to_string(),
        [single] => ts_type(single),
        many => format!(
            "readonly [{}]",
            many.iter().map(ts_type).collect::<Vec<_>>().join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABI: &str = include_str!("../../tests/fixtures/Vesting.abi.json");
    const EXPECTED: &str = include_str!("../../tests/fixtures/Vesting.ts");

    #[test]
    fn renders_the_vesting_contract() {
        let rendered = render(&Contract {
            name: "Vesting",
            entries: serde_json::from_str(ABI).unwrap(),
            abi: ABI,
            bytecode: "0x60e0",
            runtime_bytecode: "0x6080",
        });

        if std::env::var_os("FEMP_UPDATE_FIXTURES").is_some() {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/Vesting.ts");
            std::fs::write(path, &rendered).unwrap();
        }
        assert_eq!(rendered, EXPECTED);
    }
}
