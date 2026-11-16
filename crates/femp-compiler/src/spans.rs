//! Source positions of lowered code. Files of ingots on disk are absolute
//! paths; files of the built-in libraries are named `ingot:path`.

use common::diagnostics::Span as FeSpan;
use driver::DriverDataBase;
use femp_vir::Span;
use hir::analysis::semantic::SemOrigin;
use hir::analysis::ty::ty_check::BodyOwner;
use hir::span::LazySpan;

pub(crate) fn origin(db: &DriverDataBase, owner: BodyOwner<'_>, origin: SemOrigin<'_>) -> Span {
    let Some(body) = owner.body(db) else {
        return Span::default();
    };
    let resolved = match origin {
        SemOrigin::Expr(expr) => expr.span(body).resolve(db),
        SemOrigin::Stmt(stmt) => stmt.span(body).resolve(db),
        SemOrigin::Pat(pat) => pat.span(body).resolve(db),
        SemOrigin::Body(owner) => owner.body(db).and_then(|body| body.span().resolve(db)),
        SemOrigin::Synthetic => None,
    };
    resolved.map_or_else(Span::default, |span| convert(db, &span))
}

pub(crate) fn of_contract(db: &DriverDataBase, contract: hir::hir_def::Contract<'_>) -> Span {
    contract
        .span()
        .resolve(db)
        .map_or_else(Span::default, |span| convert(db, &span))
}

pub(crate) fn of_owner(db: &DriverDataBase, owner: BodyOwner<'_>) -> Span {
    owner
        .body(db)
        .and_then(|body| body.span().resolve(db))
        .map_or_else(Span::default, |span| convert(db, &span))
}

fn convert(db: &DriverDataBase, span: &FeSpan) -> Span {
    let text = span.file.text(db);
    let start = usize::from(span.range.start()).min(text.len());
    let line = text[..start].matches('\n').count() as u32 + 1;
    let file = match span.file.url(db) {
        Some(url) if url.scheme() == "file" => url
            .to_file_path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|()| url.to_string()),
        _ => {
            let ingot = span
                .file
                .containing_ingot(db)
                .and_then(|ingot| crate::lower::ingot_name(db, ingot))
                .unwrap_or_else(|| "builtin".to_string());
            let path = span
                .file
                .path(db)
                .as_ref()
                .map(|path| path.to_string())
                .unwrap_or_default();
            format!("{ingot}:{path}")
        }
    };
    Span::new(file, line)
}
