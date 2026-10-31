/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;

mod cache_and_workspace;
mod cas_and_owner_bounds;
mod supertrait_and_impl_links;
mod sysroot_core;

#[test]
fn test_map_def_kind() {
    assert_eq!(map_def_kind(hir::def::DefKind::Mod), RDefKind::Mod);
    assert_eq!(map_def_kind(hir::def::DefKind::Struct), RDefKind::Struct);
    assert_eq!(map_def_kind(hir::def::DefKind::Union), RDefKind::Union);
    assert_eq!(map_def_kind(hir::def::DefKind::Enum), RDefKind::Enum);
    assert_eq!(map_def_kind(hir::def::DefKind::Variant), RDefKind::Variant);
    assert_eq!(map_def_kind(hir::def::DefKind::Trait), RDefKind::Trait);
    assert_eq!(map_def_kind(hir::def::DefKind::TyAlias), RDefKind::TyAlias);
    assert_eq!(map_def_kind(hir::def::DefKind::ForeignTy), RDefKind::ForeignTy);
    assert_eq!(map_def_kind(hir::def::DefKind::TraitAlias), RDefKind::TraitAlias);
    assert_eq!(map_def_kind(hir::def::DefKind::AssocTy), RDefKind::AssocTy);
    assert_eq!(map_def_kind(hir::def::DefKind::TyParam), RDefKind::TyParam);
    assert_eq!(map_def_kind(hir::def::DefKind::Fn), RDefKind::Fn);
    assert_eq!(map_def_kind(hir::def::DefKind::Const { is_type_const: false }), RDefKind::Const);
    assert_eq!(map_def_kind(hir::def::DefKind::ConstParam), RDefKind::ConstParam);
    assert_eq!(map_def_kind(hir::def::DefKind::Static { mutability: hir::Mutability::Not, safety: hir::Safety::Safe, nested: false }), RDefKind::Static);
    assert_eq!(map_def_kind(hir::def::DefKind::Ctor(hir::def::CtorOf::Struct, hir::def::CtorKind::Fn)), RDefKind::StructCtor);
    assert_eq!(map_def_kind(hir::def::DefKind::Ctor(hir::def::CtorOf::Variant, hir::def::CtorKind::Fn)), RDefKind::VariantCtor);
    assert_eq!(map_def_kind(hir::def::DefKind::AssocFn), RDefKind::AssocFn);
    assert_eq!(map_def_kind(hir::def::DefKind::AssocConst { is_type_const: false }), RDefKind::AssocConst);
    assert_eq!(map_def_kind(hir::def::DefKind::Macro(rustc_span::hygiene::MacroKind::Bang.into())), RDefKind::Macro);
    assert_eq!(map_def_kind(hir::def::DefKind::ExternCrate), RDefKind::ExternCrate);
    assert_eq!(map_def_kind(hir::def::DefKind::Use), RDefKind::Use);
    assert_eq!(map_def_kind(hir::def::DefKind::ForeignMod), RDefKind::ForeignMod);
    assert_eq!(map_def_kind(hir::def::DefKind::AnonConst), RDefKind::AnonConst);
    assert_eq!(map_def_kind(hir::def::DefKind::OpaqueTy), RDefKind::OpaqueTy);
    assert_eq!(map_def_kind(hir::def::DefKind::Field), RDefKind::Field);
    assert_eq!(map_def_kind(hir::def::DefKind::LifetimeParam), RDefKind::LifetimeParam);
    assert_eq!(map_def_kind(hir::def::DefKind::GlobalAsm), RDefKind::GlobalAsm);
    assert_eq!(map_def_kind(hir::def::DefKind::Impl { of_trait: false }), RDefKind::Impl);
    assert_eq!(map_def_kind(hir::def::DefKind::Closure), RDefKind::Closure);
    assert_eq!(map_def_kind(hir::def::DefKind::SyntheticCoroutineBody), RDefKind::Other);
}

#[test]
fn test_extract_target_disambiguator() {
    assert_eq!(extract_target_disambiguator(&["-Cextra-filename=123".to_string()]), Some("123".to_string()));
    assert_eq!(extract_target_disambiguator(&["-C".to_string(), "extra-filename=456".to_string()]), Some("456".to_string()));
    assert_eq!(extract_target_disambiguator(&["--crate-type=lib".to_string()]), Some("_lib".to_string()));
    assert_eq!(extract_target_disambiguator(&["--crate-type".to_string(), "bin".to_string()]), Some("_bin".to_string()));
    assert_eq!(extract_target_disambiguator(&["-Cextra-filename=".to_string()]), None);
    assert_eq!(extract_target_disambiguator(&["-C".to_string(), "extra-filename=".to_string()]), None);
    assert_eq!(extract_target_disambiguator(&["--crate-type=".to_string()]), None);
    assert_eq!(extract_target_disambiguator(&["--crate-type".to_string(), "".to_string()]), Some("_".to_string()));
    assert_eq!(extract_target_disambiguator(&["rustc".to_string()]), None);
}

#[test]
fn test_hash_helpers() {
    assert_eq!(hash_directory_contents(Path::new("/nonexistent_dir")), None);
    assert_eq!(hash_file_contents(Path::new("/nonexistent_file")), None);

    let temp_dir = std::env::temp_dir().join(format!("ridx_hash_test_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let f1 = temp_dir.join("a.txt");
    let _ = fs::write(&f1, b"hello");
    assert!(hash_file_contents(&f1).is_some());
    assert!(hash_directory_contents(&temp_dir).is_some());
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_compute_dependency_unit_key() {
    let key1 = compute_dependency_unit_key("my_crate", "0.1.0", Some("abc"), Some(Path::new("/path")), &["--target".to_string(), "x86_64-unknown-linux-gnu".to_string()]);
    let key2 = compute_dependency_unit_key("my_crate", "0.2.0", Some("abc"), Some(Path::new("/path")), &["--target".to_string(), "x86_64-unknown-linux-gnu".to_string()]);
    assert_eq!(key1.len(), 16);
    assert_eq!(key2.len(), 16);
    assert_ne!(key1, key2);

    let temp_dir = std::env::temp_dir().join(format!("ridx_key_test_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let ext_file = temp_dir.join("dep.rmeta");
    fs::write(&ext_file, b"rmeta_bytes").unwrap();

    let key3 = compute_dependency_unit_key_with_out_dir(
        "my_crate",
        "0.1.0",
        None,
        None,
        &[
            "--target=aarch64-apple-darwin".to_string(),
            "--cfg".to_string(),
            "feat_a".to_string(),
            "--cfg=feat_b".to_string(),
            "--extern".to_string(),
            format!("dep={}", ext_file.display()),
            "--extern=plain_dep".to_string(),
            "--extern=missing_dep=/nonexistent/path.rmeta".to_string(),
        ],
        Some(&temp_dir),
    );
    assert_eq!(key3.len(), 16);
    let _ = fs::remove_dir_all(&temp_dir);
}

static DRIVER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn test_run_driver_queries() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();
    let _ = run_driver(vec!["ridx-driver".into(), "--version".into()]);
    let _ = run_driver(vec!["ridx-driver".into(), "--help".into()]);
    let _ = run_driver(vec!["ridx-driver".into(), "-vV".into()]);
    let _ = run_driver(vec!["ridx-driver".into(), "-V".into()]);
    let _ = run_driver(vec!["ridx-driver".into(), "-h".into()]);
    let _ = run_driver(vec!["ridx-driver".into(), "--print".into(), "cfg".into()]);
    let _ = run_driver(vec!["ridx-driver".into(), "--print=cfg".into()]);
    let _ = run_driver(vec!["ridx-driver".into(), "foo-driver".into(), "rustc".into(), "--version".into()]);
    let _ = run_driver(vec!["ridx-driver".into(), "--crate-name".into(), "build_script_build".into()]);
    let _ = run_driver(vec!["ridx-driver".into(), "rustc".into(), "--crate-name".into(), "build_script_main".into()]);
}

#[test]
fn test_driver_index_compilation() {
    let _lock = DRIVER_TEST_LOCK.lock().unwrap();

    let temp_dir = std::env::temp_dir().join(format!("ridx_driver_compile_test_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let src_file = temp_dir.join("lib.rs");
    let code = r#"
pub mod sub {
    pub fn sub_fn() -> i32 { 42 }
}
pub struct Point { pub x: i32, pub y: i32 }
pub struct TuplePt(pub i32, pub i32);
pub enum Status { Active, Inactive(u32), Detailed { code: u8 } }
pub union RawVal { pub f: f32, pub u: u32 }
pub trait Greeter {
    type Output;
    const GREETING: &'static str = "hi";
    fn greet(&self) -> String;
    fn default_greeting(&self) -> &'static str { Self::GREETING }
}
impl Greeter for Point {
    type Output = String;
    fn greet(&self) -> String { format!("{}:{}", self.x, self.y) }
}
pub const C: i32 = 123;
pub static S: &str = "abc";
pub type Pt = Point;
macro_rules! my_add {
    ($a:expr, $b:expr) => { $a + $b };
}
pub fn compute<T: Greeter>(p: &Point, g: &T) -> i32 {
    let mut val = my_add!(p.x, p.y) + C;
    let tp = TuplePt(1, 2);
    val += tp.0 + tp.1;
    let cl = |a: i32| a * 2;
    val += cl(sub::sub_fn());
    let _ = g.greet();
    let _ = g.default_greeting();
    let st = Status::Detailed { code: 7 };
    match st {
        Status::Active => 1,
        Status::Inactive(n) => n as i32,
        Status::Detailed { code } => code as i32,
    };
    val
}
"#;
    fs::write(&src_file, code).unwrap();
    let out_dir = temp_dir.join("out");
    fs::create_dir_all(&out_dir).unwrap();
    let rmeta_file = temp_dir.join("libtest_sample.rmeta");

    let sysroot = std::process::Command::new("rustc").args(["--print", "sysroot"]).output().unwrap();
    let sysroot_str = String::from_utf8_lossy(&sysroot.stdout).trim().to_string();

    let args = vec![
        "rustc".to_string(),
        src_file.to_str().unwrap().to_string(),
        "--crate-name".to_string(),
        "test_sample".to_string(),
        "--crate-type".to_string(),
        "lib".to_string(),
        "--emit=metadata".to_string(),
        "-o".to_string(),
        rmeta_file.to_str().unwrap().to_string(),
        "--sysroot".to_string(),
        sysroot_str,
    ];

    let _ = run_driver_with_env(args, DriverEnv::for_test(&out_dir));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_is_immutable_package() {
    assert!(is_immutable_package_with_home(Path::new("/home/user/.cargo/registry/src/github.com-1ecc6299db9ec823/foo-0.1.0"), None));
    assert!(is_immutable_package_with_home(Path::new("/home/user/.cargo/git/checkouts/bar-12345/abc"), None));
    assert!(!is_immutable_package_with_home(Path::new("/home/user/project/my_crate"), None));

    let custom_cargo = Path::new("/custom/cargo");
    assert!(is_immutable_package_with_home(Path::new("/custom/cargo/registry/foo"), Some(custom_cargo)));
    assert!(is_immutable_package_with_home(Path::new("/custom/cargo/git/foo"), Some(custom_cargo)));
    assert!(!is_immutable_package_with_home(Path::new("/custom/cargo/other/foo"), Some(custom_cargo)));
}
