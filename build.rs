use grammers_tl_gen::{Config, Outputs, generate_rust_code};
use grammers_tl_parser::parse_tl_file;
use std::{collections::HashMap, env, fs, io, path::PathBuf};

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-changed=schema/end-to-end.tl");
    let source = fs::read_to_string("schema/end-to-end.tl")?;
    // Historical constructors share names. Retain their IDs and result families,
    // and suffix only the older Rust names before passing them to the generator.
    let mut counts = HashMap::new();
    for line in source.lines().filter(|l| !l.trim_start().starts_with("//")) {
        if let Some((name, _)) = line.split_once('#') {
            *counts.entry(name.to_owned()).or_insert(0usize) += 1;
        }
    }
    let mut normalized = String::new();
    for line in source.lines() {
        if !line.trim_start().starts_with("//")
            && let Some((name, rest)) = line.split_once('#')
            && let Some(count) = counts.get_mut(name)
        {
            *count -= 1;
            if *count > 0 {
                let id = rest
                    .split_whitespace()
                    .next()
                    .ok_or_else(|| io::Error::other("Invalid schema ID"))?;
                normalized.push_str(&format!("{name}_legacy{id}#{rest}\n"));
                continue;
            }
        }
        normalized.push_str(line);
        normalized.push('\n');
    }
    let definitions = parse_tl_file(&normalized)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| io::Error::other("The secret schema could not be parsed"))?;
    let directory =
        PathBuf::from(env::var_os("OUT_DIR").ok_or_else(|| io::Error::other("Missing OUT_DIR"))?);
    let mut outputs = Outputs {
        common: fs::File::create(directory.join("secret_common.rs"))?,
        types: fs::File::create(directory.join("secret_types.rs"))?,
        functions: fs::File::create(directory.join("secret_functions.rs"))?,
        enums: fs::File::create(directory.join("secret_enums.rs"))?,
    };
    // Generated Debug is disabled because media constructors contain file keys.
    generate_rust_code(
        &mut outputs,
        &definitions,
        144,
        &Config {
            gen_name_for_id: true,
            impl_debug: false,
            ..Default::default()
        },
    )?;
    outputs.flush()
}
