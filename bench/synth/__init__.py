# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Deterministic synthetic multi-crate Rust workspace generator and 100% ground-truth LSP oracle."""

from .oracle_eval import evaluate_synth_oracle
from .source_builder import SourceBuilder, SpanRecord
from .workspace_gen import generate_synth_workspace

__all__ = [
    "SourceBuilder",
    "SpanRecord",
    "evaluate_synth_oracle",
    "generate_synth_workspace",
]
