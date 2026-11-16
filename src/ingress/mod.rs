//! Durable receipt before authorization-wide queue acknowledgement.
pub(crate) mod application;
mod coordinator;
pub mod encrypted;
mod hooks;
mod ledger;
mod requests;
mod shared;
pub use coordinator::DurableIngress;
pub use encrypted::EncryptedIngressStorage;
pub use ledger::{IngressBatch, ReceiptKind};
pub use shared::IngressProtocolStorage;
#[cfg(test)]
mod attachment_tests;
#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod strong_tests;
#[cfg(test)]
mod test_fixture;
#[cfg(test)]
mod tests;
