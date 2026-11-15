//! A tokenizer for the parts of Fe that discovery reads. It knows comments,
//! string literals, identifiers, numbers and punctuation, and records the
//! line of every token because Fe ends items at line breaks.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tok {
    Ident(String),
    Number(String),
    Str(String),
    Punct(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
}

impl Token {
    pub fn is_ident(&self, word: &str) -> bool {
        matches!(&self.tok, Tok::Ident(ident) if ident == word)
    }

    pub fn is_punct(&self, punct: &str) -> bool {
        matches!(&self.tok, Tok::Punct(p) if *p == punct)
    }

    pub fn ident(&self) -> Option<&str> {
        match &self.tok {
            Tok::Ident(ident) => Some(ident),
            _ => None,
        }
    }

    pub fn text(&self) -> String {
        match &self.tok {
            Tok::Ident(text) | Tok::Number(text) => text.clone(),
            Tok::Str(text) => format!("{text:?}"),
            Tok::Punct(punct) => punct.to_string(),
        }
    }
}

const PUNCTS: [&str; 30] = [
    "..=", "::", "->", "=>", "..", "==", "!=", "<=", ">=", "&&", "||", "#", "!", "[", "]", "(",
    ")", "{", "}", "<", ">", ",", ":", ";", "=", ".", "-", "+", "*", "/",
];

pub fn tokenize(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    let mut line = 1;
    while index < bytes.len() {
        let byte = bytes[index];
        let rest = &source[index..];
        if byte == b'\n' {
            line += 1;
            index += 1;
        } else if byte.is_ascii_whitespace() {
            index += 1;
        } else if rest.starts_with("//") {
            index += rest.find('\n').unwrap_or(rest.len());
        } else if let Some(comment) = rest.strip_prefix("/*") {
            let end = comment.find("*/").map_or(rest.len(), |end| end + 4);
            line += rest[..end].matches('\n').count();
            index += end;
        } else if byte == b'"' {
            let (text, length) = string_literal(rest);
            tokens.push(Token {
                tok: Tok::Str(text),
                line,
            });
            line += rest[..length].matches('\n').count();
            index += length;
        } else if byte.is_ascii_alphabetic() || byte == b'_' {
            let length = word_length(rest);
            tokens.push(Token {
                tok: Tok::Ident(rest[..length].to_string()),
                line,
            });
            index += length;
        } else if byte.is_ascii_digit() {
            let length = word_length(rest);
            tokens.push(Token {
                tok: Tok::Number(rest[..length].to_string()),
                line,
            });
            index += length;
        } else {
            let punct = PUNCTS.iter().find(|punct| rest.starts_with(**punct));
            let length = punct.map_or_else(
                || rest.chars().next().map_or(1, char::len_utf8),
                |p| p.len(),
            );
            if let Some(punct) = punct {
                tokens.push(Token {
                    tok: Tok::Punct(punct),
                    line,
                });
            }
            index += length;
        }
    }
    tokens
}

fn word_length(text: &str) -> usize {
    text.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(text.len())
}

// Returns the literal's contents with escapes resolved and its length in
// the source, closing quote included.
fn string_literal(text: &str) -> (String, usize) {
    let mut contents = String::new();
    let mut chars = text.char_indices().skip(1);
    while let Some((offset, c)) = chars.next() {
        match c {
            '"' => return (contents, offset + 1),
            '\\' => {
                if let Some((_, escaped)) = chars.next() {
                    contents.push(match escaped {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                }
            }
            other => contents.push(other),
        }
    }
    (contents, text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(source: &str) -> Vec<String> {
        tokenize(source).iter().map(Token::text).collect()
    }

    #[test]
    fn skips_comments_and_reads_literals() {
        assert_eq!(
            texts("#[gen(a = \"0..=9\")] // note\npub fn f(_ x: u8) -> bool /* x */ {}"),
            [
                "#",
                "[",
                "gen",
                "(",
                "a",
                "=",
                "\"0..=9\"",
                ")",
                "]",
                "pub",
                "fn",
                "f",
                "(",
                "_",
                "x",
                ":",
                "u8",
                ")",
                "->",
                "bool",
                "{",
                "}"
            ]
        );
        assert_eq!(texts("x..=10 a::b"), ["x", "..=", "10", "a", "::", "b"]);
    }

    #[test]
    fn tracks_lines() {
        let tokens = tokenize("a\n/* two\nlines */ b\n\"x\ny\" c");
        let lines: Vec<usize> = tokens.iter().map(|token| token.line).collect();
        assert_eq!(lines, [1, 3, 4, 5]);
    }

    #[test]
    fn resolves_escapes() {
        assert_eq!(tokenize(r#""a\"b""#)[0].tok, Tok::Str("a\"b".to_string()));
    }
}
