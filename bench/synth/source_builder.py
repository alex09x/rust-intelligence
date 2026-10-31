# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Line- and column-accurate Rust source builder that records exact ground-truth token coordinates."""

from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Dict, List, Optional, Tuple


@dataclass(frozen=True)
class SpanRecord:
    """Exact 0-based LSP coordinate span for a generated token or construct."""

    rel_path: str
    line: int
    col: int
    end_line: int
    end_col: int
    text: str
    role: str
    symbol_id: str

    def to_dict(self) -> dict:
        return asdict(self)

    def cursor_pos(self, offset: int = 0) -> dict:
        return {
            "rel_path": self.rel_path,
            "line": self.line,
            "col": min(self.end_col, self.col + offset),
            "end_line": self.end_line,
            "end_col": self.end_col,
            "text": self.text,
            "role": self.role,
            "symbol_id": self.symbol_id,
        }


class SourceBuilder:
    """Constructs a single Rust file while recording exact 0-based (line, col) spans for tagged tokens.

    Lines are emitted using `line_tokens(*parts)` where each part is either a plain `str` or a tuple
    `(symbol_id, role, text)`. Every tagged tuple is recorded as a `SpanRecord` at its exact UTF-16/ASCII
    column offset on the current line.
    """

    def __init__(self, rel_path: str):
        self.rel_path = rel_path.replace("\\", "/")
        self.lines: List[str] = []
        self.spans: List[SpanRecord] = []
        self.by_symbol: Dict[str, List[SpanRecord]] = {}
        self.folds: List[Tuple[int, int, str]] = []

    @property
    def current_line(self) -> int:
        return len(self.lines)

    def blank(self, count: int = 1) -> None:
        for _ in range(count):
            self.lines.append("")

    def raw_line(self, text: str = "") -> int:
        idx = len(self.lines)
        self.lines.append(text)
        return idx

    def line_tokens(self, *parts) -> Tuple[int, List[SpanRecord]]:
        """Emits one line from `parts` (`str` or `(symbol_id, role, text)`) and returns `(line_idx, recorded_spans)`."""
        line_idx = len(self.lines)
        col = 0
        buf: List[str] = []
        recorded: List[SpanRecord] = []
        for part in parts:
            if isinstance(part, str):
                buf.append(part)
                col += len(part)
            elif isinstance(part, tuple) and len(part) == 3:
                sym_id, role, text = part
                start_col = col
                end_col = col + len(text)
                buf.append(text)
                col = end_col
                rec = SpanRecord(
                    rel_path=self.rel_path,
                    line=line_idx,
                    col=start_col,
                    end_line=line_idx,
                    end_col=end_col,
                    text=text,
                    role=role,
                    symbol_id=sym_id,
                )
                self.spans.append(rec)
                self.by_symbol.setdefault(sym_id, []).append(rec)
                recorded.append(rec)
            else:
                raise TypeError(f"Unsupported token part: {part!r}")
        self.lines.append("".join(buf))
        return line_idx, recorded

    def record_fold(self, start_line: int, end_line: int, kind: str = "region") -> None:
        if end_line > start_line:
            self.folds.append((start_line, end_line, kind))

    def get_def(self, symbol_id: str, role: str = "def") -> SpanRecord:
        for rec in self.by_symbol.get(symbol_id, ()):
            if rec.role == role:
                return rec
        raise KeyError(f"Missing {role!r} span for symbol {symbol_id!r} in {self.rel_path}")

    def get_spans(self, symbol_id: str, roles: Optional[Tuple[str, ...]] = None) -> List[SpanRecord]:
        items = self.by_symbol.get(symbol_id, [])
        if roles is None:
            return list(items)
        return [r for r in items if r.role in roles]

    def render(self) -> str:
        return "\n".join(self.lines) + "\n"

    def write(self, root_dir: Path) -> Path:
        full_path = root_dir / self.rel_path
        full_path.parent.mkdir(parents=True, exist_ok=True)
        full_path.write_text(self.render(), encoding="utf-8")
        return full_path

    def verify_spans(self) -> None:
        """Asserts that every recorded SpanRecord matches the exact substring in `self.lines`."""
        for rec in self.spans:
            assert rec.line == rec.end_line, f"Multi-line token not supported in verify_spans: {rec}"
            actual_line = self.lines[rec.line]
            actual_slice = actual_line[rec.col:rec.end_col]
            if actual_slice != rec.text:
                raise AssertionError(
                    f"Coordinate mismatch in {self.rel_path}:{rec.line}:{rec.col}-{rec.end_col}: "
                    f"expected {rec.text!r}, got {actual_slice!r}"
                )
