/// Rewrites the first segment of paths in Fe source text.
///
/// `replace` receives each identifier that starts a path (`name::..`) and
/// returns the text to put in its place, or `None` to leave it unchanged.
/// Comments and string literals are copied verbatim, and segments in the
/// middle of a path or after a `.` are never offered for replacement.
pub fn rewrite_path_roots(source: &str, replace: &dyn Fn(&str) -> Option<String>) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut index = 0;
    // The last non-whitespace byte copied so far, which tells whether an
    // identifier continues a path (`a::b`) or a member access (`a.b`).
    let mut previous = b' ';

    while index < bytes.len() {
        let byte = bytes[index];
        let end = if bytes[index..].starts_with(b"//") {
            line_end(bytes, index)
        } else if byte == b'"' {
            previous = byte;
            string_end(bytes, index)
        } else if is_word_start(byte) {
            let end = word_end(bytes, index);
            let word = &source[index..end];
            let starts_path =
                previous != b':' && previous != b'.' && bytes[end..].starts_with(b"::");
            if let Some(replacement) = starts_path.then(|| replace(word)).flatten() {
                out.push_str(&replacement);
                previous = bytes[end - 1];
                index = end;
                continue;
            }
            previous = bytes[end - 1];
            end
        } else if byte.is_ascii_digit() {
            // Numeric literals such as `0x1f` or `10u8` are not identifiers.
            previous = byte;
            word_end(bytes, index)
        } else {
            if !byte.is_ascii_whitespace() {
                previous = byte;
            }
            index + char_len(byte)
        };
        out.push_str(&source[index..end]);
        index = end;
    }
    out
}

fn is_word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn word_end(bytes: &[u8], start: usize) -> usize {
    let length = bytes[start..]
        .iter()
        .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'_')
        .count();
    start + length
}

fn line_end(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |offset| start + offset)
}

fn string_end(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return index + 1,
            _ => index += 1,
        }
    }
    bytes.len()
}

fn char_len(first_byte: u8) -> usize {
    match first_byte {
        byte if byte >= 0xf0 => 4,
        byte if byte >= 0xe0 => 3,
        byte if byte >= 0xc0 => 2,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite(source: &str) -> String {
        rewrite_path_roots(source, &|word| match word {
            "ingot" => Some("ingot::core_lib".to_string()),
            "dep" => Some("ingot::dep_lib".to_string()),
            _ => None,
        })
    }

    #[test]
    fn rewrites_path_roots() {
        assert_eq!(
            rewrite("use ingot::error::Failure\nuse dep::{A, B}\n"),
            "use ingot::core_lib::error::Failure\nuse ingot::dep_lib::{A, B}\n"
        );
        assert_eq!(rewrite("let x = dep::f(1)"), "let x = ingot::dep_lib::f(1)");
    }

    #[test]
    fn leaves_inner_segments_and_members_alone() {
        assert_eq!(rewrite("use std::dep::Thing"), "use std::dep::Thing");
        assert_eq!(rewrite("value.dep::<u8>()"), "value.dep::<u8>()");
        assert_eq!(
            rewrite("let dep = 1\nlet ingot = dep"),
            "let dep = 1\nlet ingot = dep"
        );
    }

    #[test]
    fn leaves_comments_and_strings_alone() {
        assert_eq!(rewrite("// see dep::f\n"), "// see dep::f\n");
        assert_eq!(
            rewrite("\"dep::f \\\" ingot::g\""),
            "\"dep::f \\\" ingot::g\""
        );
        assert_eq!(
            rewrite("f(\"é\") // ü\ndep::g()"),
            "f(\"é\") // ü\ningot::dep_lib::g()"
        );
    }

    #[test]
    fn leaves_numeric_literals_alone() {
        assert_eq!(rewrite("let x = 0xdep"), "let x = 0xdep");
    }
}
