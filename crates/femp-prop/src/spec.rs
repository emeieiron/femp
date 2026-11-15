//! Parses generator specifications as written in `#[gen(param = "...")]`:
//!
//! - `any`: every value of the type;
//! - `lo..=hi` or `lo..hi`: a range, where a bound is a number (decimal or
//!   `0x` hex, `_` allowed), `min` or `max`;
//! - `name(arg, ..)`: a named generator from the registry, such as
//!   `one_of(1, 5, 9)` or `const(7)`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spec {
    Any,
    Range {
        lo: String,
        hi: String,
        inclusive: bool,
    },
    Named {
        name: String,
        args: Vec<String>,
    },
}

pub fn parse(text: &str) -> Result<Spec, String> {
    let text = text.trim();
    if text == "any" {
        return Ok(Spec::Any);
    }
    if let Some((lo, rest)) = text.split_once("..") {
        let (inclusive, hi) = match rest.strip_prefix('=') {
            Some(hi) => (true, hi),
            None => (false, rest),
        };
        let (lo, hi) = (lo.trim(), hi.trim());
        if lo.is_empty() || hi.is_empty() {
            return Err(format!("`{text}` needs both bounds, as in `0..=10`"));
        }
        return Ok(Spec::Range {
            lo: lo.to_string(),
            hi: hi.to_string(),
            inclusive,
        });
    }
    if let Some((name, rest)) = text.split_once('(') {
        let name = name.trim();
        let Some(inner) = rest.trim_end().strip_suffix(')') else {
            return Err(format!("`{text}` is missing a closing parenthesis"));
        };
        if !is_identifier(name) {
            return Err(format!("`{name}` is not a generator name"));
        }
        let args = if inner.trim().is_empty() {
            Vec::new()
        } else {
            inner.split(',').map(|arg| arg.trim().to_string()).collect()
        };
        if args.iter().any(String::is_empty) {
            return Err(format!("`{text}` has an empty argument"));
        }
        return Ok(Spec::Named {
            name: name.to_string(),
            args,
        });
    }
    Err(format!(
        "`{text}` is not a generator: expected `any`, a range such as `0..=10`, or a call such as `one_of(1, 2)`"
    ))
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(lo: &str, hi: &str, inclusive: bool) -> Spec {
        Spec::Range {
            lo: lo.into(),
            hi: hi.into(),
            inclusive,
        }
    }

    #[test]
    fn parses_every_form() {
        assert_eq!(parse(" any "), Ok(Spec::Any));
        assert_eq!(parse("0..=1_000"), Ok(range("0", "1_000", true)));
        assert_eq!(parse("-5..max"), Ok(range("-5", "max", false)));
        assert_eq!(
            parse("one_of(1, 0x10, -3)"),
            Ok(Spec::Named {
                name: "one_of".into(),
                args: vec!["1".into(), "0x10".into(), "-3".into()]
            })
        );
        assert_eq!(
            parse("const(true)"),
            Ok(Spec::Named {
                name: "const".into(),
                args: vec!["true".into()]
            })
        );
    }

    #[test]
    fn explains_malformed_specs() {
        for text in [
            "..=3",
            "1..",
            "one_of(1,",
            "one_of(1,,2)",
            "9lives(1)",
            "sometimes",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }
}
