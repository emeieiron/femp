use std::env;
use std::path::Path;

use femp_frontend::{Loaded, Root};

fn main() {
    let args: Vec<String> = env::args().collect();
    let loaded = Loaded::open(Path::new(&args[1]), &args[2]).unwrap_or_else(|e| panic!("{e}"));
    let roots: Vec<Root> = args[3..]
        .iter()
        .map(|spec| {
            let mut parts: Vec<String> = spec.split("::").map(str::to_string).collect();
            let name = parts.pop().unwrap();
            if parts
                .last()
                .is_some_and(|p| p.chars().next().unwrap().is_uppercase())
            {
                let owner = parts.pop().unwrap();
                Root::method(&parts, &owner, &name)
            } else {
                Root::function(&parts, &name)
            }
        })
        .collect();
    let lowered = loaded.lower(&roots);
    for result in &lowered.roots {
        println!("// {result:?}");
    }
    print!("{}", femp_vir::print::program(&lowered.program));
}
