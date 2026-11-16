//! Values of a model: primitives as `Value`s, anything else as the text a
//! report shows, such as `Counter { big: 2 }`.

use femp_core::property::{Ty, Value};
use femp_vir::{Type, TypeDef, TypeTable};

use super::sexp::Sexp;

fn number(sexp: &Sexp) -> Option<String> {
    match sexp {
        Sexp::Atom(text) => Some(text.clone()),
        Sexp::List(items) => match items.as_slice() {
            [Sexp::Atom(minus), Sexp::Atom(magnitude)] if minus == "-" => {
                Some(format!("-{magnitude}"))
            }
            _ => None,
        },
    }
}

pub fn primitive(sexp: &Sexp, ty: Ty) -> Option<Value> {
    Value::parse(ty, &number(sexp)?)
}

fn base_name(name: &str) -> &str {
    name.split('<').next().unwrap_or(name).trim()
}

/// The constructor name and arguments of a datatype value.
fn application(sexp: &Sexp) -> Option<(&str, &[Sexp])> {
    match sexp {
        Sexp::Atom(name) => Some((name, &[])),
        Sexp::List(items) => {
            let (head, args) = items.split_first()?;
            Some((head.atom()?, args))
        }
    }
}

pub fn show(sexp: &Sexp, ty: &Type, table: &TypeTable) -> String {
    if let Some(expanded) = expand_lets(sexp) {
        return show(&expanded, ty, table);
    }
    let fallback = || render(sexp);
    match ty {
        Type::Bool => sexp.atom().map(str::to_string).unwrap_or_else(fallback),
        Type::Int { .. } => number(sexp).unwrap_or_else(fallback),
        Type::Unit => "()".to_string(),
        Type::Tuple(items) => match application(sexp) {
            Some((_, args)) if args.len() == items.len() => {
                let parts: Vec<String> = args
                    .iter()
                    .zip(items)
                    .map(|(arg, item)| show(arg, item, table))
                    .collect();
                format!("({})", parts.join(", "))
            }
            _ => fallback(),
        },
        Type::Adt(id) => {
            let Some((head, args)) = application(sexp) else {
                return fallback();
            };
            match table.get(*id) {
                TypeDef::Struct { name, fields } => {
                    if fields.is_empty() {
                        return base_name(name).to_string();
                    }
                    let parts: Vec<String> = fields
                        .iter()
                        .zip(args)
                        .map(|(field, arg)| {
                            format!("{}: {}", field.name, show(arg, &field.ty, table))
                        })
                        .collect();
                    format!("{} {{ {} }}", base_name(name), parts.join(", "))
                }
                TypeDef::Enum { name, variants } => {
                    let index = head
                        .rsplit_once(".v")
                        .and_then(|(_, index)| index.parse::<usize>().ok());
                    let Some(variant) = index.and_then(|index| variants.get(index)) else {
                        return fallback();
                    };
                    let label = format!("{}::{}", base_name(name), variant.name);
                    if variant.fields.is_empty() {
                        label
                    } else {
                        let parts: Vec<String> = args
                            .iter()
                            .zip(&variant.fields)
                            .map(|(arg, field)| show(arg, field, table))
                            .collect();
                        format!("{label}({})", parts.join(", "))
                    }
                }
            }
        }
        Type::Array { elem, len } => array(sexp, elem, *len, table).unwrap_or_else(fallback),
        Type::Map { key, value } => map(sexp, key, value, table).unwrap_or_else(fallback),
    }
}

/// Maps come back like arrays: a constant with stores on top. They show the
/// entries that differ from the rest, then what every other key holds.
fn map(sexp: &Sexp, key: &Type, value: &Type, table: &TypeTable) -> Option<String> {
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut current = sexp;
    loop {
        let items = current.list()?;
        match items {
            [Sexp::Atom(store), inner, at, held] if store == "store" => {
                let at = show(at, key, table);
                if !entries.iter().any(|(known, _)| *known == at) {
                    entries.push((at, show(held, value, table)));
                }
                current = inner;
            }
            [head, held]
                if head
                    .list()
                    .is_some_and(|h| h.first().and_then(Sexp::atom) == Some("as")) =>
            {
                let rest = show(held, value, table);
                let mut parts: Vec<String> = entries
                    .into_iter()
                    .rev()
                    .filter(|(_, held)| *held != rest)
                    .map(|(at, held)| format!("{at}: {held}"))
                    .collect();
                parts.push(format!("_: {rest}"));
                return Some(format!("{{{}}}", parts.join(", ")));
            }
            _ => return None,
        }
    }
}

/// Arrays come back as a constant array with stores on top.
fn array(sexp: &Sexp, elem: &Type, len: usize, table: &TypeTable) -> Option<String> {
    let mut cells: Vec<Option<String>> = vec![None; len];
    let mut current = sexp;
    loop {
        let items = current.list()?;
        match items {
            [Sexp::Atom(store), inner, index, value] if store == "store" => {
                let index: usize = number(index)?.parse().ok()?;
                if index < len && cells[index].is_none() {
                    cells[index] = Some(show(value, elem, table));
                }
                current = inner;
            }
            [head, value]
                if head
                    .list()
                    .is_some_and(|h| h.first().and_then(Sexp::atom) == Some("as")) =>
            {
                let default = show(value, elem, table);
                let cells: Vec<String> = cells
                    .into_iter()
                    .map(|cell| cell.unwrap_or_else(|| default.clone()))
                    .collect();
                return Some(format!("[{}]", cells.join(", ")));
            }
            _ => return None,
        }
    }
}

/// A model value written with `let` bindings, with every binding
/// substituted; `None` when the value has none.
fn expand_lets(sexp: &Sexp) -> Option<Sexp> {
    let [Sexp::Atom(head), Sexp::List(bindings), body] = sexp.list()? else {
        return None;
    };
    if head != "let" {
        return None;
    }
    let mut bound: Vec<(String, Sexp)> = Vec::new();
    for binding in bindings {
        let [Sexp::Atom(name), value] = binding.list()? else {
            return None;
        };
        let value = substitute(value, &bound);
        bound.push((name.clone(), expand_lets(&value).unwrap_or(value)));
    }
    let body = substitute(body, &bound);
    Some(expand_lets(&body).unwrap_or(body))
}

fn substitute(sexp: &Sexp, bound: &[(String, Sexp)]) -> Sexp {
    match sexp {
        Sexp::Atom(name) => bound
            .iter()
            .rev()
            .find(|(known, _)| known == name)
            .map_or_else(|| sexp.clone(), |(_, value)| value.clone()),
        Sexp::List(items) => Sexp::List(items.iter().map(|item| substitute(item, bound)).collect()),
    }
}

fn render(sexp: &Sexp) -> String {
    match sexp {
        Sexp::Atom(text) => text.clone(),
        Sexp::List(items) => format!(
            "({})",
            items.iter().map(render).collect::<Vec<_>>().join(" ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use femp_vir::{Field, Variant};

    use super::*;
    use crate::solver::sexp::parse;

    #[test]
    fn shows_structs_enums_and_primitives() {
        let mut table = TypeTable::default();
        let counter = table.push(TypeDef::Struct {
            name: "Counter".into(),
            fields: vec![Field {
                name: "big".into(),
                ty: Type::uint(64),
            }],
        });
        let shape = table.push(TypeDef::Enum {
            name: "Option<u8>".into(),
            variants: vec![
                Variant {
                    name: "None".into(),
                    fields: vec![],
                },
                Variant {
                    name: "Some".into(),
                    fields: vec![Type::uint(8)],
                },
            ],
        });
        assert_eq!(
            show(&parse("(T0.mk 2)").unwrap(), &Type::Adt(counter), &table),
            "Counter { big: 2 }"
        );
        assert_eq!(
            show(&parse("(T1.v1 7)").unwrap(), &Type::Adt(shape), &table),
            "Option::Some(7)"
        );
        assert_eq!(
            show(&parse("T1.v0").unwrap(), &Type::Adt(shape), &table),
            "Option::None"
        );
        assert_eq!(
            primitive(&parse("(- 9223372036854775808)").unwrap(), Ty::Int(64))
                .unwrap()
                .to_string(),
            "-9223372036854775808"
        );
        let array = Type::Array {
            elem: Box::new(Type::uint(8)),
            len: 3,
        };
        assert_eq!(
            show(
                &parse("(store ((as const (Array Int Int)) 0) 1 5)").unwrap(),
                &array,
                &table
            ),
            "[0, 5, 0]"
        );
    }
}
