//! Generate or check one atomic Rust schema module using the pinned Grammers generator.
use kiaro_secret_chat::schema_generator::Generator;
use std::{io::Write, path::PathBuf, process::ExitCode};
fn main() -> ExitCode {
    let mut schema = None::<PathBuf>;
    let mut output = None;
    let mut check = false;
    let mut generator = Generator::default();
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--schema") => {
                let Some(value) = args.next() else {
                    return usage();
                };
                schema = Some(value.into());
            }
            Some("--output") => {
                let Some(value) = args.next() else {
                    return usage();
                };
                output = Some(PathBuf::from(value));
            }
            Some("--formatter") => {
                let Some(value) = args.next() else {
                    return usage();
                };
                generator.formatter = value.into();
            }
            Some("--check") => check = true,
            Some("--help") => {
                println!(
                    "generate-schema --output PATH [--schema PATH] [--check] [--formatter PATH]"
                );
                return ExitCode::SUCCESS;
            }
            _ => return usage(),
        }
    }
    let Some(output) = output else { return usage() };
    let embedded = if schema.is_none() {
        match embedded_schema() {
            Ok(file) => Some(file),
            Err(_) => {
                eprintln!("The bundled Rust schema could not be staged");
                return ExitCode::from(2);
            }
        }
    } else {
        None
    };
    let Some(schema) = schema
        .as_deref()
        .or_else(|| embedded.as_ref().map(|file| file.path()))
    else {
        return usage();
    };
    match generator.generate(schema, &output, check) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => {
            eprintln!("The generated Rust schema is absent or differs");
            ExitCode::from(1)
        }
        Err(_) => {
            eprintln!("The Rust schema could not be generated or formatted");
            ExitCode::from(2)
        }
    }
}
fn usage() -> ExitCode {
    eprintln!("Usage: generate-schema --output PATH [--schema PATH] [--check] [--formatter PATH]");
    ExitCode::from(2)
}

fn embedded_schema() -> std::io::Result<tempfile::NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix(".kiaro-schema-")
        .suffix(".tl")
        .tempfile()?;
    file.write_all(include_bytes!("../../schema/end-to-end.tl"))?;
    file.flush()?;
    Ok(file)
}
