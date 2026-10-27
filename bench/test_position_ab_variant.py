# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from position_ab_variant_rules import (  # noqa: E402
    CLASS_DEFINITION, CLASS_HOVER, CLASS_SWAPPED, CLASS_LOCATIONS, CLASS_SIGNATURE, callee_before_paren, classify_definition, in_macro_invocation, classify_hover, classify_signature,
    classify_variant_locations, cursor_token, is_variant_declaration)

FILE = "/opt/ws/src/nodes.rs"
ITEM = ("src/nodes.rs", 5)
STRUCT = ("src/nodes.rs", 1)
LINES = ["pub struct Foo(u32);", "", "pub enum Ty {", "    #[doc = \"x\"]", "    Foo(Foo),", "    Bar,", "}", "fn f(t: Ty) { match t { Ty::Foo(x) => g(x) } }"]


def lines_of(path):
    return LINES


def locs(res, lines):
    return [(FILE, tuple(s)) for s in res or []]


def test_a_variant_declaration_is_a_name_at_the_start_of_a_line_of_an_enum_body():
    assert is_variant_declaration(LINES, 5, "Foo")
    assert is_variant_declaration(LINES, 6, "Bar")
    assert not is_variant_declaration(LINES, 1, "Foo")
    assert not is_variant_declaration(LINES, 8, "Foo")
    assert not is_variant_declaration(["struct S {", "    Foo(u8),", "}"], 2, "Foo")


def test_the_cursor_token_and_the_callee_before_the_parenthesis():
    assert cursor_token(LINES, 8, 29) == "Foo"
    assert cursor_token(LINES, 8, 0) == "fn"
    assert callee_before_paren(LINES, 8, 32) == ("Foo", 8, 28)
    assert callee_before_paren(LINES, 8, 13) is None
    assert callee_before_paren(LINES, 8, 10) == ("f", 8, 3)


def test_a_definition_that_became_the_variant_declaration_is_v1():
    cls, _ = classify_definition([(1, 12, 1, 15)], [(5, 5, 5, 8)], lines_of, "Foo", locs)
    assert cls == CLASS_DEFINITION
    assert classify_definition([(1, 12, 1, 15)], [(1, 12, 1, 15)], lines_of, "Foo", locs)[0] is None
    assert classify_definition([(1, 12, 1, 15)], [(6, 5, 6, 8)], lines_of, "Foo", locs)[0] is None
    assert classify_definition([(5, 5, 5, 8)], [(1, 12, 1, 15), (5, 5, 5, 8)], lines_of, "Foo", locs)[0] is None


def test_the_callee_is_found_on_an_earlier_line():
    lines = ["let x = Ty::Foo(Inner {", "    a: b.clone(),", "    c: 1,", "});"]
    assert callee_before_paren(lines, 3, 5) == ("Foo", 1, 12)
    assert callee_before_paren(lines, 4, 0) == ("Foo", 1, 12)


def test_a_definition_swapped_inside_a_macro_between_items_of_one_name_is_v5():
    lines = ["pub enum Fragment {", "    Expr(TokenStream),", "}", "pub struct Expr(pub Fragment);"]
    f = lambda p: lines
    swapped = classify_definition([(None, (4, 12, 4, 16))], [(FILE, (2, 5, 2, 9))], f, "Err", lambda r, l: [(FILE, tuple(s)) for _, s in r])
    assert swapped[0] == CLASS_SWAPPED
    same_token = classify_definition([(None, (4, 12, 4, 16))], [(FILE, (2, 5, 2, 9))], f, "Expr", lambda r, l: [(FILE, tuple(s)) for _, s in r])
    assert same_token[0] == CLASS_DEFINITION


def test_a_hover_that_names_the_variant_is_v2_only_with_a_variant_definition():
    assert classify_hover("```rust\nstruct Foo\n", "```rust\nvariant Foo\nType: ()\n", "Foo", True)[0] == CLASS_HOVER
    assert classify_hover("```rust\nstruct Foo\n", "```rust\nvariant Foo\nType: ()\n", "Foo", False)[0] is None
    assert classify_hover("```rust\nstruct Foo\n", "```rust\nstruct Foo\n", "Foo", True)[0] is None


def test_an_empty_signature_help_of_a_variant_callee_is_v4():
    assert classify_signature("path::Foo", True, "Foo", True)[0] == CLASS_SIGNATURE
    assert classify_signature("path::Bar", True, "Foo", True)[0] is None
    assert classify_signature("path::Foo", False, "Foo", True)[0] is None
    assert classify_signature("path::Foo", True, "Foo", False)[0] is None


def test_locations_gained_for_the_variant_and_lost_for_the_struct_are_v3():
    def def_of(path, line, col):
        return ITEM if line == 8 else STRUCT

    before, after = [(1, 12, 1, 15)], [(8, 22, 8, 25)]
    cls, ev = classify_variant_locations(before, after, FILE, lines_of, locs, def_of, ITEM, True)
    assert cls == CLASS_LOCATIONS and ev["added_count"] == 1 and ev["removed_count"] == 1
    assert classify_variant_locations(before, after, FILE, lines_of, locs, lambda *a: STRUCT, ITEM, True)[0] is None
    assert classify_variant_locations(before, after, FILE, lines_of, locs, lambda *a: ITEM, ITEM, True)[0] is None
    assert classify_variant_locations(before, after, FILE, lines_of, locs, def_of, ITEM, False)[0] is None
    assert classify_variant_locations(before, before, FILE, lines_of, locs, def_of, ITEM, True)[0] is None


def test_a_line_inside_the_open_brackets_of_a_macro_invocation_is_in_a_macro():
    lines = ["declare_enum!(", "    Copy => [],", "    PartialEq => [eq],", ");", "fn f() {}"]
    assert in_macro_invocation(lines, 3)
    assert not in_macro_invocation(lines, 5)
    assert is_variant_declaration(lines, 3, "PartialEq")
    assert not is_variant_declaration(["fn f() {", "    A,", "}"], 2, "A")


def test_the_declaration_site_and_macro_arguments_are_not_gained_locations_of_another_item():
    lines = ["pub enum Ty {", "    Foo(u8),", "}", "impl_from!(", "    Foo,", ");"]
    f = lambda p: lines
    ls = lambda res, l: [(FILE, tuple(s)) for s in res or []]

    def def_of(path, line, col):
        return ITEM if line == 8 else STRUCT

    # gained: the declaration site itself (line 2) and an argument of a macro invocation (line 5)
    cls, ev = classify_variant_locations([(1, 1, 1, 4)], [(2, 5, 2, 8), (5, 5, 5, 8)], FILE, f, ls, def_of, ("src/nodes.rs", 2), True)
    assert cls == CLASS_LOCATIONS and ev["added_generated"] == 2
