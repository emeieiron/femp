pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let starts_with_letter = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    starts_with_letter && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn looks_like_version(text: &str) -> bool {
    let core = text.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()))
}

pub fn pascal_case(name: &str) -> String {
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            let first = chars.next().map(|c| c.to_ascii_uppercase());
            first.into_iter().chain(chars).collect::<String>()
        })
        .collect()
}

pub fn camel_case(name: &str) -> String {
    let pascal = pascal_case(name);
    let mut chars = pascal.chars();
    let first = chars.next().map(|c| c.to_ascii_lowercase());
    first.into_iter().chain(chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_portable_names() {
        assert!(is_valid_name("vesting"));
        assert!(is_valid_name("vesting_core-2"));
    }

    #[test]
    fn rejects_unportable_names() {
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("2fast"));
        assert!(!is_valid_name("has space"));
        assert!(!is_valid_name("dot.ted"));
    }

    #[test]
    fn recognises_versions() {
        assert!(looks_like_version("0.1.0"));
        assert!(looks_like_version("26.6.0-rc.1"));
        assert!(!looks_like_version("../shared"));
        assert!(!looks_like_version("1.0"));
    }

    #[test]
    fn converts_case() {
        assert_eq!(pascal_case("vesting_web"), "VestingWeb");
        assert_eq!(camel_case("vested_at"), "vestedAt");
        assert_eq!(camel_case("claim"), "claim");
    }
}
