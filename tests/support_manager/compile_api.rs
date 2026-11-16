#[allow(dead_code)]
#[path = "../support_compile/mod.rs"]
mod shared;
pub fn compile_api(source: &str) -> std::process::Output {
    shared::compile_program(source)
}
