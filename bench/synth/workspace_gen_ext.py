# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Extended synthetic Rust constructs for all 13 rename families, blanket/ref impls, and refusals."""

from .source_builder import SourceBuilder


def emit_rename_family_extensions(
    sb_models: SourceBuilder,
    sb_submod: SourceBuilder,
    tag: str,
    fam: dict,
    rng=None,
) -> None:
    """Emits constructs covering all 13 rename families, seed-driven combinatorics, and refusal targets for `tag`."""
    has_box_impl = bool(rng.getrandbits(1)) if rng is not None else True
    has_ufcs_call = bool(rng.getrandbits(1)) if rng is not None else True
    has_extra_impl = bool(rng.getrandbits(1)) if rng is not None else False
    struct_alt = fam["struct_alt"]
    enum_state = fam["enum_state"]
    var_ready = fam["var_ready"]
    var_busy = fam["var_busy"]

    # Submodule file content (`crates/core_pkg/src/sub_mod_<tag>.rs`)
    fn_sub_help = f"sub_helper_{tag}"
    fsh_s, _ = sb_submod.line_tokens("pub fn ", (fn_sub_help, "def", fn_sub_help), "(x: i64) -> i64 {")
    sb_submod.raw_line("    x + 1")
    fsh_e = sb_submod.raw_line("}")
    sb_submod.record_fold(fsh_s, fsh_e)

    # Family 13: Enum variant + Enum prefix references
    fn_check_state = f"check_state_{tag}"
    fcs_s, _ = sb_models.line_tokens(
        "pub fn ", (fn_check_state, "def", fn_check_state), "(s: ", (enum_state, "type_ref", enum_state), ") -> i64 {"
    )
    sb_models.raw_line("    match s {")
    sb_models.line_tokens("        ", (enum_state, "prefix_ref", enum_state), "::", (var_ready, "ref", var_ready), " => 0,")
    sb_models.line_tokens("        ", (enum_state, "prefix_ref", enum_state), "::", (var_busy, "ref", var_busy), "(v) => v,")
    sb_models.raw_line("    }")
    fcs_e = sb_models.raw_line("}")
    sb_models.record_fold(fcs_s, fcs_e)
    sb_models.blank()

    # Dedicated carrier struct and trait for `&T` ref impl + blanket impl + associated items + Display refusal
    struct_carrier = f"AssocCarrier_{tag}"
    field_payload = f"payload_{tag}"
    sc_s, _ = sb_models.line_tokens("pub struct ", (struct_carrier, "def", struct_carrier), " {")
    sb_models.line_tokens("    pub ", (field_payload, "def", field_payload), ": i64,")
    sc_e = sb_models.raw_line("}")
    sb_models.record_fold(sc_s, sc_e)
    sb_models.blank()

    trait_rb = f"RefBlanketTrait_{tag}"
    fn_rb_step = f"rb_step_{tag}"
    trb_s, _ = sb_models.line_tokens("pub trait ", (trait_rb, "def", trait_rb), " {")
    sb_models.line_tokens("    fn ", (fn_rb_step, "decl", fn_rb_step), "(&self, delta: i64) -> i64;")
    trb_e = sb_models.raw_line("}")
    sb_models.record_fold(trb_s, trb_e)

    irb1_s, _ = sb_models.line_tokens(
        "impl ", (trait_rb, "trait_impl_ref", trait_rb), " for ", (struct_carrier, "rb_impl_target", struct_carrier), " {"
    )
    frb1_s, _ = sb_models.line_tokens("    fn ", (fn_rb_step, "impl_def", fn_rb_step), "(&self, delta: i64) -> i64 {")
    sb_models.line_tokens("        self.", (field_payload, "ref", field_payload), " + delta")
    frb1_e = sb_models.raw_line("    }")
    sb_models.record_fold(frb1_s, frb1_e)
    irb1_e = sb_models.raw_line("}")
    sb_models.record_fold(irb1_s, irb1_e)

    irb2_s, _ = sb_models.line_tokens(
        "impl<T: ", (trait_rb, "trait_bound_ref", trait_rb), " + ?Sized> ",
        (trait_rb, "trait_impl_ref", trait_rb), " for &T {"
    )
    frb2_s, _ = sb_models.line_tokens("    fn ", (fn_rb_step, "impl_def_ref", fn_rb_step), "(&self, delta: i64) -> i64 {")
    sb_models.line_tokens("        (**self).", (fn_rb_step, "call_ref", fn_rb_step), "(delta)")
    frb2_e = sb_models.raw_line("    }")
    sb_models.record_fold(frb2_s, frb2_e)
    irb2_e = sb_models.raw_line("}")
    sb_models.record_fold(irb2_s, irb2_e)

    if has_box_impl:
        irb3_s, _ = sb_models.line_tokens(
            "impl<T: ", (trait_rb, "trait_bound_ref", trait_rb), " + ?Sized> ",
            (trait_rb, "trait_impl_ref", trait_rb), " for Box<T> {"
        )
        frb3_s, _ = sb_models.line_tokens("    fn ", (fn_rb_step, "impl_def_ref", fn_rb_step), "(&self, delta: i64) -> i64 {")
        sb_models.line_tokens("        (**self).", (fn_rb_step, "call_ref", fn_rb_step), "(delta)")
        frb3_e = sb_models.raw_line("    }")
        sb_models.record_fold(frb3_s, frb3_e)
        irb3_e = sb_models.raw_line("}")
        sb_models.record_fold(irb3_s, irb3_e)

    trait_blanket = f"BlanketConsumer_{tag}"
    fn_blanket = f"consume_rb_{tag}"
    tb_s, _ = sb_models.line_tokens("pub trait ", (trait_blanket, "def", trait_blanket), " {")
    sb_models.line_tokens("    fn ", (fn_blanket, "decl", fn_blanket), "(&self) -> i64;")
    tb_e = sb_models.raw_line("}")
    sb_models.record_fold(tb_s, tb_e)
    ib_s, _ = sb_models.line_tokens(
        "impl<T: ", (trait_rb, "trait_bound_ref", trait_rb), "> ",
        (trait_blanket, "trait_impl_ref", trait_blanket), " for T {"
    )
    fb_s, _ = sb_models.line_tokens("    fn ", (fn_blanket, "impl_def", fn_blanket), "(&self) -> i64 {")
    if has_ufcs_call:
        sb_models.line_tokens("        <T as ", (trait_rb, "trait_ref", trait_rb), ">::", (fn_rb_step, "call_ref", fn_rb_step), "(self, 1)")
    else:
        sb_models.line_tokens("        self.", (fn_rb_step, "call_ref", fn_rb_step), "(1)")
    fb_e = sb_models.raw_line("    }")
    sb_models.record_fold(fb_s, fb_e)
    ib_e = sb_models.raw_line("}")
    sb_models.record_fold(ib_s, ib_e)
    sb_models.blank()

    # Family 2: Struct field shorthand (both field rename and local rename)
    struct_sh = f"ShorthandBox_{tag}"
    field_sh = f"count_{tag}"
    local_sh = f"sh_local_{tag}"
    fn_make_sh = f"make_shorthand_{tag}"
    ssh_s, _ = sb_models.line_tokens("pub struct ", (struct_sh, "def", struct_sh), " {")
    sb_models.line_tokens("    pub ", (field_sh, "def", field_sh), ": i64,")
    sb_models.line_tokens("    pub ", (local_sh, "field_keep", local_sh), ": i64,")
    ssh_e = sb_models.raw_line("}")
    sb_models.record_fold(ssh_s, ssh_e)
    fmsh_s, _ = sb_models.line_tokens(
        "pub fn ", (fn_make_sh, "def", fn_make_sh), "(",
        (field_sh, "local_keep", field_sh), ": i64, ",
        (local_sh, "local_def", local_sh), ": i64) -> ", (struct_sh, "type_ref", struct_sh), " {"
    )
    sb_models.line_tokens(
        "    let b = ", (struct_sh, "type_ref", struct_sh), " { ",
        (field_sh, "shorthand_field", field_sh), ", ",
        (local_sh, "shorthand_local", local_sh), " };"
    )
    sb_models.line_tokens("    let _ = b.", (field_sh, "ref", field_sh), ";")
    sb_models.raw_line("    b")
    fmsh_e = sb_models.raw_line("}")
    sb_models.record_fold(fmsh_s, fmsh_e)
    sb_models.blank()

    # Family 5: `use ... as` alias (both renaming the alias and renaming the original symbol)
    struct_orig_alias = f"OrigForAlias_{tag}"
    alias_kept = f"KeptAlias_{tag}"
    alias_target = f"AliasedAlt_{tag}"
    fn_use_alias = f"use_aliases_{tag}"
    sb_models.line_tokens("pub struct ", (struct_orig_alias, "def", struct_orig_alias), "(pub i64);")
    sb_models.line_tokens(
        "use self::", (struct_orig_alias, "use_orig", struct_orig_alias), " as ", (alias_kept, "alias_keep", alias_kept), ";"
    )
    sb_models.line_tokens("use self::", (struct_alt, "use_ref", struct_alt), " as ", (alias_target, "alias_def", alias_target), ";")
    fua_s, _ = sb_models.line_tokens(
        "pub fn ", (fn_use_alias, "def", fn_use_alias), "(a: ", (alias_target, "alias_ref", alias_target), ") -> i64 {"
    )
    sb_models.line_tokens(
        "    let o = ", (alias_kept, "alias_keep_ref", alias_kept), "(a.", (fam["field_w"], "ref", fam["field_w"]), ");"
    )
    sb_models.raw_line("    o.0")
    fua_e = sb_models.raw_line("}")
    sb_models.record_fold(fua_s, fua_e)
    sb_models.blank()

    # Family 7: Macro callsite
    macro_name = f"apply_op_{tag}"
    fn_macro_tgt = f"macro_target_{tag}"
    fn_macro_caller = f"call_via_macro_{tag}"
    mc_s, _ = sb_models.line_tokens("macro_rules! ", (macro_name, "def", macro_name), " {")
    sb_models.raw_line("    ($e:expr) => { $e + 1 };")
    mc_e = sb_models.raw_line("}")
    sb_models.record_fold(mc_s, mc_e)
    fmt_s, _ = sb_models.line_tokens("pub fn ", (fn_macro_tgt, "def", fn_macro_tgt), "(v: i64) -> i64 {")
    sb_models.raw_line("    v * 2")
    fmt_e = sb_models.raw_line("}")
    sb_models.record_fold(fmt_s, fmt_e)
    fmc_s, _ = sb_models.line_tokens("pub fn ", (fn_macro_caller, "def", fn_macro_caller), "(x: i64) -> i64 {")
    sb_models.line_tokens("    ", (macro_name, "call_ref", macro_name), "!(", (fn_macro_tgt, "call_ref", fn_macro_tgt), "(x))")
    fmc_e = sb_models.raw_line("}")
    sb_models.record_fold(fmc_s, fmc_e)
    sb_models.blank()

    # Family 8: `Self` keyword inside `impl`
    struct_self_ty = f"SelfBox_{tag}"
    fn_self_make = f"make_self_{tag}"
    sst_s, _ = sb_models.line_tokens("pub struct ", (struct_self_ty, "def", struct_self_ty), " {")
    sb_models.raw_line("    pub v: i64,")
    sst_e = sb_models.raw_line("}")
    sb_models.record_fold(sst_s, sst_e)
    ist_s, _ = sb_models.line_tokens("impl ", (struct_self_ty, "inherent_impl", struct_self_ty), " {")
    fsm_s, _ = sb_models.line_tokens(
        "    pub fn ", (fn_self_make, "def", fn_self_make), "(v: i64) -> ", (f"self_kw_{tag}", "self_cursor", "Self"), " {"
    )
    sb_models.raw_line("        Self { v }")
    fsm_e = sb_models.raw_line("    }")
    sb_models.record_fold(fsm_s, fsm_e)
    ist_e = sb_models.raw_line("}")
    sb_models.record_fold(ist_s, ist_e)
    sb_models.blank()

    # Family 10 & 11: Generic parameter `T` and Lifetime `'a`
    fn_generic = f"generic_id_{tag}"
    ty_param = f"ParamT_{tag}"
    fn_lifetime = f"select_slice_{tag}"
    lt_param = f"lt_{tag}"
    fg_s, _ = sb_models.line_tokens(
        "pub fn ", (fn_generic, "def", fn_generic), "<", (ty_param, "def", ty_param), ": Clone>(x: ",
        (ty_param, "ref", ty_param), ") -> ", (ty_param, "ref", ty_param), " {"
    )
    sb_models.line_tokens("    let cloned: ", (ty_param, "ref", ty_param), " = x.clone();")
    sb_models.raw_line("    cloned")
    fg_e = sb_models.raw_line("}")
    sb_models.record_fold(fg_s, fg_e)
    fl_s, _ = sb_models.line_tokens(
        "pub fn ", (fn_lifetime, "def", fn_lifetime), "<", (lt_param, "def", f"'{lt_param}"), ">(s: &",
        (lt_param, "ref", f"'{lt_param}"), " str) -> &", (lt_param, "ref", f"'{lt_param}"), " str {"
    )
    sb_models.raw_line("    s")
    fl_e = sb_models.raw_line("}")
    sb_models.record_fold(fl_s, fl_e)
    sb_models.blank()

    # Family 12: Associated type and Associated const
    trait_assoc = f"AssocSpec_{tag}"
    assoc_ty = f"OutputTy_{tag}"
    assoc_const = f"MAX_LIMIT_{tag}"
    fn_read_assoc = f"read_assoc_{tag}"
    ta_s, _ = sb_models.line_tokens("pub trait ", (trait_assoc, "def", trait_assoc), " {")
    sb_models.line_tokens("    type ", (assoc_ty, "decl", assoc_ty), ";")
    sb_models.line_tokens("    const ", (assoc_const, "decl", assoc_const), ": i64;")
    ta_e = sb_models.raw_line("}")
    sb_models.record_fold(ta_s, ta_e)
    ia_s, _ = sb_models.line_tokens(
        "impl ", (trait_assoc, "trait_impl_ref", trait_assoc), " for ", (struct_carrier, "assoc_impl_target", struct_carrier), " {"
    )
    sb_models.line_tokens("    type ", (assoc_ty, "impl_def", assoc_ty), " = i64;")
    sb_models.line_tokens("    const ", (assoc_const, "impl_def", assoc_const), ": i64 = 100;")
    ia_e = sb_models.raw_line("}")
    sb_models.record_fold(ia_s, ia_e)
    fra_s, _ = sb_models.line_tokens(
        "pub fn ", (fn_read_assoc, "def", fn_read_assoc), "() -> <",
        (struct_carrier, "type_ref", struct_carrier), " as ", (trait_assoc, "trait_ref", trait_assoc), ">::",
        (assoc_ty, "proj_ref", assoc_ty), " {"
    )
    sb_models.line_tokens(
        "    <", (struct_carrier, "type_ref", struct_carrier), " as ", (trait_assoc, "trait_ref", trait_assoc), ">::",
        (assoc_const, "proj_ref", assoc_const)
    )
    fra_e = sb_models.raw_line("}")
    sb_models.record_fold(fra_s, fra_e)
    sb_models.blank()

    # Refusal target: External trait implementation (`core::fmt::Display::fmt`) and keyword cursor
    fn_carrier_op = f"carrier_op_{tag}"
    idisp_s, _ = sb_models.line_tokens("impl core::fmt::Display for ", (struct_carrier, "disp_target", struct_carrier), " {")
    fdisp_s, _ = sb_models.line_tokens(
        "    ", (f"kw_fn_{tag}", "kw_cursor", "fn"), " ", (f"ext_fmt_{tag}", "ext_trait_fn", "fmt"),
        "(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {"
    )
    sb_models.line_tokens("        write!(f, \"{}\", self.", (fn_carrier_op, "ref", fn_carrier_op), "())")
    fdisp_e = sb_models.raw_line("    }")
    sb_models.record_fold(fdisp_s, fdisp_e)
    idisp_e = sb_models.raw_line("}")
    sb_models.record_fold(idisp_s, idisp_e)
    sb_models.blank()

    # Inherent impl immediately after external Display impl (tests brace-depth & target_line scoping)
    icop_s, _ = sb_models.line_tokens("impl ", (struct_carrier, "inherent_impl", struct_carrier), " {")
    fcop_s, _ = sb_models.line_tokens("    pub fn ", (fn_carrier_op, "def", fn_carrier_op), "(&self) -> i64 {")
    sb_models.line_tokens("        self.", (field_payload, "ref", field_payload), " + 1")
    fcop_e = sb_models.raw_line("    }")
    sb_models.record_fold(fcop_s, fcop_e)
    icop_e = sb_models.raw_line("}")
    sb_models.record_fold(icop_s, icop_e)
    sb_models.blank()

    extra_struct = f"ExtraCarrier_{tag}" if has_extra_impl else None
    if extra_struct is not None:
        es_s, _ = sb_models.line_tokens("pub struct ", (extra_struct, "def", extra_struct), "(pub i64);")
        sb_models.record_fold(es_s, es_s)
        ies_s, _ = sb_models.line_tokens(
            "impl ", (trait_rb, "trait_impl_ref", trait_rb), " for ", (extra_struct, "rb_impl_target", extra_struct), " {"
        )
        fes_s, _ = sb_models.line_tokens("    fn ", (fn_rb_step, "impl_def_ref", fn_rb_step), "(&self, delta: i64) -> i64 {")
        sb_models.raw_line("        self.0 + delta")
        fes_e = sb_models.raw_line("    }")
        sb_models.record_fold(fes_s, fes_e)
        ies_e = sb_models.raw_line("}")
        sb_models.record_fold(ies_s, ies_e)
        sb_models.blank()

    fam.update({
        "sub_mod": f"sub_mod_{tag}",
        "fn_sub_help": fn_sub_help,
        "fn_check_state": fn_check_state,
        "struct_carrier": struct_carrier,
        "field_payload": field_payload,
        "trait_rb": trait_rb,
        "fn_rb_step": fn_rb_step,
        "trait_blanket": trait_blanket,
        "fn_blanket": fn_blanket,
        "struct_sh": struct_sh,
        "field_sh": field_sh,
        "local_sh": local_sh,
        "fn_make_sh": fn_make_sh,
        "struct_orig_alias": struct_orig_alias,
        "alias_kept": alias_kept,
        "alias_target": alias_target,
        "fn_use_alias": fn_use_alias,
        "macro_name": macro_name,
        "fn_macro_tgt": fn_macro_tgt,
        "fn_macro_caller": fn_macro_caller,
        "struct_self_ty": struct_self_ty,
        "fn_self_make": fn_self_make,
        "self_kw": f"self_kw_{tag}",
        "fn_generic": fn_generic,
        "ty_param": ty_param,
        "fn_lifetime": fn_lifetime,
        "lt_param": lt_param,
        "trait_assoc": trait_assoc,
        "assoc_ty": assoc_ty,
        "assoc_const": assoc_const,
        "fn_read_assoc": fn_read_assoc,
        "kw_fn": f"kw_fn_{tag}",
        "ext_fmt": f"ext_fmt_{tag}",
        "fn_carrier_op": fn_carrier_op,
        "extra_struct": extra_struct,
    })
