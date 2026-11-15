use std::env;

use anyhow::{Result, bail};
use femp_core::artifact::write_file;
use femp_core::manifest::DEFAULT_FE_VERSION;
use femp_core::naming::is_valid_name;

use crate::cli::NewArgs;
use crate::console;

macro_rules! template {
    ($path:literal) => {
        include_str!(concat!("../../templates/project/", $path))
    };
}

const FILES: &[(&str, &str)] = &[
    ("femp.toml", template!("femp.toml")),
    (".gitignore", template!("gitignore")),
    ("README.md", template!("README.md")),
    ("shared/fe.toml", template!("shared/fe.toml")),
    ("shared/src/lib.fe", template!("shared/src/lib.fe")),
    ("evm/fe.toml", template!("evm/fe.toml")),
    ("evm/src/lib.fe", template!("evm/src/lib.fe")),
    ("native/fe.toml", template!("native/fe.toml")),
    ("native/src/lib.fe", template!("native/src/lib.fe")),
    ("wasm/fe.toml", template!("wasm/fe.toml")),
    ("wasm/src/lib.fe", template!("wasm/src/lib.fe")),
];

pub fn run(args: &NewArgs) -> Result<bool> {
    if !is_valid_name(&args.name) {
        bail!(
            "`{}` is not a valid project name: use letters, digits, `_` and `-`, starting with a letter",
            args.name
        );
    }
    let root = env::current_dir()?.join(&args.name);
    if root.exists() {
        bail!("`{}` already exists", root.display());
    }

    for (path, template) in FILES {
        write_file(&root.join(path), render(template, &args.name))?;
    }
    console::step("Created", &format!("project `{}`", args.name));
    console::detail(&format!("cd {}\nfemp test\nfemp build", args.name));
    Ok(true)
}

fn render(template: &str, name: &str) -> String {
    template
        .replace("{{name}}", name)
        .replace("{{ident}}", &fe_identifier(name))
        .replace("{{fe_version}}", DEFAULT_FE_VERSION)
}

fn fe_identifier(name: &str) -> String {
    name.replace('-', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_in_every_placeholder() {
        for (path, template) in FILES {
            let rendered = render(template, "my-app");
            assert!(!rendered.contains("{{"), "unfilled placeholder in {path}");
        }
    }

    #[test]
    fn derives_identifiers_from_names() {
        assert_eq!(
            render("{{name}} {{ident}}_core", "my-app"),
            "my-app my_app_core"
        );
    }
}
