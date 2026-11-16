#![allow(clippy::unwrap_used)]
#[path = "support_primitive/compile_boundary.rs"]
mod compile_boundary;
use kiaro_secret_chat::visualization::{PALETTE, key_visualization};
#[test]
fn design_vector() {
    let bytes: Vec<_> = (0..36).collect();
    let picture = key_visualization(&bytes).unwrap();
    let expected = [
        "000010002000",
        "300001001100",
        "210031000200",
        "120022003200",
        "030013002300",
        "330000101010",
        "201030100110",
        "111021103110",
        "021012102210",
        "321003101310",
        "231033100020",
        "102020203020",
    ];
    for (row, expected) in picture.rows.iter().zip(expected) {
        assert_eq!(row.iter().map(u8::to_string).collect::<String>(), expected);
    }
    assert_eq!(picture.hex, hex::encode(&bytes[..32]));
    let mut hash = [0; 36];
    hash[0] = 0b11100100;
    assert_eq!(
        &key_visualization(&hash).unwrap().rows[0][..4],
        &[0, 1, 2, 3]
    );
}
#[test]
fn palette() {
    assert_eq!(PALETTE, ["#FFFFFF", "#D5E6F3", "#2D5775", "#2F99C9"]);
}
#[test]
fn length35() {
    assert!(key_visualization(&[0; 35]).is_err());
    for n in [0, 16, 32] {
        assert!(key_visualization(&vec![0; n]).is_err());
    }
}
#[test]
fn length37() {
    assert!(key_visualization(&[0; 37]).is_err());
}
#[test]
fn none_is_refused_by_the_type_system() {
    compile_boundary::refuses("None::<&[u8]>");
}
#[test]
fn text_is_refused_by_the_type_system() {
    compile_boundary::refuses("\"000000000000000000000000000000000000\"");
}
#[test]
fn rendering_needs_only_the_standard_library() {
    let source = syn::parse_file(include_str!("../src/visualization/renderer.rs")).unwrap();
    struct Imports(Vec<String>);
    impl<'ast> syn::visit::Visit<'ast> for Imports {
        fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
            if let syn::UseTree::Path(p) = &item.tree {
                self.0.push(p.ident.to_string());
            }
            syn::visit::visit_item_use(self, item);
        }
        fn visit_expr_path(&mut self, p: &'ast syn::ExprPath) {
            if p.path.segments.len() > 1 {
                let root = p.path.segments[0].ident.to_string();
                assert!(
                    ["String", "Some", "None", "Self", "std", "KeyVisualization"]
                        .contains(&root.as_str()),
                    "external renderer reference {root}"
                );
            }
            syn::visit::visit_expr_path(self, p);
        }
    }
    let mut imports = Imports(Vec::new());
    syn::visit::Visit::visit_file(&mut imports, &source);
    assert!(imports.0.iter().all(|x| x == "std"));
}
