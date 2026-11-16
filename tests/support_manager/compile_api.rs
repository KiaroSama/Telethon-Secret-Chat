#[allow(dead_code)]
#[path = "../support_compile/mod.rs"]
mod shared;
#[allow(dead_code)]
pub fn prebuilt_receipt_contract() {
    shared::prebuilt_receipt_contract();
}
pub fn compile_api(source: &str) -> std::process::Output {
    shared::compile_program(source)
}
