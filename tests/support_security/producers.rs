#![allow(clippy::unwrap_used)]
use super::audit;
use std::collections::{BTreeMap, BTreeSet};
use syn::{
    Expr, Item, Stmt,
    visit::{self, Visit},
};
#[derive(Default)]
struct Calls {
    names: Vec<String>,
    fills: BTreeSet<String>,
}
fn root_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Path(p) => p.path.segments.first().map(|s| s.ident.to_string()),
        Expr::Reference(r) => root_name(&r.expr),
        Expr::Index(i) => root_name(&i.expr),
        Expr::Paren(p) => root_name(&p.expr),
        _ => None,
    }
}
impl<'a> Visit<'a> for Calls {
    fn visit_expr_call(&mut self, call: &'a syn::ExprCall) {
        if let Expr::Path(path) = call.func.as_ref() {
            let name = path
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect::<Vec<_>>()
                .join("::");
            if name == "getrandom::fill"
                && let Some(Expr::Reference(r)) = call.args.first()
            {
                assert!(r.mutability.is_some());
                if let Some(name) = root_name(&r.expr) {
                    self.fills.insert(name);
                }
            }
            self.names.push(name);
        }
        visit::visit_expr_call(self, call);
    }
}
fn scope(path: &str, selector: &str) -> syn::File {
    let tree = audit::parse(&audit::root().join(path));
    let items = audit::selected(&tree, selector);
    assert!(!items.is_empty(), "missing producer {path}/{selector}");
    syn::File {
        shebang: None,
        attrs: vec![],
        items,
    }
}
fn calls(tree: &syn::File) -> Calls {
    let mut c = Calls::default();
    c.visit_file(tree);
    c
}
fn require(path: &str, selector: &str, expected: &str, minimum: usize) -> syn::File {
    let tree = scope(path, selector);
    let c = calls(&tree);
    assert!(
        c.names
            .iter()
            .filter(|name| name.as_str() == expected)
            .count()
            >= minimum,
        "{path}/{selector} no longer invokes actual secure producer {expected}: {:?}",
        c.names
    );
    tree
}
fn blocks(tree: &syn::File) -> Vec<&syn::Block> {
    let mut result = vec![];
    for item in &tree.items {
        match item {
            Item::Fn(f) => result.push(f.block.as_ref()),
            Item::Impl(i) => {
                for member in &i.items {
                    if let syn::ImplItem::Fn(f) = member {
                        result.push(&f.block);
                    }
                }
            }
            _ => {}
        }
    }
    result
}
#[derive(Default)]
struct Paths(BTreeSet<String>);
impl<'a> Visit<'a> for Paths {
    fn visit_expr_path(&mut self, p: &'a syn::ExprPath) {
        if let Some(first) = p.path.segments.first() {
            self.0.insert(first.ident.to_string());
        }
        visit::visit_expr_path(self, p);
    }
}
fn assert_filled_result(tree: &syn::File, destination: &str) {
    let filled = calls(tree).fills;
    assert!(
        filled.contains(destination),
        "OS RNG must fill the actual output buffer"
    );
    for block in blocks(tree) {
        let Some(Stmt::Expr(value, None)) = block.stmts.last() else {
            panic!("producer has no returned value")
        };
        let mut paths = Paths::default();
        paths.visit_expr(value);
        assert!(
            paths.0.contains(destination),
            "OS-filled bytes must reach the returned value"
        );
    }
}
fn assert_returned_field(tree: &syn::File, field: &str, producer: &str) {
    struct Fields<'a> {
        field: &'a str,
        producer: &'a str,
        found: bool,
    }
    impl<'ast> Visit<'ast> for Fields<'_> {
        fn visit_field_value(&mut self, field: &'ast syn::FieldValue) {
            if matches!(&field.member, syn::Member::Named(name) if name == self.field) {
                let mut producer_calls = Calls::default();
                producer_calls.visit_expr(&field.expr);
                self.found |= producer_calls
                    .names
                    .iter()
                    .any(|name| name == self.producer);
            }
            visit::visit_field_value(self, field);
        }
    }
    for block in blocks(tree) {
        let Some(Stmt::Expr(value, None)) = block.stmts.last() else {
            panic!("missing producer result")
        };
        let mut fields = Fields {
            field,
            producer,
            found: false,
        };
        fields.visit_expr(value);
        assert!(
            fields.found,
            "returned field {field} must contain actual {producer} output"
        );
    }
}
fn assert_bound_result(tree: &syn::File, destination: &str, producer: &str) {
    for block in blocks(tree) {
        let binding = block.stmts.iter().any(|statement| {
            let Stmt::Local(local) = statement else {
                return false;
            };
            let syn::Pat::Ident(name) = &local.pat else {
                return false;
            };
            if name.ident != destination {
                return false;
            }
            let Some(initializer) = &local.init else {
                return false;
            };
            let mut calls = Calls::default();
            calls.visit_expr(&initializer.expr);
            calls.names.iter().any(|name| name == producer)
        });
        assert!(
            binding,
            "output buffer must originate from the secure producer"
        );
        let Some(Stmt::Expr(value, None)) = block.stmts.last() else {
            panic!("missing producer result")
        };
        let mut paths = Paths::default();
        paths.visit_expr(value);
        assert!(
            paths.0.contains(destination),
            "generated material must reach returned value"
        );
    }
}
pub fn check() {
    let random = require("src/crypto.rs", "item:random_bytes", "getrandom::fill", 1);
    assert_filled_result(&random, "bytes");
    let padding = require("src/crypto.rs", "item:pad_payload", "getrandom::fill", 1);
    assert_filled_result(&padding, "result");
    let id = require("src/crypto.rs", "item:random_id", "random_bytes", 1);
    let c = calls(&id);
    assert!(
        c.names.contains(&"i64::from_le_bytes".into()),
        "exchange identity must retain all eight random bytes"
    );
    let wrapper = require("src/framing.rs", "item:wrap", "random_bytes", 1);
    assert_returned_field(&wrapper, "random_bytes", "random_bytes");
    let file_key = require(
        "src/files.rs",
        "method:FileKey::generate",
        "random_bytes",
        2,
    );
    assert_returned_field(&file_key, "key", "random_bytes");
    assert_returned_field(&file_key, "iv", "random_bytes");
    let exponent = require(
        "src/dh.rs",
        "method:SecretExponent::generate",
        "random_bytes",
        1,
    );
    assert_bound_result(&exponent, "bytes", "random_bytes");
    let c = calls(&exponent);
    assert!(c.names.contains(&"Zeroizing::new".into()));
    require(
        "src/dh.rs",
        "method:DhConfig::new",
        "Self::from_parameters",
        1,
    );
    require(
        "src/dh.rs",
        "method:DhConfig::from_parameters",
        "grammers_crypto::check_dh_prime",
        1,
    );
    require(
        "src/manager/establishment.rs",
        "method:SecretChatManager::create",
        "SecretExponent::generate",
        1,
    );
    require("src/rekey.rs", "item:start", "random_id", 1);
    require("src/rekey.rs", "item:start", "SecretExponent::generate", 1);
    // Fresh encrypted-ingress nonces and pre-ack identity challenges are additional native producers.
    require(
        "src/ingress/encrypted.rs",
        "method:EncryptedIngressStorage::commit",
        "random_bytes",
        1,
    );
    require(
        "src/ingress/coordinator.rs",
        "method:DurableIngress::new",
        "random_bytes",
        1,
    );
    require(
        "src/ingress/coordinator.rs",
        "method:DurableIngress::verify_client",
        "random_bytes",
        1,
    );
    let values = [
        kiaro_secret_chat::crypto::random_bytes::<32>().unwrap(),
        kiaro_secret_chat::crypto::random_bytes::<32>().unwrap(),
    ];
    assert_ne!(values[0], values[1]);
    assert!(values.iter().all(|v| v.iter().any(|b| *b != 0)));
    let key = kiaro_secret_chat::files::FileKey::generate().unwrap();
    assert_ne!(key.key(), key.iv());
    // Ensure every retained legacy producer remains represented, rather than checking an unused import.
    let expected = BTreeMap::from([
        ("crypto.py", "src/crypto.rs"),
        ("framing.py", "src/framing.rs"),
        ("files.py0", "src/files.rs"),
        ("dh.py", "src/dh.rs"),
        ("handshake.py", "src/protocol.rs"),
        ("rekey.py", "src/rekey.rs"),
    ]);
    let scopes = audit::scopes();
    for (legacy, path) in expected {
        assert!(scopes[legacy].files.iter().any(|f| f.path == path));
    }
}
