use std::collections::HashMap;
fn main() {
    let src = std::fs::read_to_string("examples/tree.tyto").unwrap();
    let m: std::collections::HashMap<(usize, usize), &'static str> =
        tyto_lang::analysis::semantics::semantic_tokens(&src)
            .into_iter()
            .map(|t| ((t.line, t.col), tyto_lang::analysis::semantics::TOKEN_TYPES[t.ty as usize]))
            .collect();
    let mut keys: Vec<_> = m.keys().cloned().collect();
    keys.sort();
    for k in keys {
        println!("{:?} = {}", k, m[&k]);
    }
}
