// Kept as the Fe CLI (Apache-2.0) writes it, so that femp's manifests are
// the compiler's.

//! The build manifest that `fe build --backend wasm` writes next to a module.
//!
//! The manifest says what a host needs to know before it runs the module:
//! the hash and name of the file, the imports the host must supply, the exports
//! it may call with their Fe types, the version of the host contract, the
//! optimization level and the compiler. It is built from the bytes of the
//! module (so it cannot disagree with them) and from what the compiler knows
//! about the exported functions (their parameter names and Fe types). It
//! contains nothing that depends on the machine, the time, or the location of
//! the sources, so two builds of one program give one manifest.

use codegen::{WASM_HOST_CONTRACT_VERSION, WASM_REPLY_TYPE, WasmExportInfo};
use mir::{WasmParamShape, WasmResultShape};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// The version of the manifest format. It changes when a field is removed or
/// changes its meaning.
pub const MANIFEST_VERSION: u32 = 1;

/// What the compiler knows when a module is written.
pub struct ManifestInput<'a> {
    /// The file name of the module, without a directory.
    pub module_file: &'a str,
    pub module: &'a [u8],
    /// The optimization level as given to `-O`.
    pub optimization: &'a str,
    /// The compiler, as `fe --version` prints it.
    pub compiler: &'a str,
    pub exports: &'a [WasmExportInfo],
}

#[derive(Serialize)]
struct Manifest {
    manifest_version: u32,
    module: ModuleEntry,
    contract_version: u32,
    optimization: String,
    compiler: String,
    imports: Vec<ImportEntry>,
    exports: Vec<ExportEntry>,
}

#[derive(Serialize)]
struct ModuleEntry {
    file: String,
    sha256: String,
    size: usize,
    initial_memory_pages: u32,
}

#[derive(Serialize)]
struct FuncType {
    params: Vec<&'static str>,
    results: Vec<&'static str>,
}

#[derive(Serialize)]
struct ImportEntry {
    module: String,
    name: String,
    kind: &'static str,
    #[serde(rename = "type")]
    ty: FuncType,
}

#[derive(Serialize)]
struct ParamEntry {
    name: String,
    #[serde(rename = "type")]
    ty: String,
    wasm: Vec<&'static str>,
}

#[derive(Serialize)]
struct ResultEntry {
    #[serde(rename = "type")]
    ty: String,
    wasm: Vec<&'static str>,
}

#[derive(Serialize)]
struct ExportEntry {
    name: String,
    kind: &'static str,
    /// `"allocator"` for the three functions a module with text parameters
    /// exports for the host; absent for the program's own functions.
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Vec<ParamEntry>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Option<ResultEntry>>,
}

/// The manifest of a module as JSON text with a final newline.
pub fn render(input: &ManifestInput<'_>) -> Result<String, String> {
    let summary = Summary::read(input.module)?;
    let mut exports = Vec::new();
    let mut described = vec![false; summary.exports.len()];
    for info in input.exports {
        let position = summary
            .exports
            .iter()
            .position(|export| export.name == info.name && export.kind == ExportKind::Function)
            .ok_or_else(|| {
                format!(
                    "the module does not export the function `{}` that the compiler reported",
                    info.name
                )
            })?;
        described[position] = true;
        let ty = summary.export_type(position)?;
        exports.push(function_entry(info, ty)?);
    }

    let takes_text = input
        .exports
        .iter()
        .any(|info| info.params.iter().any(|p| p.shape == WasmParamShape::Text));
    for (position, export) in summary.exports.iter().enumerate() {
        if described[position] {
            continue;
        }
        match export.kind {
            ExportKind::Memory if export.name == mir::WASM_MEMORY_EXPORT => {
                exports.push(ExportEntry {
                    name: export.name.clone(),
                    kind: "memory",
                    role: None,
                    params: None,
                    result: None,
                })
            }
            ExportKind::Function if takes_text => {
                let ty = summary.export_type(position)?;
                exports.push(allocator_entry(&export.name, ty)?);
            }
            _ => {
                return Err(format!(
                    "the module exports `{}`, which the compiler did not report",
                    export.name
                ));
            }
        }
    }
    if takes_text {
        for name in mir::WASM_ALLOCATOR_EXPORTS {
            if !exports.iter().any(|entry| entry.name == name) {
                return Err(format!(
                    "the module has text parameters but does not export `{name}`"
                ));
            }
        }
    }
    exports.sort_by(|a, b| a.name.cmp(&b.name));

    let imports = summary
        .imports
        .iter()
        .map(|import| ImportEntry {
            module: import.module.clone(),
            name: import.name.clone(),
            kind: "function",
            ty: FuncType {
                params: summary.types[import.type_index]
                    .params
                    .iter()
                    .map(ValType::name)
                    .collect(),
                results: summary.types[import.type_index]
                    .results
                    .iter()
                    .map(ValType::name)
                    .collect(),
            },
        })
        .collect();

    let manifest = Manifest {
        manifest_version: MANIFEST_VERSION,
        module: ModuleEntry {
            file: input.module_file.to_string(),
            sha256: hex::encode(Sha256::digest(input.module)),
            size: input.module.len(),
            initial_memory_pages: summary.memory_pages.unwrap_or(0),
        },
        contract_version: WASM_HOST_CONTRACT_VERSION,
        optimization: input.optimization.to_string(),
        compiler: input.compiler.to_string(),
        imports,
        exports,
    };
    let pretty = serde_json::to_string_pretty(&manifest).map_err(|err| err.to_string())?;
    let mut text = inline_string_arrays(&pretty);
    text.push('\n');
    Ok(text)
}

/// Puts an array of strings on one line (`"wasm": ["i32", "i32"]`) instead of
/// one element per line. The text is the output of `serde_json`'s pretty
/// printer, in which a string never contains a line break.
fn inline_string_arrays(pretty: &str) -> String {
    let lines: Vec<&str> = pretty.lines().collect();
    let is_element = |line: &str| {
        let line = line.trim();
        let line = line.strip_suffix(',').unwrap_or(line);
        line.len() >= 2 && line.starts_with('"') && line.ends_with('"')
    };
    let mut out = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        let line = lines[at];
        if line.ends_with('[') {
            let mut end = at + 1;
            while end < lines.len() && is_element(lines[end]) {
                end += 1;
            }
            let closes = end < lines.len() && matches!(lines[end].trim(), "]" | "],");
            if closes && end > at + 1 {
                let items: Vec<&str> = lines[at + 1..end]
                    .iter()
                    .map(|item| item.trim().trim_end_matches(','))
                    .collect();
                out.push(format!("{line}{}{}", items.join(", "), lines[end].trim()));
                at = end + 1;
                continue;
            }
        }
        out.push(line.to_string());
        at += 1;
    }
    out.join("\n")
}

/// The entry of one of the program's functions. The raw types of the module
/// are split among the parameters by their shape, and must agree with it.
fn function_entry(info: &WasmExportInfo, ty: &FunctionType) -> Result<ExportEntry, String> {
    let mut raw = ty.params.iter();
    let mut params = Vec::new();
    for param in &info.params {
        let width = match param.shape {
            WasmParamShape::Scalar => 1,
            WasmParamShape::Text => 2,
        };
        let wasm: Vec<&'static str> = raw.by_ref().take(width).map(ValType::name).collect();
        let fits = match param.shape {
            WasmParamShape::Scalar => wasm.len() == 1,
            WasmParamShape::Text => wasm == ["i32", "i32"],
        };
        if !fits {
            return Err(format!(
                "the parameter `{}` of `{}` does not have the WebAssembly type the compiler expects",
                param.name, info.name
            ));
        }
        params.push(ParamEntry {
            name: param.name.clone(),
            ty: param.ty.clone(),
            wasm,
        });
    }
    if raw.next().is_some() {
        return Err(format!(
            "the export `{}` has more WebAssembly parameters than the compiler reported",
            info.name
        ));
    }
    let result = match &info.result {
        None => {
            if !ty.results.is_empty() {
                return Err(format!(
                    "the export `{}` returns a value but has no result type",
                    info.name
                ));
            }
            None
        }
        Some((name, shape)) => {
            let wasm: Vec<&'static str> = ty.results.iter().map(ValType::name).collect();
            let fits = match shape {
                WasmResultShape::Reply => wasm == ["i64"],
                _ => wasm.len() == 1,
            };
            if !fits {
                return Err(format!(
                    "the result of `{}` does not have the WebAssembly type the compiler expects",
                    info.name
                ));
            }
            Some(ResultEntry {
                ty: if *shape == WasmResultShape::Reply {
                    WASM_REPLY_TYPE.to_string()
                } else {
                    name.clone()
                },
                wasm,
            })
        }
    };
    Ok(ExportEntry {
        name: info.name.clone(),
        kind: "function",
        role: None,
        params: Some(params),
        result: Some(result),
    })
}

/// The entry of `alloc`, `arena_mark` or `arena_release`, whose signatures are
/// fixed by the host contract.
fn allocator_entry(name: &str, ty: &FunctionType) -> Result<ExportEntry, String> {
    let (params, result): (&[(&str, &str)], Option<&str>) = match name {
        "alloc" => (&[("size", "u32")], Some("u32")),
        "arena_mark" => (&[], Some("u32")),
        "arena_release" => (&[("mark", "u32")], None),
        _ => {
            return Err(format!(
                "the module exports `{name}`, which the compiler did not report"
            ));
        }
    };
    let raw_params: Vec<_> = ty.params.iter().map(ValType::name).collect();
    let raw_results: Vec<_> = ty.results.iter().map(ValType::name).collect();
    if raw_params != vec!["i32"; params.len()]
        || raw_results != vec!["i32"; usize::from(result.is_some())]
    {
        return Err(format!(
            "the export `{name}` does not have the WebAssembly type of the allocator contract"
        ));
    }
    Ok(ExportEntry {
        name: name.to_string(),
        kind: "function",
        role: Some("allocator"),
        params: Some(
            params
                .iter()
                .map(|&(param, ty)| ParamEntry {
                    name: param.to_string(),
                    ty: ty.to_string(),
                    wasm: vec!["i32"],
                })
                .collect(),
        ),
        result: Some(result.map(|ty| ResultEntry {
            ty: ty.to_string(),
            wasm: vec!["i32"],
        })),
    })
}

// ---------------------------------------------------------------------------
// A reader for the parts of a module that the manifest describes.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValType {
    I32,
    I64,
    F32,
    F64,
}

impl ValType {
    fn name(&self) -> &'static str {
        match self {
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::F32 => "f32",
            Self::F64 => "f64",
        }
    }
}

#[derive(Debug)]
struct FunctionType {
    params: Vec<ValType>,
    results: Vec<ValType>,
}

#[derive(Debug)]
struct Import {
    module: String,
    name: String,
    type_index: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum ExportKind {
    Function,
    Table,
    Memory,
    Global,
    Tag,
}

#[derive(Debug)]
struct Export {
    name: String,
    kind: ExportKind,
    index: usize,
}

#[derive(Debug)]
struct Summary {
    types: Vec<FunctionType>,
    imports: Vec<Import>,
    /// The type index of each function the module defines.
    defined: Vec<usize>,
    exports: Vec<Export>,
    memory_pages: Option<u32>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn byte(&mut self) -> Result<u8, String> {
        let byte = *self
            .bytes
            .get(self.pos)
            .ok_or("the module ends inside a section")?;
        self.pos += 1;
        Ok(byte)
    }

    fn u32(&mut self) -> Result<u32, String> {
        let mut result = 0u64;
        for shift in (0..35).step_by(7) {
            let byte = self.byte()?;
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return u32::try_from(result)
                    .map_err(|_| "a number in the module is too large".to_string());
            }
        }
        Err("a number in the module is too long".to_string())
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(len)
            .filter(|&end| end <= self.bytes.len());
        let end = end.ok_or("the module ends inside a section")?;
        let slice = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn name(&mut self) -> Result<String, String> {
        let len = self.u32()? as usize;
        String::from_utf8(self.take(len)?.to_vec())
            .map_err(|_| "a name in the module is not UTF-8".to_string())
    }

    fn val_type(&mut self) -> Result<ValType, String> {
        match self.byte()? {
            0x7f => Ok(ValType::I32),
            0x7e => Ok(ValType::I64),
            0x7d => Ok(ValType::F32),
            0x7c => Ok(ValType::F64),
            other => Err(format!(
                "the module uses the value type 0x{other:02x}, which the manifest does not describe"
            )),
        }
    }

    fn limits(&mut self) -> Result<u32, String> {
        let flags = self.byte()?;
        if flags > 1 {
            return Err(format!(
                "the module uses memory limits 0x{flags:02x}, which the manifest does not describe"
            ));
        }
        let minimum = self.u32()?;
        if flags == 1 {
            self.u32()?;
        }
        Ok(minimum)
    }
}

impl Summary {
    fn read(bytes: &[u8]) -> Result<Self, String> {
        if bytes.get(..8) != Some(b"\0asm\x01\0\0\0") {
            return Err("the output is not a WebAssembly module of version 1".to_string());
        }
        let mut summary = Summary {
            types: Vec::new(),
            imports: Vec::new(),
            defined: Vec::new(),
            exports: Vec::new(),
            memory_pages: None,
        };
        let mut reader = Reader { bytes, pos: 8 };
        while reader.pos < bytes.len() {
            let id = reader.byte()?;
            let size = reader.u32()? as usize;
            let body = reader.take(size)?;
            let mut section = Reader {
                bytes: body,
                pos: 0,
            };
            match id {
                1 => summary.read_types(&mut section)?,
                2 => summary.read_imports(&mut section)?,
                3 => {
                    for _ in 0..section.u32()? {
                        summary.defined.push(section.u32()? as usize);
                    }
                }
                5 => {
                    for _ in 0..section.u32()? {
                        let minimum = section.limits()?;
                        summary.memory_pages.get_or_insert(minimum);
                    }
                }
                7 => summary.read_exports(&mut section)?,
                _ => {}
            }
        }
        Ok(summary)
    }

    fn read_types(&mut self, section: &mut Reader<'_>) -> Result<(), String> {
        for _ in 0..section.u32()? {
            if section.byte()? != 0x60 {
                return Err("the module has a type that is not a function type".to_string());
            }
            let mut params = Vec::new();
            for _ in 0..section.u32()? {
                params.push(section.val_type()?);
            }
            let mut results = Vec::new();
            for _ in 0..section.u32()? {
                results.push(section.val_type()?);
            }
            self.types.push(FunctionType { params, results });
        }
        Ok(())
    }

    fn read_imports(&mut self, section: &mut Reader<'_>) -> Result<(), String> {
        for _ in 0..section.u32()? {
            let module = section.name()?;
            let name = section.name()?;
            let kind = section.byte()?;
            if kind != 0 {
                return Err(format!(
                    "the module imports `{module}.{name}`, which is not a function; the manifest describes function imports"
                ));
            }
            let type_index = section.u32()? as usize;
            if type_index >= self.types.len() {
                return Err(format!(
                    "the import `{module}.{name}` names a type that does not exist"
                ));
            }
            self.imports.push(Import {
                module,
                name,
                type_index,
            });
        }
        Ok(())
    }

    fn read_exports(&mut self, section: &mut Reader<'_>) -> Result<(), String> {
        for _ in 0..section.u32()? {
            let name = section.name()?;
            let kind = match section.byte()? {
                0 => ExportKind::Function,
                1 => ExportKind::Table,
                2 => ExportKind::Memory,
                3 => ExportKind::Global,
                4 => ExportKind::Tag,
                other => return Err(format!("the export `{name}` has the kind 0x{other:02x}")),
            };
            let index = section.u32()? as usize;
            self.exports.push(Export { name, kind, index });
        }
        Ok(())
    }

    /// The function type of the export at `position` of the export list.
    fn export_type(&self, position: usize) -> Result<&FunctionType, String> {
        let export = &self.exports[position];
        // Imported functions come first in the function index space.
        let type_index = match export.index.checked_sub(self.imports.len()) {
            None => self.imports[export.index].type_index,
            Some(defined) => *self.defined.get(defined).ok_or_else(|| {
                format!(
                    "the export `{}` names a function that does not exist",
                    export.name
                )
            })?,
        };
        self.types.get(type_index).ok_or_else(|| {
            format!(
                "the export `{}` names a type that does not exist",
                export.name
            )
        })
    }
}
