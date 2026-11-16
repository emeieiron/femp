/// A file name made of ASCII letters, digits, `_` and `-`, as the compiler
/// names its outputs; anything else becomes `_`.
pub fn file_name(name: &str) -> String {
    file_name_or(name, "contract")
}

pub fn file_name_or(name: &str, default: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        default.to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_what_a_file_name_cannot_hold() {
        assert_eq!(file_name("Vesting"), "Vesting");
        assert_eq!(file_name("a::B<c>"), "a__B_c_");
        assert_eq!(file_name(""), "contract");
        assert_eq!(file_name_or("", "main"), "main");
    }
}
