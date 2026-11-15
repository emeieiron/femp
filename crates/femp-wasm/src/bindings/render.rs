use femp_core::codegen::typescript::{GENERATED_HEADER, SourceWriter, string_literal};
use femp_core::naming::{camel_case, pascal_case};

use crate::manifest::{Export, WasmManifest};

pub const HOST_SCRIPT_FILE: &str = "host.js";
pub const HOST_TYPES_FILE: &str = "host.d.ts";

pub fn script(manifest: &WasmManifest, manifest_json: &str) -> String {
    let class = class_name(manifest);
    let module_file = string_literal(&manifest.module.file);
    let mut out = SourceWriter::new();
    out.line(GENERATED_HEADER).blank();
    out.line(format!(
        "import {{ instantiate }} from \"./{HOST_SCRIPT_FILE}\";"
    ));
    out.blank();
    out.line(format!(
        "export const manifest = {};",
        manifest_json.trim_end()
    ));
    out.blank();
    out.block(format!("export class {class} {{"), "}", |out| {
        out.block(
            format!(
                "static async load(source = new URL({module_file}, import.meta.url), options = {{}}) {{"
            ),
            "}",
            |out| {
                out.line(format!(
                    "return new {class}(await instantiate(source, {{ ...options, manifest }}));"
                ));
            },
        );
        out.blank();
        out.block("constructor(guest) {", "}", |out| {
            out.line("this.guest = guest;");
        });
        for function in manifest.functions() {
            let params = parameter_names(function).join(", ");
            let call_args = std::iter::once(string_literal(&function.name))
                .chain(parameter_names(function))
                .collect::<Vec<_>>()
                .join(", ");
            out.blank();
            out.block(
                format!("{}({params}) {{", camel_case(&function.name)),
                "}",
                |out| {
                    out.line(format!("return this.guest.call({call_args});"));
                },
            );
        }
    });
    out.finish()
}

pub fn types(manifest: &WasmManifest) -> String {
    let class = class_name(manifest);
    let mut out = SourceWriter::new();
    out.line(GENERATED_HEADER).blank();
    out.line(format!(
        "import type {{ Guest, HostOptions, Manifest, ModuleSource, Reply }} from \"./{HOST_SCRIPT_FILE}\";"
    ));
    out.blank();
    out.line("export declare const manifest: Manifest;");
    out.blank();
    out.block(format!("export declare class {class} {{"), "}", |out| {
        out.line("/** Loads the module that sits next to this file unless a source is given. */");
        out.line(format!(
            "static load(source?: ModuleSource, options?: Omit<HostOptions, \"manifest\">): Promise<{class}>;"
        ));
        out.line("constructor(guest: Guest);");
        out.line("readonly guest: Guest;");
        for function in manifest.functions() {
            let params = function
                .params
                .iter()
                .map(|param| format!("{}: {}", camel_case(&param.name), ts_type(&param.ty)))
                .collect::<Vec<_>>()
                .join(", ");
            let result = function
                .result
                .as_ref()
                .map_or("void", |result| ts_type(&result.ty));
            out.line(format!(
                "{}({params}): {result};",
                camel_case(&function.name)
            ));
        }
    });
    // `Reply` is only referenced by modules that return one.
    out.blank().line("export type { Reply };");
    out.finish()
}

fn class_name(manifest: &WasmManifest) -> String {
    let stem = manifest
        .module
        .file
        .strip_suffix(".wasm")
        .unwrap_or(&manifest.module.file);
    pascal_case(stem)
}

fn parameter_names(function: &Export) -> Vec<String> {
    function
        .params
        .iter()
        .map(|param| camel_case(&param.name))
        .collect()
}

// Mirrors the conversions of the host runtime: 64-bit integers cross as
// `bigint`, every narrower integer as `number`.
fn ts_type(fe_type: &str) -> &'static str {
    match fe_type {
        "bool" => "boolean",
        "u64" | "i64" => "bigint",
        "HostText" => "string | Uint8Array",
        "Reply" => "Reply",
        _ => "number",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = include_str!("../../tests/fixtures/vesting_web.manifest.json");
    const EXPECTED_TYPES: &str = include_str!("../../tests/fixtures/vesting_web.d.ts");
    const EXPECTED_SCRIPT: &str = include_str!("../../tests/fixtures/vesting_web.js");

    fn update(name: &str, contents: &str) {
        if std::env::var_os("FEMP_UPDATE_FIXTURES").is_some() {
            let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
            std::fs::write(path, contents).unwrap();
        }
    }

    #[test]
    fn renders_declarations_for_the_vesting_module() {
        let rendered = types(&WasmManifest::parse(MANIFEST).unwrap());
        update("vesting_web.d.ts", &rendered);
        assert_eq!(rendered, EXPECTED_TYPES);
    }

    #[test]
    fn renders_the_wrapper_for_the_vesting_module() {
        let rendered = script(&WasmManifest::parse(MANIFEST).unwrap(), MANIFEST);
        update("vesting_web.js", &rendered);
        assert_eq!(rendered, EXPECTED_SCRIPT);
    }

    #[test]
    fn maps_boundary_types() {
        assert_eq!(ts_type("u64"), "bigint");
        assert_eq!(ts_type("u32"), "number");
        assert_eq!(ts_type("usize"), "number");
        assert_eq!(ts_type("bool"), "boolean");
        assert_eq!(ts_type("HostText"), "string | Uint8Array");
        assert_eq!(ts_type("Reply"), "Reply");
    }
}
