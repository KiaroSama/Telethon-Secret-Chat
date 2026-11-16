#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::crypto;
pub fn run(case: &str, index: usize) {
    let name = case.split('[').next().unwrap();
    let lengths = [0, 1, 11, 12, 15, 16, 17, 31, 63, 100, 1000];
    let input = vec![b'x'; lengths[index % 11]];
    match name {
        "test_the_total_is_divisible_by_sixteen" => {
            assert_eq!(crypto::pad_payload(&input).unwrap().len() % 16, 0);
            if input.is_empty() {
                for n in 0..1024 {
                    let padded = crypto::pad_payload(&vec![0; n]).unwrap();
                    assert_eq!(padded.len() % 16, 0);
                    assert!((12..=27).contains(&(padded.len() - n)));
                }
            }
        }
        "test_the_padding_is_at_least_twelve_bytes" => {
            assert!(crypto::pad_payload(&input).unwrap().len() - input.len() >= 12)
        }
        "test_the_padding_is_at_most_1024_bytes" => {
            assert!(crypto::pad_payload(&input).unwrap().len() - input.len() <= 1024)
        }
        "test_the_padding_is_not_constant" => {
            let mut seen = std::collections::BTreeSet::new();
            for _ in 0..50 {
                seen.insert(crypto::pad_payload(b"same").unwrap().to_vec());
            }
            assert!(seen.len() > 40);
            let key = crypto::SharedKey::from_slice(&[1; 256]).unwrap();
            assert_ne!(
                crypto::encrypt_frame(&key, b"same", crypto::Direction::Initiator).unwrap(),
                crypto::encrypt_frame(&key, b"same", crypto::Direction::Initiator).unwrap()
            );
        }
        "test_the_payload_is_returned_unchanged_at_the_front" => {
            let input: Vec<u8> = (0..64).collect();
            assert!(crypto::pad_payload(&input).unwrap().starts_with(&input));
        }
        "test_the_padding_source_is_os_urandom" => {
            struct SecureCalls(usize);
            impl<'ast> syn::visit::Visit<'ast> for SecureCalls {
                fn visit_expr_call(&mut self, c: &'ast syn::ExprCall) {
                    if let syn::Expr::Path(p) = &*c.func {
                        let names: Vec<_> = p
                            .path
                            .segments
                            .iter()
                            .map(|x| x.ident.to_string())
                            .collect();
                        if names == ["getrandom", "fill"] {
                            self.0 += 1;
                        }
                    }
                    syn::visit::visit_expr_call(self, c);
                }
            }
            let ast = syn::parse_file(include_str!("../../src/crypto.rs")).unwrap();
            let f = ast
                .items
                .iter()
                .find_map(|x| match x {
                    syn::Item::Fn(f) if f.sig.ident == "pad_payload" => Some(f),
                    _ => None,
                })
                .unwrap();
            let mut calls = SecureCalls(0);
            syn::visit::Visit::visit_block(&mut calls, &f.block);
            assert!(calls.0 > 0);
            let value = crypto::pad_payload(&[0; 20]).unwrap();
            assert!(value[20..].iter().any(|x| *x != 0));
        }
        "test_the_exempt_file_says_it_is_generated_on_its_first_line" => {
            for source in [
                include_str!(concat!(env!("OUT_DIR"), "/secret_common.rs")),
                include_str!(concat!(env!("OUT_DIR"), "/secret_types.rs")),
                include_str!(concat!(env!("OUT_DIR"), "/secret_enums.rs")),
            ] {
                let line = source
                    .lines()
                    .next()
                    .unwrap()
                    .trim_start_matches('/')
                    .trim_start();
                assert!(
                    line.to_lowercase().starts_with("generated"),
                    "generated output needs first-line provenance: {line}"
                );
            }
        }
        _ => panic!("unknown padding scenario {case}"),
    }
}
