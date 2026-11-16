#![allow(clippy::unwrap_used, dead_code)]
use kiaro_secret_chat::{
    crypto::{self, SharedKey},
    dh::{self, DhConfig, SecretExponent},
    error::{DhFailure, Error},
};
use num_bigint::{BigInt, BigUint};
use num_traits::One;
use sha1::{Digest, Sha1};
pub fn prime() -> BigUint {
    BigUint::from_bytes_be(
        &hex::decode(concat!(
            "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74",
            "020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F1437",
            "4FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED",
            "EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF05",
            "98DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB",
            "9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3B",
            "E39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF695581718",
            "3995497CEA956AE515D2261898FA051015728E5A8AACAA68FFFFFFFFFFFFFFFF"
        ))
        .unwrap(),
    )
}
fn config() -> DhConfig {
    DhConfig::new(2, &prime().to_bytes_be()).unwrap()
}
fn rejected(g: BigInt, p: BigUint) -> Error {
    match DhConfig::from_parameters(&g, &p) {
        Ok(_) => panic!("unsafe DH parameters accepted"),
        Err(e) => e,
    }
}
fn dh_error(g: BigInt, p: BigUint, reason: DhFailure) {
    assert!(matches!(rejected(g,p),Error::DhFailure(r)if r==reason));
}
fn parse_case(case: &str) -> (&str, &str) {
    case.split_once('[')
        .map_or((case, ""), |(a, b)| (a, b.trim_end_matches(']')))
}
pub fn parameters(case: &str) {
    let (name, param) = parse_case(case);
    match name {
        "test_a_real_safe_prime_with_g_2_is_accepted" => {
            let c = config();
            assert_eq!(c.generator(), 2);
            assert_eq!(c.prime_bytes(), prime().to_bytes_be());
        }
        "test_a_composite_prime_is_refused" => {
            let p = (BigUint::one() << 2047) + 7u32;
            dh_error(2.into(), p, DhFailure::UnsafePrime);
            let mut raw = vec![0; 256];
            raw[0] = 128;
            raw[255] = 7;
            assert!(matches!(
                DhConfig::new(2, &raw),
                Err(Error::DhFailure(DhFailure::UnsafePrime))
            ));
            assert!(
                Error::DhFailure(DhFailure::UnsafePrime)
                    .to_string()
                    .contains("safe prime")
            );
        }
        "test_the_residue_condition_is_what_refuses_a_prime_of_the_wrong_class" => {
            dh_error(
                2.into(),
                (BigUint::one() << 2047) + 1u32,
                DhFailure::Residue,
            );
            assert!(
                Error::DhFailure(DhFailure::Residue)
                    .to_string()
                    .contains("subgroup")
            );
        }
        "test_a_prime_of_the_wrong_bit_length_is_refused" => {
            dh_error(2.into(), 2039u32.into(), DhFailure::Length);
            for length in [0, 1, 128, 255, 257] {
                assert!(matches!(
                    DhConfig::new(2, &vec![255; length]),
                    Err(Error::DhFailure(DhFailure::Length))
                ));
            }
            assert!(
                Error::DhFailure(DhFailure::Length)
                    .to_string()
                    .contains("2048")
            );
        }
        "test_a_prime_one_bit_too_large_is_refused" => {
            dh_error(2.into(), prime() * 2u32 + 1u32, DhFailure::Length)
        }
        "test_a_prime_whose_half_is_composite_is_refused" => {
            assert!(dh::is_probable_prime(&13u32.into()).unwrap());
            assert!(!dh::is_safe_prime(&13u32.into()).unwrap());
            assert!(dh::is_safe_prime(&11u32.into()).unwrap());
        }
        "test_the_primality_test_agrees_with_a_known_sieve" => {
            let mut sieve = vec![true; 2000];
            sieve[0] = false;
            sieve[1] = false;
            for i in 2..45 {
                if sieve[i] {
                    for j in (i * i..2000).step_by(i) {
                        sieve[j] = false;
                    }
                }
            }
            for (n, expected) in sieve.into_iter().enumerate() {
                assert_eq!(dh::is_probable_prime(&n.into()).unwrap(), expected, "n={n}");
            }
        }
        "test_a_generator_at_or_below_one_is_refused" => dh_error(
            param.parse::<i32>().unwrap().into(),
            prime(),
            DhFailure::GeneratorRange,
        ),
        "test_a_generator_at_or_above_p_minus_one_is_refused" => dh_error(
            BigInt::from(prime() - 1u32),
            prime(),
            DhFailure::GeneratorRange,
        ),
        "test_a_generator_outside_the_documented_set_is_refused" => {
            dh_error(9.into(), prime(), DhFailure::GeneratorRange);
            for generator in [0, 1, 8, 9, u32::MAX] {
                assert!(matches!(
                    DhConfig::new(generator, &prime().to_bytes_be()),
                    Err(Error::DhFailure(DhFailure::GeneratorRange))
                ));
            }
        }
        "test_g_2_against_a_prime_failing_its_residue_condition" => {
            assert!(dh::generator_is_quadratic_residue(2, &prime()));
            assert!(!dh::generator_is_quadratic_residue(2, &13u32.into()));
        }
        "test_every_documented_residue_condition" => {
            let v: Vec<u32> = param.split('-').map(|x| x.parse().unwrap()).collect();
            let m = match v[0] {
                2 => 8,
                3 => 3,
                5 => 5,
                6 => 24,
                7 => 7,
                _ => panic!("fixturegenerator"),
            };
            assert!(dh::generator_is_quadratic_residue(
                v[0],
                &(v[1] + m * 10).into()
            ));
            assert!(!dh::generator_is_quadratic_residue(
                v[0],
                &(v[2] + m * 10).into()
            ));
        }
        "test_g_4_has_no_condition" => {
            assert!(dh::generator_is_quadratic_residue(4, &13u32.into()));
            assert!(dh::generator_is_quadratic_residue(4, &17u32.into()));
        }
        "test_a_structurally_wrong_prime_is_refused_before_any_exponentiation" => {
            dh_error(2.into(), 2039u32.into(), DhFailure::Length);
            let ast = syn::parse_file(include_str!("../../src/dh.rs")).unwrap();
            let method=ast.items.iter().find_map(|item|match item{syn::Item::Impl(i)if matches!(&*i.self_ty,syn::Type::Path(p)if p.path.is_ident("DhConfig"))=>i.items.iter().find_map(|m|match m{syn::ImplItem::Fn(f)if f.sig.ident=="from_parameters"=>Some(f),_=>None}),_=>None}).unwrap();
            let first = &method.block.stmts[0];
            let syn::Stmt::Expr(syn::Expr::If(guard), _) = first else {
                panic!("length guard must be first")
            };
            assert!(matches!(
                guard.then_branch.stmts.last(),
                Some(syn::Stmt::Expr(syn::Expr::Return(_), _))
            ));
            struct Calls(Vec<String>);
            impl<'ast> syn::visit::Visit<'ast> for Calls {
                fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
                    if let syn::Expr::Path(p) = &*node.func {
                        self.0
                            .extend(p.path.segments.iter().map(|s| s.ident.to_string()));
                    }
                    syn::visit::visit_expr_call(self, node);
                }
            }
            let mut calls = Calls(Vec::new());
            syn::visit::Visit::visit_expr_if(&mut calls, guard);
            for banned in ["check_dh_prime", "modpow", "is_probable_prime"] {
                assert!(!calls.0.iter().any(|x| x == banned));
            }
        }
        "test_the_refusal_names_the_chat_without_naming_the_value" => {
            let value: BigUint = (BigUint::one() << 2047usize) + 1u32;
            let e = rejected(2.into(), value.clone()).with_chat(77);
            assert!(e.to_string().contains("77"));
            for text in [e.to_string(), format!("{e:?}")] {
                assert!(!text.contains(&value.to_string()));
                assert!(!text.to_lowercase().contains(&value.to_str_radix(16)));
            }
        }
        _ => panic!("unknown DH parameters scenario {case}"),
    }
}
pub fn peer(case: &str) {
    let (name, param) = parse_case(case);
    let c = config();
    let p = prime();
    let floor: BigUint = BigUint::one() << 1984usize;
    let check = |v: BigUint| c.check_peer(&v.to_bytes_be());
    match name {
        "test_a_normal_peer_value_is_accepted" => assert!(check(&p / 2u32).is_ok()),
        "test_a_value_at_or_below_one_is_refused" => {
            assert!(
                c.check_peer_integer(&param.parse::<BigInt>().unwrap())
                    .is_err()
            );
            if param == "0" {
                for bytes in [vec![], vec![0], vec![1], vec![255; 257]] {
                    assert!(c.check_peer(&bytes).is_err());
                }
            }
        }
        "test_a_value_at_or_above_p_minus_one_is_refused" => {
            assert!(check(&p - 1u32 + param.parse::<u32>().unwrap()).is_err())
        }
        "test_a_value_below_the_2_pow_1984_floor_is_refused" => {
            assert!(check(&floor - 1u32).is_err())
        }
        "test_a_value_above_the_matching_ceiling_is_refused" => {
            assert!(check(&p - &floor + 1u32).is_err())
        }
        "test_the_boundary_values_themselves_are_accepted" => {
            assert!(check(floor.clone()).is_ok());
            assert!(check(&p - &floor).is_ok());
        }
        "test_the_check_reads_bytes_as_a_big_endian_integer" => {
            let v = &floor + 12345u32;
            assert_eq!(dh::value_from_bytes(&v.to_bytes_be()), v);
        }
        "test_checking_a_peer_value_never_exponentiates" => {
            let v = param.parse::<BigUint>().unwrap();
            let expected = v >= floor && v <= p - floor;
            assert_eq!(check(v).is_ok(), expected);
            struct Powers(bool);
            impl<'ast> syn::visit::Visit<'ast> for Powers {
                fn visit_expr_method_call(&mut self, x: &'ast syn::ExprMethodCall) {
                    if x.method == "modpow" {
                        self.0 = true;
                    }
                    syn::visit::visit_expr_method_call(self, x);
                }
            }
            let ast = syn::parse_file(include_str!("../../src/dh.rs")).unwrap();
            let mut visitor = Powers(false);
            for item in ast.items {
                if let syn::Item::Impl(implementation) = item {
                    for item in implementation.items {
                        if let syn::ImplItem::Fn(f) = item
                            && (f.sig.ident == "check_peer" || f.sig.ident == "check_peer_integer")
                        {
                            syn::visit::Visit::visit_block(&mut visitor, &f.block);
                        }
                    }
                }
            }
            assert!(!visitor.0);
        }
        "test_the_refusal_carries_no_peer_value" => {
            let value = &floor - 1u32;
            let e = check(value.clone()).unwrap_err().with_chat(9);
            assert!(!e.to_string().contains(&value.to_string()));
            assert!(!format!("{e:?}").contains(&value.to_str_radix(16)));
        }
        _ => panic!("unknown peer scenario {case}"),
    }
}
pub fn fingerprint(case: &str) {
    let (name, _) = parse_case(case);
    let raw: [u8; 256] = std::array::from_fn(|i| i as u8);
    let c = config();
    match name {
        "test_the_fingerprint_is_sha1_of_the_key_last_eight_bytes_little_endian" => {
            let h = Sha1::digest(raw);
            assert_eq!(
                crypto::key_fingerprint(&raw),
                i64::from_le_bytes(h[12..20].try_into().unwrap())
            );
        }
        "test_the_fingerprint_fits_a_signed_int64" => {
            for _ in 0..50 {
                let raw = crypto::random_bytes::<256>().unwrap();
                let fp = crypto::key_fingerprint(&raw);
                let encoded = fp.to_le_bytes();
                assert_eq!(i64::from_le_bytes(encoded), fp);
            }
        }
        "test_a_different_key_gives_a_different_fingerprint" => assert_ne!(
            crypto::key_fingerprint(&[0; 256]),
            crypto::key_fingerprint(&[1; 256])
        ),
        "test_both_sides_derive_the_same_key_and_fingerprint" => {
            let a = SecretExponent::generate().unwrap();
            let b = SecretExponent::generate().unwrap();
            let ka = c.shared_key(&c.public_value(&b), &a).unwrap();
            let kb = c.shared_key(&c.public_value(&a), &b).unwrap();
            assert_eq!(ka.expose(), kb.expose());
            assert_eq!(ka.fingerprint(), kb.fingerprint());
        }
        "test_the_shared_key_is_always_256_bytes" => {
            let short = c
                .shared_key(
                    &(&prime() / 2u32).to_bytes_be(),
                    &SecretExponent::from_bytes([0; 256]),
                )
                .unwrap();
            assert_eq!(short.expose().len(), 256);
            let a = SecretExponent::generate().unwrap();
            let b = SecretExponent::generate().unwrap();
            assert_eq!(
                c.shared_key(&c.public_value(&b), &a)
                    .unwrap()
                    .expose()
                    .len(),
                256
            );
        }
        "test_the_padding_is_on_the_LEFT" => {
            let short = c
                .shared_key(
                    &(&prime() / 2u32).to_bytes_be(),
                    &SecretExponent::from_bytes([0; 256]),
                )
                .unwrap();
            assert_eq!(short.expose()[255], 1);
            assert_eq!(&short.expose()[..255], &[0; 255]);
        }
        "test_the_secret_is_2048_bits_from_a_secure_source" => {
            let mut values = std::collections::BTreeSet::new();
            for _ in 0..20 {
                let secret = SecretExponent::generate().unwrap();
                assert!(BigUint::from_bytes_be(secret.expose()).bits() > 2000);
                values.insert(secret.expose().to_vec());
            }
            assert_eq!(values.len(), 20);
        }
        "test_deriving_a_key_validates_the_peer_value_first" => {
            assert!(matches!(
                c.shared_key(&[1], &SecretExponent::generate().unwrap()),
                Err(Error::PeerValue)
            ));
            let ast = syn::parse_file(include_str!("../../src/dh.rs")).unwrap();
            let function = ast
                .items
                .iter()
                .find_map(|i| {
                    if let syn::Item::Impl(i) = i {
                        i.items.iter().find_map(|f| {
                            if let syn::ImplItem::Fn(f) = f {
                                (f.sig.ident == "shared_key").then_some(f)
                            } else {
                                None
                            }
                        })
                    } else {
                        None
                    }
                })
                .unwrap();
            let syn::Stmt::Local(local) = &function.block.stmts[0] else {
                panic!("peer validation must precede derivation")
            };
            let syn::Expr::Try(try_expr) = &*local.init.as_ref().unwrap().expr else {
                panic!("refusal must propagate")
            };
            assert!(
                matches!(&*try_expr.expr,syn::Expr::MethodCall(call)if call.method=="check_peer")
            );
        }
        "test_the_fingerprint_comparison_refuses_a_mismatch" => {
            let key = SharedKey::from_slice(&crypto::random_bytes::<256>().unwrap()).unwrap();
            dh::verify_fingerprint(&key, key.fingerprint()).unwrap();
            assert!(matches!(
                dh::verify_fingerprint(&key, key.fingerprint() ^ 1),
                Err(Error::Fingerprint)
            ));
        }
        "test_a_fingerprint_mismatch_does_not_report_either_value" => {
            let key = SharedKey::from_slice(&crypto::random_bytes::<256>().unwrap()).unwrap();
            let claimed = key.fingerprint() ^ 1;
            let error = dh::verify_fingerprint(&key, claimed)
                .unwrap_err()
                .with_chat(5);
            let text = format!("{error}{error:?}");
            for banned in [
                claimed.to_string(),
                key.fingerprint().to_string(),
                hex::encode(key.expose())[..32].to_string(),
            ] {
                assert!(!text.contains(&banned));
            }
        }
        _ => panic!("unknown fingerprint scenario {case}"),
    }
}
