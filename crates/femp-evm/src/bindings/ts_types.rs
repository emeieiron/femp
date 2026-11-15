use crate::abi::AbiParam;

const HEX: &str = "`0x${string}`";
const MAX_NUMBER_BITS: u32 = 48;

pub fn ts_type(param: &AbiParam) -> String {
    let (base, array_depth) = split_arrays(&param.ty);
    let mut ty = base_type(base, &param.components);
    for _ in 0..array_depth {
        ty = format!("readonly {}[]", parenthesize_if_union(ty));
    }
    ty
}

pub fn tuple_type(params: &[AbiParam]) -> String {
    let members: Vec<String> = params
        .iter()
        .enumerate()
        .map(|(index, param)| format!("{}: {}", param_name(param, index), ts_type(param)))
        .collect();
    format!("[{}]", members.join(", "))
}

pub fn param_name(param: &AbiParam, index: usize) -> String {
    if param.name.is_empty() {
        format!("arg{index}")
    } else if is_reserved(&param.name) {
        format!("{}_", param.name)
    } else {
        param.name.clone()
    }
}

// `uint64[3][]` is an array of arrays; each trailing `[..]` adds a level.
fn split_arrays(ty: &str) -> (&str, usize) {
    match ty.find('[') {
        Some(index) => (&ty[..index], ty[index..].matches('[').count()),
        None => (ty, 0),
    }
}

fn base_type(base: &str, components: &[AbiParam]) -> String {
    match base {
        "bool" => "boolean".to_string(),
        "string" => "string".to_string(),
        "address" => HEX.to_string(),
        "tuple" => struct_type_of(components),
        _ if base.starts_with("bytes") => HEX.to_string(),
        _ => match integer_bits(base) {
            Some(bits) if bits <= MAX_NUMBER_BITS => "number".to_string(),
            Some(_) => "bigint".to_string(),
            None => "unknown".to_string(),
        },
    }
}

fn integer_bits(base: &str) -> Option<u32> {
    let digits = base
        .strip_prefix("uint")
        .or_else(|| base.strip_prefix("int"))?;
    if digits.is_empty() {
        Some(256)
    } else {
        digits.parse().ok()
    }
}

pub fn struct_type_of(components: &[AbiParam]) -> String {
    let fields: Vec<String> = components
        .iter()
        .enumerate()
        .map(|(index, field)| format!("{}: {}", param_name(field, index), ts_type(field)))
        .collect();
    format!("{{ {} }}", fields.join("; "))
}

fn parenthesize_if_union(ty: String) -> String {
    if ty.contains('|') {
        format!("({ty})")
    } else {
        ty
    }
}

fn is_reserved(name: &str) -> bool {
    matches!(
        name,
        "break"
            | "case"
            | "catch"
            | "class"
            | "const"
            | "continue"
            | "debugger"
            | "default"
            | "delete"
            | "do"
            | "else"
            | "enum"
            | "export"
            | "extends"
            | "false"
            | "finally"
            | "for"
            | "function"
            | "if"
            | "import"
            | "in"
            | "instanceof"
            | "new"
            | "null"
            | "return"
            | "super"
            | "switch"
            | "this"
            | "throw"
            | "true"
            | "try"
            | "typeof"
            | "var"
            | "void"
            | "while"
            | "with"
            | "yield"
            | "let"
            | "static"
            | "await"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(name: &str, ty: &str) -> AbiParam {
        AbiParam {
            name: name.to_string(),
            ty: ty.to_string(),
            components: Vec::new(),
        }
    }

    #[test]
    fn maps_scalars() {
        assert_eq!(ts_type(&param("", "uint32")), "number");
        assert_eq!(ts_type(&param("", "uint64")), "bigint");
        assert_eq!(ts_type(&param("", "int")), "bigint");
        assert_eq!(ts_type(&param("", "bool")), "boolean");
        assert_eq!(ts_type(&param("", "address")), HEX);
        assert_eq!(ts_type(&param("", "bytes32")), HEX);
        assert_eq!(ts_type(&param("", "string")), "string");
    }

    #[test]
    fn maps_arrays_and_structs() {
        assert_eq!(ts_type(&param("", "uint256[]")), "readonly bigint[]");
        assert_eq!(
            ts_type(&param("", "uint8[2][]")),
            "readonly readonly number[][]"
        );

        let pair = AbiParam {
            name: "pair".to_string(),
            ty: "tuple[]".to_string(),
            components: vec![param("owner", "address"), param("", "uint64")],
        };
        assert_eq!(
            ts_type(&pair),
            format!("readonly {{ owner: {HEX}; arg1: bigint }}[]")
        );
    }

    #[test]
    fn names_parameters_safely() {
        assert_eq!(param_name(&param("amount", "uint64"), 0), "amount");
        assert_eq!(param_name(&param("", "uint64"), 2), "arg2");
        assert_eq!(param_name(&param("function", "uint64"), 0), "function_");
        assert_eq!(
            tuple_type(&[param("to", "address"), param("amount", "uint256")]),
            format!("[to: {HEX}, amount: bigint]")
        );
    }
}
