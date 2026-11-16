//! Reads the items discovery cares about from one module's tokens:
//! functions with their attributes and signatures, structs, contracts, and
//! inherent `impl` blocks. Everything else is skipped. The parser is deliberately
//! shallow: signatures it records are later compiled into a harness, so the
//! Fe compiler checks anything this reads wrongly.

use super::lex::{Tok, Token};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attr {
    pub name: String,
    pub args: Vec<AttrArg>,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrArg {
    pub key: Option<String>,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawParam {
    pub label: Option<String>,
    pub name: String,
    pub ty: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEffect {
    pub ty: String,
    pub mutable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFn {
    pub attrs: Vec<Attr>,
    pub public: bool,
    pub name: String,
    pub line: usize,
    pub generic: bool,
    pub has_self: bool,
    pub params: Vec<RawParam>,
    pub result: Option<String>,
    pub effects: Vec<RawEffect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawStruct {
    pub attrs: Vec<Attr>,
    pub public: bool,
    pub name: String,
    pub line: usize,
}

/// A `contract` declaration; its body is skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawContract {
    pub public: bool,
    pub name: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawImpl {
    pub type_name: String,
    pub methods: Vec<RawFn>,
}

/// The items of one module path. `conditional` is set when the module is
/// compiled for some targets only (`#![cfg(..)]`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawModule {
    pub path: Vec<String>,
    pub conditional: bool,
    pub fns: Vec<RawFn>,
    pub structs: Vec<RawStruct>,
    pub contracts: Vec<RawContract>,
    pub impls: Vec<RawImpl>,
}

pub fn parse_module(tokens: &[Token], path: Vec<String>) -> Vec<RawModule> {
    let mut parser = Parser {
        tokens,
        position: 0,
    };
    let mut modules = Vec::new();
    parser.items(path, false, &mut modules);
    modules
}

struct Parser<'a> {
    tokens: &'a [Token],
    position: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&'a Token> {
        self.tokens.get(self.position)
    }

    fn peek_at(&self, offset: usize) -> Option<&'a Token> {
        self.tokens.get(self.position + offset)
    }

    fn next(&mut self) -> Option<&'a Token> {
        let token = self.tokens.get(self.position);
        self.position += 1;
        token
    }

    fn at_punct(&self, punct: &str) -> bool {
        self.peek().is_some_and(|token| token.is_punct(punct))
    }

    fn at_ident(&self, word: &str) -> bool {
        self.peek().is_some_and(|token| token.is_ident(word))
    }

    fn eat_punct(&mut self, punct: &str) -> bool {
        let found = self.at_punct(punct);
        if found {
            self.position += 1;
        }
        found
    }

    fn items(&mut self, path: Vec<String>, nested: bool, out: &mut Vec<RawModule>) {
        let mut module = RawModule {
            path: path.clone(),
            ..RawModule::default()
        };
        while let Some(token) = self.peek() {
            if token.is_punct("}") {
                if nested {
                    self.position += 1;
                    break;
                }
                self.position += 1;
                continue;
            }
            if token.is_punct("#") && self.peek_at(1).is_some_and(|t| t.is_punct("!")) {
                let attr = self.inner_attr();
                module.conditional |= attr.is_some_and(|attr| attr.name == "cfg");
                continue;
            }
            let attrs = self.attrs();
            let public = self.visibility();
            let Some(keyword) = self.peek() else { break };
            match keyword.ident() {
                Some("fn") => {
                    if let Some(function) = self.function(attrs, public) {
                        module.fns.push(function);
                    }
                }
                Some("struct") => {
                    if let Some(item) = self.structure(attrs, public) {
                        module.structs.push(item);
                    }
                }
                Some("impl") => {
                    if let Some(item) = self.implementation() {
                        module.impls.push(item);
                    }
                }
                Some("contract") => {
                    if let Some(item) = self.contract(public) {
                        module.contracts.push(item);
                    }
                }
                Some("mod") if self.peek_at(2).is_some_and(|t| t.is_punct("{")) => {
                    self.position += 1;
                    let name = self
                        .next()
                        .and_then(Token::ident)
                        .unwrap_or_default()
                        .to_string();
                    self.position += 1;
                    let mut inner = path.clone();
                    inner.push(name);
                    self.items(inner, true, out);
                }
                _ => self.skip_item(),
            }
        }
        out.push(module);
    }

    fn attrs(&mut self) -> Vec<Attr> {
        let mut attrs = Vec::new();
        while self.at_punct("#") && self.peek_at(1).is_some_and(|t| t.is_punct("[")) {
            self.position += 2;
            if let Some(attr) = self.attr_body() {
                attrs.push(attr);
            }
        }
        attrs
    }

    fn inner_attr(&mut self) -> Option<Attr> {
        self.position += 2;
        if !self.eat_punct("[") {
            return None;
        }
        self.attr_body()
    }

    // Reads `name(args)]` after the opening `#[`.
    fn attr_body(&mut self) -> Option<Attr> {
        let line = self.peek()?.line;
        let mut name = String::new();
        while let Some(token) = self.peek() {
            match &token.tok {
                Tok::Ident(ident) => name.push_str(ident),
                Tok::Punct("::") => name.push_str("::"),
                _ => break,
            }
            self.position += 1;
        }
        let mut args = Vec::new();
        if self.at_punct("(") {
            let inner = self.balanced("(", ")");
            for entry in split_commas(&inner[1..inner.len().saturating_sub(1)]) {
                args.push(attr_arg(entry));
            }
        }
        // Anything else up to the closing bracket, such as `= value`.
        let mut depth = 0;
        while let Some(token) = self.next() {
            match &token.tok {
                Tok::Punct("[") => depth += 1,
                Tok::Punct("]") if depth == 0 => break,
                Tok::Punct("]") => depth -= 1,
                _ => {}
            }
        }
        Some(Attr { name, args, line })
    }

    fn visibility(&mut self) -> bool {
        if !self.at_ident("pub") {
            return false;
        }
        self.position += 1;
        if self.at_punct("(") {
            // `pub(ingot)` and similar are not visible to a harness ingot.
            self.balanced("(", ")");
            return false;
        }
        true
    }

    fn function(&mut self, attrs: Vec<Attr>, public: bool) -> Option<RawFn> {
        self.position += 1;
        let name_token = self.next()?;
        let name = name_token.ident()?.to_string();
        let generic = self.at_punct("<");
        if generic {
            self.balanced("<", ">");
        }
        if !self.at_punct("(") {
            self.skip_item();
            return None;
        }
        let params_tokens = self.balanced("(", ")");
        let mut has_self = false;
        let mut params = Vec::new();
        for entry in split_commas(&params_tokens[1..params_tokens.len() - 1]) {
            let words: Vec<&str> = entry.iter().filter_map(Token::ident).collect();
            if entry.iter().all(|t| t.ident().is_some()) && words.last() == Some(&"self") {
                has_self = true;
            } else if let Some(param) = param(entry) {
                params.push(param);
            }
        }
        let mut result = None;
        if self.eat_punct("->") {
            let mut tokens = Vec::new();
            while let Some(token) = self.peek() {
                if token.is_punct("{") || token.is_ident("uses") || token.is_ident("where") {
                    break;
                }
                tokens.push(token.clone());
                self.position += 1;
            }
            result = Some(join(&tokens));
        }
        let mut effects = Vec::new();
        if self.at_ident("uses") {
            self.position += 1;
            let clause = if self.at_punct("(") {
                let tokens = self.balanced("(", ")");
                tokens[1..tokens.len() - 1].to_vec()
            } else {
                let mut tokens = Vec::new();
                while let Some(token) = self
                    .peek()
                    .filter(|t| !t.is_punct("{") && !t.is_ident("where"))
                {
                    tokens.push(token.clone());
                    self.position += 1;
                }
                tokens
            };
            effects = split_commas(&clause)
                .into_iter()
                .filter_map(effect)
                .collect();
        }
        while let Some(token) = self.peek() {
            if token.is_punct("{") {
                self.balanced("{", "}");
                break;
            }
            self.position += 1;
        }
        Some(RawFn {
            attrs,
            public,
            name,
            line: name_token.line,
            generic,
            has_self,
            params,
            result,
            effects,
        })
    }

    fn structure(&mut self, attrs: Vec<Attr>, public: bool) -> Option<RawStruct> {
        let keyword_line = self.next()?.line;
        let name_token = self.next()?;
        let name = name_token.ident()?.to_string();
        if self.at_punct("<") {
            self.balanced("<", ">");
        }
        if self.at_punct("{") {
            self.balanced("{", "}");
        } else if self.at_punct("(") {
            self.balanced("(", ")");
        } else {
            self.skip_line(keyword_line);
        }
        Some(RawStruct {
            attrs,
            public,
            name,
            line: name_token.line,
        })
    }

    fn contract(&mut self, public: bool) -> Option<RawContract> {
        self.position += 1;
        let name_token = self.next()?;
        let name = name_token.ident()?.to_string();
        while let Some(token) = self.peek() {
            if token.is_punct("{") {
                self.balanced("{", "}");
                break;
            }
            if token.is_punct("(") {
                self.balanced("(", ")");
            } else {
                self.position += 1;
            }
        }
        Some(RawContract {
            public,
            name,
            line: name_token.line,
        })
    }

    fn implementation(&mut self) -> Option<RawImpl> {
        self.position += 1;
        if self.at_punct("<") {
            self.balanced("<", ">");
        }
        let mut header = Vec::new();
        while let Some(token) = self.peek() {
            if token.is_punct("{") {
                break;
            }
            header.push(token.clone());
            self.position += 1;
        }
        if header.iter().any(|token| token.is_ident("for")) || !self.at_punct("{") {
            if self.at_punct("{") {
                self.balanced("{", "}");
            }
            return None;
        }
        let type_name = header.first()?.ident()?.to_string();
        self.position += 1;
        let mut methods = Vec::new();
        while let Some(token) = self.peek() {
            if token.is_punct("}") {
                self.position += 1;
                break;
            }
            let attrs = self.attrs();
            let public = self.visibility();
            if self.at_ident("fn") {
                if let Some(method) = self.function(attrs, public) {
                    methods.push(method);
                }
            } else {
                self.skip_item();
            }
        }
        Some(RawImpl { type_name, methods })
    }

    /// Consumes a bracketed group, returning it with its delimiters.
    fn balanced(&mut self, open: &str, close: &str) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut depth = 0usize;
        while let Some(token) = self.next() {
            tokens.push(token.clone());
            if token.is_punct(open) {
                depth += 1;
            } else if token.is_punct(close) {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    break;
                }
            }
        }
        tokens
    }

    /// Skips an item this parser does not read. An item with a body ends
    /// with it; others end at the first line break outside brackets.
    fn skip_item(&mut self) {
        let start = self.position;
        let Some(first) = self.peek() else { return };
        let mut depth = 0usize;
        let mut last_line = first.line;
        while let Some(token) = self.peek() {
            let consumed = self.position > start;
            if depth == 0 && consumed && (token.line > last_line || token.is_punct("}")) {
                return;
            }
            self.position += 1;
            last_line = token.line;
            match &token.tok {
                Tok::Punct("(" | "[" | "{") => depth += 1,
                Tok::Punct(")" | "]") => depth = depth.saturating_sub(1),
                Tok::Punct("}") => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    fn skip_line(&mut self, line: usize) {
        while self.peek().is_some_and(|token| token.line == line) {
            self.position += 1;
        }
    }
}

fn split_commas(tokens: &[Token]) -> Vec<&[Token]> {
    let mut entries = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, token) in tokens.iter().enumerate() {
        match &token.tok {
            Tok::Punct("(" | "[" | "{" | "<") => depth += 1,
            Tok::Punct(")" | "]" | "}" | ">") => depth = depth.saturating_sub(1),
            Tok::Punct(",") if depth == 0 => {
                entries.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    entries.push(&tokens[start..]);
    entries.retain(|entry| !entry.is_empty());
    entries
}

fn attr_arg(tokens: &[Token]) -> AttrArg {
    let value = |tokens: &[Token]| match tokens {
        [
            Token {
                tok: Tok::Str(text),
                ..
            },
        ] => text.clone(),
        _ => join(tokens),
    };
    match tokens {
        [key, eq, rest @ ..] if key.ident().is_some() && eq.is_punct("=") => AttrArg {
            key: key.ident().map(str::to_string),
            value: value(rest),
        },
        _ => AttrArg {
            key: None,
            value: value(tokens),
        },
    }
}

// `_ name: T`, `label name: T` or `name: T`, possibly after `mut`/`own`.
fn param(tokens: &[Token]) -> Option<RawParam> {
    let colon = tokens.iter().position(|token| token.is_punct(":"))?;
    let names: Vec<&str> = tokens[..colon]
        .iter()
        .filter_map(Token::ident)
        .filter(|word| !matches!(*word, "mut" | "own" | "ref"))
        .collect();
    let (label, name) = match names.as_slice() {
        ["_", name] => (None, *name),
        [label, name] => (Some(label.to_string()), *name),
        [name] => (Some(name.to_string()), *name),
        _ => return None,
    };
    Some(RawParam {
        label,
        name: name.to_string(),
        ty: join(&tokens[colon + 1..]),
    })
}

// `name: mut Type`, `mut Type`, or `Type`; contract-field forms such as
// `mut store` name no type and are kept as written.
fn effect(tokens: &[Token]) -> Option<RawEffect> {
    let after_colon = match tokens.iter().position(|token| token.is_punct(":")) {
        Some(colon) => &tokens[colon + 1..],
        None => tokens,
    };
    let mutable = after_colon
        .first()
        .is_some_and(|token| token.is_ident("mut"));
    let ty = after_colon
        .iter()
        .skip(mutable as usize)
        .map(Token::text)
        .collect::<String>();
    (!ty.is_empty()).then_some(RawEffect { ty, mutable })
}

fn join(tokens: &[Token]) -> String {
    let mut text = String::new();
    let mut previous_word = false;
    for token in tokens {
        let word = matches!(&token.tok, Tok::Ident(_) | Tok::Number(_) | Tok::Str(_));
        if word && previous_word {
            text.push(' ');
        }
        text.push_str(&token.text());
        if token.is_punct(",") {
            text.push(' ');
        }
        previous_word = word;
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::lex::tokenize;

    fn parse(source: &str) -> Vec<RawModule> {
        parse_module(&tokenize(source), Vec::new())
    }

    #[test]
    fn reads_function_signatures() {
        let modules = parse(
            "use a::{b,\n c}\nconst X: u64 = 1\n\
             #[property(cases = 50)]\n#[gen(a = \"0..=9\", b = \"any\")]\n\
             pub fn adds(a: u64, _ b: u64, with c: i8) -> bool {\n  a + b > 0\n}\n\
             fn hidden() uses (clock: mut Clock, log: Log) {}\n",
        );
        let functions = &modules[0].fns;
        assert_eq!(functions.len(), 2);
        let adds = &functions[0];
        assert!(adds.public);
        assert_eq!(adds.line, 6);
        assert_eq!(adds.attrs[0].name, "property");
        assert_eq!(
            adds.attrs[0].args,
            [AttrArg {
                key: Some("cases".into()),
                value: "50".into()
            }]
        );
        assert_eq!(adds.attrs[1].args[0].value, "0..=9");
        let shapes: Vec<(Option<&str>, &str, &str)> = adds
            .params
            .iter()
            .map(|p| (p.label.as_deref(), p.name.as_str(), p.ty.as_str()))
            .collect();
        assert_eq!(
            shapes,
            [
                (Some("a"), "a", "u64"),
                (None, "b", "u64"),
                (Some("with"), "c", "i8")
            ]
        );
        assert_eq!(adds.result.as_deref(), Some("bool"));
        let hidden = &functions[1];
        assert!(!hidden.public);
        assert_eq!(
            hidden.effects,
            [
                RawEffect {
                    ty: "Clock".into(),
                    mutable: true
                },
                RawEffect {
                    ty: "Log".into(),
                    mutable: false
                }
            ]
        );
    }

    #[test]
    fn reads_machines_and_skips_trait_impls() {
        let modules = parse(
            "#[machine(steps = \"1..=4\")]\npub struct Counter { pub value: u64 }\n\
             impl Copy for Counter {}\n\
             impl Counter {\n  #[init]\n  pub fn new(start: u64) -> Counter { Counter { value: start } }\n\
             \n  #[command(weight = 2)]\n  pub fn add(self, amount: u64) -> Self { self }\n\
             const LIMIT: u64 = 3\n  fn helper(self) -> u64 { 1 }\n}\n\
             pub struct Plain\n",
        );
        let module = &modules[0];
        assert_eq!(module.structs.len(), 2);
        assert_eq!(module.structs[0].attrs[0].name, "machine");
        assert_eq!(module.impls.len(), 1);
        let methods = &module.impls[0].methods;
        let names: Vec<&str> = methods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["new", "add", "helper"]);
        assert!(!methods[0].has_self);
        assert!(methods[1].has_self);
        assert_eq!(methods[1].params[0].name, "amount");
        assert_eq!(methods[1].result.as_deref(), Some("Self"));
    }

    #[test]
    fn reads_inline_modules_and_conditional_modules() {
        let modules = parse(
            "#![cfg(target = \"evm\")]\npub mod inner {\n  pub fn f() {}\n}\npub fn g() -> Result<E, u64> {}\n",
        );
        let inner = modules.iter().find(|m| m.path == ["inner"]).unwrap();
        assert_eq!(inner.fns[0].name, "f");
        let root = modules.iter().find(|m| m.path.is_empty()).unwrap();
        assert!(root.conditional);
        assert_eq!(root.fns[0].result.as_deref(), Some("Result<E, u64>"));
    }

    #[test]
    fn skips_generic_and_restricted_functions() {
        let modules = parse("pub fn id<T>(x: T) -> T { x }\npub(ingot) fn inner() {}\n");
        assert!(modules[0].fns[0].generic);
        assert!(!modules[0].fns[1].public);
    }
}
