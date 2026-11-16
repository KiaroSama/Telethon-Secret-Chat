#![allow(clippy::unwrap_used)]
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use syn::{
    Item, UseTree,
    visit::{self, Visit},
};
#[derive(Deserialize)]
pub struct Scope {
    pub responsibility: String,
    pub files: Vec<Source>,
}
#[derive(Deserialize)]
pub struct Source {
    pub path: String,
    #[serde(default)]
    pub selectors: Vec<String>,
}
pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
pub fn parse(path: &Path) -> syn::File {
    syn::parse_file(&fs::read_to_string(path).unwrap())
        .unwrap_or_else(|error| panic!("invalid Rust source {}: {error}", path.display()))
}
pub fn scopes() -> BTreeMap<String, Scope> {
    serde_json::from_str(include_str!("scopes.json")).unwrap()
}
fn rust_files(folder: &Path, result: &mut BTreeSet<PathBuf>) {
    for entry in fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, result);
        } else if path.extension().is_some_and(|e| e == "rs") {
            result.insert(path);
        }
    }
}
pub fn sources() -> BTreeSet<PathBuf> {
    let root = root();
    let mut result = BTreeSet::new();
    rust_files(&root.join("src"), &mut result);
    rust_files(&root.join("vendor/grammers"), &mut result);
    for entry in fs::read_dir(root.join("tools")).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') || name == "target" {
            continue;
        }
        let package = entry.path();
        if package.join("Cargo.toml").is_file() && package.join("src").is_dir() {
            rust_files(&package.join("src"), &mut result);
        }
    }
    result.insert(root.join("build.rs"));
    for name in ["secret_types.rs", "secret_enums.rs", "secret_common.rs"] {
        result.insert(PathBuf::from(env!("OUT_DIR")).join(name));
    }
    result
}
fn banned(root: &str) -> bool {
    matches!(
        root,
        "rand" | "fastrand" | "rand_pcg" | "rand_xorshift" | "oorandom" | "nanorand" | "random"
    )
}
#[derive(Default)]
pub struct Audit {
    pub offences: BTreeSet<String>,
}
fn use_paths(tree: &UseTree, prefix: &mut Vec<String>, output: &mut Vec<Vec<String>>) {
    match tree {
        UseTree::Path(p) => {
            prefix.push(p.ident.to_string());
            use_paths(&p.tree, prefix, output);
            prefix.pop();
        }
        UseTree::Name(n) => {
            let mut path = prefix.clone();
            path.push(n.ident.to_string());
            output.push(path);
        }
        UseTree::Rename(n) => {
            let mut path = prefix.clone();
            path.push(n.ident.to_string());
            output.push(path);
        }
        UseTree::Glob(_) => output.push(prefix.clone()),
        UseTree::Group(g) => {
            for node in &g.items {
                use_paths(node, prefix, output)
            }
        }
    }
}
impl Audit {
    fn path(&mut self, path: &syn::Path) {
        if let Some(first) = path.segments.first() {
            let name = first.ident.to_string();
            if path.segments.len() > 1 && banned(&name) {
                self.offences.insert(format!("insecure RNG path: {name}"));
            }
        }
    }
    fn macro_tokens(&mut self, tokens: proc_macro2::TokenStream) {
        if let Ok(file) = syn::parse2::<syn::File>(tokens.clone()) {
            self.visit_file(&file);
        }
        let tokens = tokens.into_iter().collect::<Vec<_>>();
        for (index, token) in tokens.iter().enumerate() {
            match token {
                proc_macro2::TokenTree::Group(group) => self.macro_tokens(group.stream()),
                proc_macro2::TokenTree::Ident(ident)
                    if banned(&ident.to_string())
                        && matches!(tokens.get(index+1),Some(proc_macro2::TokenTree::Punct(p))if p.as_char()==':')
                        && matches!(tokens.get(index+2),Some(proc_macro2::TokenTree::Punct(p))if p.as_char()==':') =>
                {
                    self.offences
                        .insert(format!("insecure RNG macro path: {ident}"));
                }
                _ => {}
            }
        }
    }
}
impl<'ast> Visit<'ast> for Audit {
    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        let mut paths = Vec::new();
        use_paths(&node.tree, &mut Vec::new(), &mut paths);
        for path in paths {
            if path.first().is_some_and(|p| banned(p)) {
                self.offences
                    .insert(format!("insecure RNG import: {}", path.join("::")));
            }
        }
        visit::visit_item_use(self, node);
    }
    fn visit_item_extern_crate(&mut self, node: &'ast syn::ItemExternCrate) {
        if banned(&node.ident.to_string()) {
            self.offences
                .insert(format!("insecure RNG external crate: {}", node.ident));
        }
        visit::visit_item_extern_crate(self, node);
    }
    fn visit_path(&mut self, node: &'ast syn::Path) {
        self.path(node);
        visit::visit_path(self, node);
    }
    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        self.path(&node.path);
        self.macro_tokens(node.tokens.clone());
        visit::visit_macro(self, node);
    }
}
pub fn audit(tree: &syn::File) -> BTreeSet<String> {
    let mut visitor = Audit::default();
    visitor.visit_file(tree);
    visitor.offences
}
fn self_type(item: &syn::ItemImpl) -> String {
    if let syn::Type::Path(p) = item.self_ty.as_ref() {
        p.path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default()
    } else {
        String::new()
    }
}
fn named(item: &Item) -> Option<String> {
    match item {
        Item::Fn(i) => Some(i.sig.ident.to_string()),
        Item::Struct(i) => Some(i.ident.to_string()),
        Item::Enum(i) => Some(i.ident.to_string()),
        Item::Trait(i) => Some(i.ident.to_string()),
        Item::Mod(i) => Some(i.ident.to_string()),
        _ => None,
    }
}
pub fn selected(tree: &syn::File, selector: &str) -> Vec<Item> {
    let (kind, name) = selector.split_once(':').unwrap_or(("item", selector));
    let mut result = Vec::new();
    for item in &tree.items {
        match (kind, item) {
            ("type", Item::Impl(i)) if self_type(i) == name => result.push(item.clone()),
            ("type", _) if named(item).as_deref() == Some(name) => result.push(item.clone()),
            ("inherent", Item::Impl(i)) if self_type(i) == name && i.trait_.is_none() => {
                result.push(item.clone())
            }
            ("storage_impl", Item::Impl(i))
                if self_type(i) == name
                    && i.trait_.as_ref().is_some_and(|(_, p, _)| {
                        p.segments.last().is_some_and(|s| s.ident == "Storage")
                    }) =>
            {
                result.push(item.clone())
            }
            ("method", Item::Impl(i)) => {
                if let Some((owner, method)) = name.split_once("::")
                    && self_type(i) == owner
                {
                    let mut copy = i.clone();
                    copy.items.retain(
                        |member| matches!(member,syn::ImplItem::Fn(f)if f.sig.ident==method),
                    );
                    if !copy.items.is_empty() {
                        result.push(Item::Impl(copy));
                    }
                }
            }
            ("item", _) if named(item).as_deref() == Some(name) => result.push(item.clone()),
            _ => {}
        }
    }
    result
}
pub fn check_scope(key: &str) {
    let scopes = scopes();
    let scope = &scopes[key];
    assert!(!scope.responsibility.is_empty() && !scope.files.is_empty());
    for source in &scope.files {
        let path = if let Some(name) = source.path.strip_prefix("$generated/") {
            PathBuf::from(env!("OUT_DIR")).join(name)
        } else {
            root().join(&source.path)
        };
        let tree = parse(&path);
        if source.selectors.is_empty() {
            assert!(
                audit(&tree).is_empty(),
                "{}: {:?}",
                scope.responsibility,
                audit(&tree)
            );
        } else {
            for selector in &source.selectors {
                let items = selected(&tree, selector);
                assert!(
                    !items.is_empty(),
                    "missing responsibility {key}: {selector} in {}",
                    path.display()
                );
                let tree = syn::File {
                    shebang: None,
                    attrs: vec![],
                    items,
                };
                assert!(
                    audit(&tree).is_empty(),
                    "{} / {selector}: {:?}",
                    scope.responsibility,
                    audit(&tree)
                );
            }
        }
    }
}
