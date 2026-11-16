#![allow(clippy::unwrap_used)]
use kiaro_secret_chat::{
    crypto,
    files::{self, FileKey},
};
use md5::{Digest, Md5};
use sha1::Sha1;
fn encrypt(data: &[u8], key: &FileKey) -> Vec<u8> {
    let mut out = Vec::new();
    files::encrypt_file(&mut &data[..], &mut out, data.len() as u64, key).unwrap();
    out
}
fn decrypt(data: &[u8], size: u64, key: &FileKey) -> Vec<u8> {
    let mut out = Vec::new();
    files::decrypt_file(&mut &data[..], &mut out, size, key).unwrap();
    out
}
pub fn run(name: &str) {
    let key = FileKey::generate().unwrap();
    match name {
        "test_a_file_key_is_thirty_two_bytes_and_so_is_its_iv" => {
            assert_eq!((key.key().len(), key.iv().len()), (32, 32))
        }
        "test_every_file_gets_its_own_key" => {
            let mut seen = std::collections::BTreeSet::new();
            for _ in 0..50 {
                let k = FileKey::generate().unwrap();
                seen.insert((*k.key(), *k.iv()));
            }
            assert_eq!(seen.len(), 50);
        }
        "test_the_file_key_is_unrelated_to_the_chat_key" => {
            let ast = syn::parse_file(include_str!("../../src/files.rs")).unwrap();
            let method=ast.items.iter().find_map(|i|match i{syn::Item::Impl(i)if matches!(&*i.self_ty,syn::Type::Path(p)if p.path.is_ident("FileKey"))=>i.items.iter().find_map(|m|match m{syn::ImplItem::Fn(f)if f.sig.ident=="generate"=>Some(f),_=>None}),_=>None}).unwrap();
            assert!(method.sig.inputs.is_empty());
        }
        "test_the_file_key_comes_from_a_secure_source" => {
            let ast = syn::parse_file(include_str!("../../src/files.rs")).unwrap();
            struct Calls(usize);
            impl<'a> syn::visit::Visit<'a> for Calls {
                fn visit_expr_call(&mut self, c: &'a syn::ExprCall) {
                    if matches!(&*c.func,syn::Expr::Path(p)if p.path.is_ident("random_bytes")) {
                        self.0 += 1;
                    }
                    syn::visit::visit_expr_call(self, c);
                }
            }
            let mut count = Calls(0);
            for i in ast.items {
                if let syn::Item::Impl(i) = i {
                    for m in i.items {
                        if let syn::ImplItem::Fn(f) = m
                            && f.sig.ident == "generate"
                        {
                            syn::visit::Visit::visit_block(&mut count, &f.block);
                        }
                    }
                }
            }
            assert_eq!(count.0, 2);
            assert_ne!(key.key(), &[0; 32]);
            assert_ne!(key.iv(), &[0; 32]);
        }
        "test_the_fingerprint_is_the_documented_md5_fold" => {
            let mut data = key.key().to_vec();
            data.extend_from_slice(key.iv());
            let hash = Md5::digest(data);
            let expected = i32::from_le_bytes(std::array::from_fn(|i| hash[i] ^ hash[4 + i]));
            assert_eq!(key.fingerprint(), expected);
            assert_eq!(
                FileKey::new(&[0; 32], &[0; 32]).unwrap().fingerprint(),
                -1593105637
            );
        }
        "test_it_is_md5_and_not_the_sha1_of_section_1_5" => {
            let mut raw = key.key().to_vec();
            raw.extend_from_slice(key.iv());
            let h = Sha1::digest(raw);
            assert_ne!(
                i64::from(key.fingerprint()),
                i64::from_le_bytes(h[12..20].try_into().unwrap())
            );
        }
        "test_it_fits_a_signed_int32" => {
            for _ in 0..50 {
                let fp = FileKey::generate().unwrap().fingerprint();
                assert_eq!(i32::from_le_bytes(fp.to_le_bytes()), fp);
            }
        }
        "test_a_different_key_or_iv_changes_it" => {
            let other = FileKey::generate().unwrap();
            assert_ne!(
                key.fingerprint(),
                FileKey::new(key.key(), other.iv()).unwrap().fingerprint()
            );
            assert_ne!(
                key.fingerprint(),
                FileKey::new(other.key(), key.iv()).unwrap().fingerprint()
            );
        }
        "test_a_file_round_trips_through_its_own_key" => {
            let input = crypto::random_bytes::<1000>().unwrap();
            assert_eq!(decrypt(&encrypt(&input, &key), 1000, &key), input);
            assert!(files::encrypt_file(&mut &b"more"[..], &mut Vec::new(), 3, &key).is_err());
        }
        "test_the_ciphertext_is_padded_to_a_cipher_block" => {
            assert_eq!(encrypt(&[b'x'; 1001], &key).len() % 16, 0)
        }
        "test_the_padding_is_removed_on_the_way_back" => {
            for n in [0, 1, 15, 16, 17, 4096, 65535, 65536, 65537, 131073] {
                let data: Vec<u8> = (0..n).map(|i| i as u8).collect();
                assert_eq!(decrypt(&encrypt(&data, &key), n as u64, &key), data);
            }
        }
        "test_a_file_encrypted_under_one_key_does_not_open_under_another" => {
            let input = b"the contents".repeat(8);
            assert_ne!(
                decrypt(&encrypt(&input, &key), 96, &FileKey::generate().unwrap()),
                input
            );
        }
        "test_a_mismatched_fingerprint_is_refused" => {
            assert!(key.verify(key.fingerprint() ^ 1).is_err())
        }
        "test_a_matching_fingerprint_is_accepted" => key.verify(key.fingerprint()).unwrap(),
        "test_nothing_is_written_when_the_fingerprint_does_not_match" => {
            let directory = tempfile::tempdir().unwrap();
            let target = directory.path().join("should-not-exist.bin");
            let data = encrypt(b"secret bytes", &key);
            assert!(
                files::save_file(&mut &data[..], &target, 12, &key, key.fingerprint() ^ 255)
                    .is_err()
            );
            assert!(!target.exists());
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        }
        "test_a_good_file_is_written" => {
            let directory = tempfile::tempdir().unwrap();
            let target = directory.path().join("ok.bin");
            let data = b"the real contents".repeat(10);
            let encrypted = encrypt(&data, &key);
            files::save_file(
                &mut &encrypted[..],
                &target,
                data.len() as u64,
                &key,
                key.fingerprint(),
            )
            .unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), data);
            std::fs::write(&target, b"existing").unwrap();
            assert!(
                files::save_file(&mut &b"short"[..], &target, 20, &key, key.fingerprint()).is_err()
            );
            assert_eq!(std::fs::read(&target).unwrap(), b"existing");
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
            let mut extended = encrypt(b"data", &key);
            extended.extend([0; 16]);
            assert!(files::decrypt_file(&mut &extended[..], &mut Vec::new(), 4, &key).is_err());
        }
        "test_the_refusal_names_no_key_and_no_content" => {
            let error = key.verify(key.fingerprint() ^ 1).unwrap_err().with_chat(1);
            let text = format!("{error}{error:?}").to_lowercase();
            assert!(!text.contains(&hex::encode(key.key())[..16]));
            assert!(!text.contains(&hex::encode(key.iv())[..16]));
        }
        "test_a_leftover_media_temp_file_is_removed_before_saving" => {
            let directory = tempfile::tempdir().unwrap();
            let leftover = directory
                .path()
                .join(format!(".secret-chat-file-{}-x.tmp", std::process::id()));
            std::fs::write(&leftover, b"interrupted plaintext").unwrap();
            let data = encrypt(b"secret bytes", &key);
            let target = directory.path().join("out.bin");
            files::save_file(&mut &data[..], &target, 12, &key, key.fingerprint()).unwrap();
            assert!(!leftover.exists());
            assert_eq!(std::fs::read(target).unwrap(), b"secret bytes");
        }
        _ => panic!("unknown file-key scenario {name}"),
    }
}
