//! Reads the S-expressions a solver answers with.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sexp {
    Atom(String),
    List(Vec<Sexp>),
}

impl Sexp {
    pub fn atom(&self) -> Option<&str> {
        match self {
            Self::Atom(text) => Some(text),
            Self::List(_) => None,
        }
    }

    pub fn list(&self) -> Option<&[Sexp]> {
        match self {
            Self::List(items) => Some(items),
            Self::Atom(_) => None,
        }
    }
}

/// Whether `text` holds a complete expression: brackets balance outside
/// strings and quoted symbols.
pub fn complete(text: &str) -> bool {
    let mut depth = 0i64;
    let mut started = false;
    let mut in_string = false;
    let mut in_quote = false;
    for c in text.chars() {
        match c {
            '"' if !in_quote => in_string = !in_string,
            '|' if !in_string => in_quote = !in_quote,
            '(' if !in_string && !in_quote => {
                depth += 1;
                started = true;
            }
            ')' if !in_string && !in_quote => depth -= 1,
            _ => {}
        }
    }
    started && depth == 0 && !in_string && !in_quote
}

pub fn parse(text: &str) -> Result<Sexp, String> {
    let tokens = tokenize(text);
    let mut position = 0;
    let parsed = read(&tokens, &mut position)?;
    Ok(parsed)
}

fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '(' | ')' => tokens.push(c.to_string()),
            c if c.is_whitespace() => {}
            '|' => {
                let mut symbol = String::new();
                for next in chars.by_ref() {
                    if next == '|' {
                        break;
                    }
                    symbol.push(next);
                }
                tokens.push(symbol);
            }
            '"' => {
                let mut string = String::from('"');
                while let Some(next) = chars.next() {
                    string.push(next);
                    if next == '"' {
                        if chars.peek() == Some(&'"') {
                            string.push(chars.next().unwrap_or('"'));
                        } else {
                            break;
                        }
                    }
                }
                tokens.push(string);
            }
            _ => {
                let mut atom = c.to_string();
                while let Some(next) = chars.peek() {
                    if next.is_whitespace() || *next == '(' || *next == ')' {
                        break;
                    }
                    atom.push(*next);
                    chars.next();
                }
                tokens.push(atom);
            }
        }
    }
    tokens
}

fn read(tokens: &[String], position: &mut usize) -> Result<Sexp, String> {
    let token = tokens
        .get(*position)
        .ok_or_else(|| "unexpected end of the solver's answer".to_string())?;
    *position += 1;
    match token.as_str() {
        "(" => {
            let mut items = Vec::new();
            loop {
                match tokens.get(*position).map(String::as_str) {
                    Some(")") => {
                        *position += 1;
                        return Ok(Sexp::List(items));
                    }
                    Some(_) => items.push(read(tokens, position)?),
                    None => return Err("an unclosed list in the solver's answer".to_string()),
                }
            }
        }
        ")" => Err("an unexpected `)` in the solver's answer".to_string()),
        atom => Ok(Sexp::Atom(atom.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_model_values() {
        let parsed = parse("((v3 250)\n (v5 (- 3)) (v7 (T0.mk 1 |odd name|)))").unwrap();
        let pairs = parsed.list().unwrap();
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0].list().unwrap()[1].atom(), Some("250"));
        assert_eq!(
            pairs[1].list().unwrap()[1],
            Sexp::List(vec![Sexp::Atom("-".into()), Sexp::Atom("3".into())])
        );
        assert!(complete("((a 1)\n (b 2))"));
        assert!(!complete("((a 1)\n"));
        assert!(!complete("sat"));
    }
}
