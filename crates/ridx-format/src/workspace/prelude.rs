/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Standard library prelude mappings for Rust 2021/2024 editions.
//! Maps standard unqualified type and trait names to their canonical sysroot paths.

/// Lookup candidate sysroot paths for standard prelude types and traits in Rust 2021/2024.
pub fn prelude_candidate_paths(short_name: &str) -> &'static [&'static str] {
    match short_name {
        // Alloc types
        "Box" => &["alloc::boxed::Box", "std::boxed::Box"],
        "String" => &["alloc::string::String", "std::string::String"],
        "Vec" => &["alloc::vec::Vec", "std::vec::Vec"],
        "ToString" => &["alloc::string::ToString", "std::string::ToString"],
        "ToOwned" => &["alloc::borrow::ToOwned", "std::borrow::ToOwned"],

        // Core types / enums
        "Option" => &["core::option::Option", "std::option::Option"],
        "Some" => &["core::option::Option", "std::option::Option"],
        "None" => &["core::option::Option", "std::option::Option"],
        "Result" => &["core::result::Result", "std::result::Result"],
        "Ok" => &["core::result::Result", "std::result::Result"],
        "Err" => &["core::result::Result", "std::result::Result"],

        // Standard traits in 2021/2024 prelude
        "Clone" => &["core::clone::Clone", "std::clone::Clone"],
        "Copy" => &["core::marker::Copy", "std::marker::Copy"],
        "Send" => &["core::marker::Send", "std::marker::Send"],
        "Sync" => &["core::marker::Sync", "std::marker::Sync"],
        "Sized" => &["core::marker::Sized", "std::marker::Sized"],
        "Unpin" => &["core::marker::Unpin", "std::marker::Unpin"],
        "Drop" => &["core::ops::drop::Drop", "core::ops::Drop"],
        "Default" => &["core::default::Default", "std::default::Default"],
        "PartialEq" => &["core::cmp::PartialEq", "std::cmp::PartialEq"],
        "PartialOrd" => &["core::cmp::PartialOrd", "std::cmp::PartialOrd"],
        "Eq" => &["core::cmp::Eq", "std::cmp::Eq"],
        "Ord" => &["core::cmp::Ord", "std::cmp::Ord"],
        "AsRef" => &["core::convert::AsRef", "std::convert::AsRef"],
        "AsMut" => &["core::convert::AsMut", "std::convert::AsMut"],
        "Into" => &["core::convert::Into", "std::convert::Into"],
        "From" => &["core::convert::From", "std::convert::From"],
        "TryFrom" => &["core::convert::TryFrom", "std::convert::TryFrom"],
        "TryInto" => &["core::convert::TryInto", "std::convert::TryInto"],
        "Iterator" => &[
            "core::iter::traits::iterator::Iterator",
            "core::iter::Iterator",
        ],
        "Extend" => &["core::iter::traits::collect::Extend", "core::iter::Extend"],
        "IntoIterator" => &[
            "core::iter::traits::collect::IntoIterator",
            "core::iter::IntoIterator",
        ],
        "FromIterator" => &[
            "core::iter::traits::collect::FromIterator",
            "core::iter::FromIterator",
        ],
        "DoubleEndedIterator" => &[
            "core::iter::traits::double_ended::DoubleEndedIterator",
            "core::iter::DoubleEndedIterator",
        ],
        "ExactSizeIterator" => &[
            "core::iter::traits::exact_size::ExactSizeIterator",
            "core::iter::ExactSizeIterator",
        ],
        "Fn" => &["core::ops::function::Fn", "core::ops::Fn"],
        "FnMut" => &["core::ops::function::FnMut", "core::ops::FnMut"],
        "FnOnce" => &["core::ops::function::FnOnce", "core::ops::FnOnce"],
        "Future" => &["core::future::future::Future", "core::future::Future"],
        "IntoFuture" => &[
            "core::future::into_future::IntoFuture",
            "core::future::IntoFuture",
        ],
        _ => &[],
    }
}

/// Returns true if a trait belongs to the standard 2021/2024 prelude and is in scope by default.
/// Checked strictly against canonical full paths (e.g. `core::clone::Clone`).
pub fn is_prelude_trait(path: &str) -> bool {
    matches!(
        path,
        "core::clone::Clone"
            | "std::clone::Clone"
            | "core::marker::Copy"
            | "core::marker::Send"
            | "core::marker::Sync"
            | "core::marker::Sized"
            | "core::marker::Unpin"
            | "core::default::Default"
            | "std::default::Default"
            | "core::cmp::PartialEq"
            | "std::cmp::PartialEq"
            | "core::cmp::PartialOrd"
            | "std::cmp::PartialOrd"
            | "core::cmp::Eq"
            | "std::cmp::Eq"
            | "core::cmp::Ord"
            | "std::cmp::Ord"
            | "core::iter::traits::iterator::Iterator"
            | "core::iter::Iterator"
            | "core::iter::traits::collect::IntoIterator"
            | "core::iter::IntoIterator"
            | "core::iter::traits::collect::Extend"
            | "core::iter::Extend"
            | "core::iter::traits::collect::FromIterator"
            | "core::iter::FromIterator"
            | "core::iter::traits::double_ended::DoubleEndedIterator"
            | "core::iter::DoubleEndedIterator"
            | "core::iter::traits::exact_size::ExactSizeIterator"
            | "core::iter::ExactSizeIterator"
            | "core::convert::AsRef"
            | "std::convert::AsRef"
            | "core::convert::AsMut"
            | "std::convert::AsMut"
            | "core::convert::Into"
            | "std::convert::Into"
            | "core::convert::From"
            | "std::convert::From"
            | "core::convert::TryInto"
            | "std::convert::TryInto"
            | "core::convert::TryFrom"
            | "std::convert::TryFrom"
            | "alloc::string::ToString"
            | "std::string::ToString"
            | "alloc::borrow::ToOwned"
            | "std::borrow::ToOwned"
            | "core::ops::function::Fn"
            | "core::ops::Fn"
            | "core::ops::function::FnMut"
            | "core::ops::FnMut"
            | "core::ops::function::FnOnce"
            | "core::ops::FnOnce"
            | "core::ops::drop::Drop"
            | "core::ops::Drop"
            | "core::future::future::Future"
            | "core::future::Future"
            | "core::future::into_future::IntoFuture"
            | "core::future::IntoFuture"
    )
}
