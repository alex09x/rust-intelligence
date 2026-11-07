/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Core `u32`-keyed identifiers and trait boundaries (`SourceDb`, `ItemDb`) decoupling
//! `ridx-syntax::name_res` from binary index storage (`ridx-format`).

/// Dense crate or compilation-unit identifier (`u32::MAX` = absent sentinel).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CrateId(pub u32);

impl CrateId {
    pub const NONE: Self = Self(u32::MAX);
    /// Synthetic crate slot used for dirty-file local items not yet present in the compiler index.
    pub const LOCAL_OVERLAY: Self = Self(u32::MAX - 1);

    #[inline]
    pub const fn is_some(self) -> bool {
        self.0 != Self::NONE.0
    }
}

/// Dense source-file identifier inside a [`SourceDb`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(pub u32);

impl FileId {
    pub const NONE: Self = Self(u32::MAX);

    #[inline]
    pub const fn is_some(self) -> bool {
        self.0 != Self::NONE.0
    }
}

/// Dense module identifier inside a crate's [`super::mod_tree::ModuleTree`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModuleId(pub u32);

impl ModuleId {
    pub const ROOT: Self = Self(0);
    pub const NONE: Self = Self(u32::MAX);

    #[inline]
    pub const fn is_some(self) -> bool {
        self.0 != Self::NONE.0
    }
}

/// Compact 8-byte global definition key (`(CrateId, def_index)`), mapping 1-to-1 to `(shard_idx, def_idx)`
/// in `WorkspaceIndex` or `(CrateId::LOCAL_OVERLAY, local_idx)` for dirty-file definitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GlobalDefId {
    pub krate: CrateId,
    pub index: u32,
}

impl GlobalDefId {
    pub const NONE: Self = Self { krate: CrateId::NONE, index: u32::MAX };

    #[inline]
    pub const fn new(krate: CrateId, index: u32) -> Self {
        Self { krate, index }
    }

    #[inline]
    pub const fn is_some(self) -> bool {
        self.krate.is_some() && self.index != u32::MAX
    }
}

/// Rust language edition of a crate.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Edition {
    Edition2015 = 0,
    Edition2018 = 1,
    #[default]
    Edition2021 = 2,
    Edition2024 = 3,
}

impl Edition {
    #[inline]
    pub const fn has_uniform_paths(self) -> bool {
        (self as u8) >= (Self::Edition2018 as u8)
    }
}

/// Visibility class aligned with `ridx-format::ItemVis`, extended with local `super` / `self` forms.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VisKind {
    Public = 0,
    Crate = 1,
    Restricted = 2,
    #[default]
    Private = 3,
    Super = 4,
}

/// Kind of an indexed or local definition, aligned with `ridx-format::DefKind`.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Mod = 0,
    Struct = 1,
    Enum = 2,
    Union = 3,
    Trait = 4,
    Fn = 5,
    TypeAlias = 6,
    Const = 7,
    Static = 8,
    Variant = 9,
    Field = 10,
    AssocFn = 11,
    AssocTy = 12,
    AssocConst = 13,
    Macro = 14,
    Impl = 15,
    Use = 16,
    ExternCrate = 17,
}

impl ItemKind {
    /// Bitmask of Rust namespaces (`NsMask`) occupied by a definition of this kind.
    #[inline]
    pub const fn ns_mask(self, has_value_ctor: bool) -> NsMask {
        match self {
            Self::Mod | Self::Enum | Self::Union | Self::Trait | Self::TypeAlias | Self::AssocTy => NsMask::TYPE,
            Self::Struct | Self::Variant => {
                if has_value_ctor {
                    NsMask(NsMask::TYPE.0 | NsMask::VALUE.0)
                } else {
                    NsMask::TYPE
                }
            }
            Self::Fn | Self::Const | Self::Static | Self::Field | Self::AssocFn | Self::AssocConst => NsMask::VALUE,
            Self::Macro => NsMask::MACRO,
            Self::ExternCrate => NsMask::TYPE,
            Self::Impl | Self::Use => NsMask::EMPTY,
        }
    }
}

/// Bitmask over the three Rust namespaces (`type`, `value`, `macro`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NsMask(pub u8);

impl NsMask {
    pub const EMPTY: Self = Self(0);
    pub const TYPE: Self = Self(1 << 0);
    pub const VALUE: Self = Self(1 << 1);
    pub const MACRO: Self = Self(1 << 2);
    pub const ALL: Self = Self(Self::TYPE.0 | Self::VALUE.0 | Self::MACRO.0);

    #[inline]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[inline]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Indexed definition summary surfaced by [`ItemDb`] for module children and associated items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexedDef<'a> {
    pub id: GlobalDefId,
    pub name: &'a str,
    pub kind: ItemKind,
    pub vis: VisKind,
    pub has_value_ctor: bool,
}

/// Indexed `use` / re-export row surfaced by [`ItemDb`] (matching `v5` `UseEntry`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexedUse<'a> {
    pub name: &'a str,
    pub type_def: GlobalDefId,
    pub value_def: GlobalDefId,
    pub macro_def: GlobalDefId,
    pub vis: VisKind,
    pub is_glob: bool,
    pub is_reexport: bool,
    pub is_prelude: bool,
    pub is_underscore: bool,
    pub prim_ty_code: u8,
}

/// Read-only virtual filesystem / source provider for workspace `.rs` files.
pub trait SourceDb {
    /// Maps a forward-slash normalized crate-relative or workspace-relative path to its [`FileId`].
    fn resolve_path(&self, normalized_path: &str) -> Option<FileId>;

    /// Forward-slash normalized path of `file`.
    fn file_path(&self, file: FileId) -> &str;

    /// UTF-8 source text of `file` (including any unsaved editor overlay).
    fn file_text(&self, file: FileId) -> &str;
}

/// Compiler-index query boundary (`v5` `.ridx` adapter or in-memory test database).
pub trait ItemDb {
    /// Returns `true` when bare `#[cfg(key)]` is active for `krate` (e.g. `unix`, `test`).
    fn cfg_flag(&self, krate: CrateId, key: &str) -> bool;

    /// Returns `true` when `#[cfg(key = "value")]` is active for `krate` (e.g. `feature = "rt"`).
    fn cfg_key_value(&self, krate: CrateId, key: &str, value: &str) -> bool;

    /// Language edition of `krate`.
    fn crate_edition(&self, _krate: CrateId) -> Edition {
        Edition::Edition2021
    }

    /// Canonical crate name of `krate` (underscores, e.g. `"ridx_syntax"`).
    fn crate_name(&self, _krate: CrateId) -> &str {
        ""
    }

    /// Resolves an external dependency crate available to `from_krate` under `extern_name`.
    fn extern_crate(&self, _from_krate: CrateId, _extern_name: &str) -> Option<CrateId> {
        None
    }

    /// Returns the crate root [`GlobalDefId`] of `krate`, if indexed.
    fn crate_root_def(&self, _krate: CrateId) -> Option<GlobalDefId> {
        None
    }

    /// Returns the prelude module [`GlobalDefId`] configured for `krate`, if any.
    fn prelude_module(&self, _krate: CrateId) -> Option<GlobalDefId> {
        None
    }

    /// Looks up a declarative `macro_rules!` body (`($pat) => { $body }; ...`) for item-level
    /// expansion when `call_path` (e.g. `"cfg_if"` or `"cfg::alloc"`) is invoked in `module_path`.
    fn macro_rules_body(&self, _krate: CrateId, _module_path: &str, _call_path: &str) -> Option<&str> {
        None
    }

    /// Appends direct child definitions of `module_def` into `out`.
    fn module_defs<'a>(&'a self, _module_def: GlobalDefId, _out: &mut Vec<IndexedDef<'a>>) {}

    /// Appends `UseEntry` rows belonging to `module_def` (or crate root) into `out`.
    fn module_uses<'a>(&'a self, _module_def: GlobalDefId, _out: &mut Vec<IndexedUse<'a>>) {}

    /// Appends inherent and trait associated items on `owner_def` into `out`.
    fn assoc_items<'a>(&'a self, _owner_def: GlobalDefId, _out: &mut Vec<IndexedDef<'a>>) {}

    /// Looks up a lang item (`u16` discriminant matching `LangItemKind`) across visible shards.
    fn lang_item(&self, _lang_kind: u16) -> Option<GlobalDefId> {
        None
    }
}
