/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

// Preliminary numbers from README / LSP_BENCHMARK_COMPARISON_REPORT.md (seconds).
const WORKSPACES = {
  "Tokio": {
    meta: "Tokio · 42 crates · 880 files",
    target: "runtime::context::blocking::try_enter_blocking_region",
    rows: [
      ["Go to definition", 24e-6, "24 µs", 10.554, "10,554 ms"],
      ["Find references", 655e-6, "655 µs", 1.273, "1,273 ms"],
      ["Hover", 42e-6, "42 µs", 0.344, "344 ms"],
      ["Incoming calls", 262e-6, "262 µs", 0.363, "363 ms"],
      ["Workspace symbols (Runtime)", 16.9e-3, "16.9 ms", 0.323, "323 ms"],
    ],
  },
  "rust-analyzer": {
    meta: "rust-analyzer · 261 crates · 3,050 files",
    target: "ide::AnalysisHost",
    rows: [
      ["Go to definition", 704e-6, "704 µs", 10.905, "10,905 ms"],
      ["Find references", 4.06e-3, "4.06 ms", 1.487, "1,487 ms"],
      ["Hover", 410e-6, "410 µs", 0.381, "381 ms"],
      ["Incoming calls", 1.61e-3, "1.61 ms", 0.382, "382 ms"],
    ],
  },
};

const ARCH = {
  editor: ["your editor process", "LSP client", "Any editor that speaks the Language Server Protocol. It launches ridx lsp and talks to it over stdin/stdout.", ["No plugin of its own required", "Same binary for every editor"]],
  lsp: ["ridx process", "ridx lsp", "A small server that routes requests. It answers from the mapped index and overlays your unsaved edits on top.", ["JSON-RPC over stdio", "Honours $/cancelRequest", "Multi-root workspaces routed by path"]],
  overlay: ["RAM · only what you have open", "OverlayManager", "Holds the buffers you are editing right now. Answers reflect what is on screen, before you save.", ["Live incremental edits", "Lexical scoping for rename", "Transactional WorkspaceEdit with rollback"]],
  index: ["mmap · OS page cache", "WorkspaceIndex", "Maps the .ridx shards for this workspace. The kernel pages data in as queries touch it and can drop it under memory pressure.", ["O(log N) binary search", "Scoped strictly to Cargo.lock", "Shard files closed after mapping"]],
  fly: ["background thread", "Flycheck", "On save, runs cargo check in the background and publishes compiler diagnostics without blocking requests.", ["Separate target/ridx-flycheck dir", "Previous run cancelled on new save", "Clears fixed errors"]],
  t1: ["disk · shared by all projects", "Tier 1 · Global CAS", "crates.io and git dependencies are indexed once per source hash and target, then reused by every workspace and worktree on the machine.", ["~/.cache/ridx/v2/objects/", "Immutable, content-addressed", "Linked into a project, not rebuilt"]],
  t2: ["disk · inside your project", "Tier 2 · Workspace variant", "Shards for your own workspace members, plus a manifest that whitelists exactly the crates in your Cargo.lock.", ["target/ridx/variants/<id>/", "ridx_manifest.json whitelist", "No symbols leak between projects"]],
  driver: ["build time · inside rustc", "ridx-driver", "A rustc wrapper. While cargo check compiles, it records definitions, references and inferred types straight from the compiler and writes .ridx shards.", ["Uses rustc’s own resolution", "No second analyzer to disagree", "Shards can ship via a CAS build cache"]],
};

const FORMAT = [
  ["Header · magic RIDX, v2", "offsets & counts"], ["String intern pool", "UTF-8, deduped"],
  ["File path table", "16 B / entry"], ["Definitions table", "40 B / entry"],
  ["Definitions hash index", "FNV-1a, 16 B"], ["References table (sorted by span)", "48 B / entry"],
  ["References target hash index", "16 B"], ["Types table", "28 B / entry"],
];
const TONES = ["#f74c00", "#ff9900", "#ffb347", "#00d2ff", "#38bdf8", "#7dd3fc", "#c4b5fd", "#a78bfa"];

const LSP = [
  ["Navigation", "definition · declaration · implementation · typeDefinition"],
  ["References", "references · documentHighlight"],
  ["Intelligence", "hover · completion · signatureHelp · inlayHint"],
  ["Hierarchies", "call hierarchy · type hierarchy"],
  ["Refactoring", "rename · codeAction · executeCommand"],
  ["Formatting", "formatting · rangeFormatting · onTypeFormatting"],
  ["Symbols", "documentSymbol · workspace/symbol"],
  ["Diagnostics", "publishDiagnostics (background cargo check) · pull diagnostics"],
  ["Workspaces", "didChangeWorkspaceFolders · watched files"],
];

const EDITORS = {
  "VS Code": `// settings.json — point the rust-analyzer extension at a
// wrapper script that runs: exec ridx lsp "$@"
{
  "rust-analyzer.server.path": "~/.local/bin/ridx-lsp"
}`,
  "Neovim": `vim.lsp.config("ridx", {
  cmd = { "ridx", "lsp" },
  filetypes = { "rust" },
  root_markers = { "Cargo.toml" },
})
vim.lsp.enable("ridx")`,
  "Helix": `# languages.toml
[language-server.ridx]
command = "ridx"
args = ["lsp"]

[[language]]
name = "rust"
language-servers = ["ridx"]`,
  "Zed": `{
  "lsp": {
    "rust-analyzer": {
      "binary": { "path": "ridx", "arguments": ["lsp"] }
    }
  }
}`,
};

const el = (tag, cls, html) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (html !== undefined) e.innerHTML = html;
  return e;
};
const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;");

function tabs(boxId, keys, onSelect) {
  const box = document.getElementById(boxId);
  const buttons = keys.map((k) => {
    const b = el("button", "chip");
    b.textContent = k;
    b.addEventListener("click", () => select(k));
    box.appendChild(b);
    return b;
  });
  function select(k) {
    buttons.forEach((b) => b.classList.toggle("on", b.textContent === k));
    onSelect(k);
  }
  select(keys[0]);
}

// Log scale from 10 µs to 20 s.
const pct = (s) => Math.max(1.5, Math.min(100, (Math.log10(s / 1e-5) / Math.log10(2e6)) * 100));

tabs("ws-tabs", Object.keys(WORKSPACES), (name) => {
  const ws = WORKSPACES[name];
  document.getElementById("ws-meta").textContent = ws.meta;
  document.getElementById("ws-target").textContent = `target: ${ws.target}`;
  const rows = document.getElementById("ws-rows");
  rows.replaceChildren(...ws.rows.map(([cap, , rl, , al]) => el("div", "cap",
    `<b>${cap}</b>
     <div class="bar-row"><span class="mono rust">ridx</span><div class="track"><div class="fr" style="width:0"></div></div><b class="mono">${rl}</b></div>
     <div class="bar-row"><span class="mono mute">rust-analyzer</span><div class="track"><div class="fa" style="width:0"></div></div><b class="mono mute">${al}</b></div>`)));
  requestAnimationFrame(() => {
    ws.rows.forEach(([, r, , a], i) => {
      const cap = rows.children[i];
      cap.querySelector(".fr").style.width = `${pct(r)}%`;
      cap.querySelector(".fa").style.width = `${pct(a)}%`;
    });
  });
});

const archButtons = [...document.querySelectorAll("[data-arch]")];
const archDetail = document.getElementById("arch-detail");
function showArch(key) {
  archButtons.forEach((b) => b.classList.toggle("on", b.dataset.arch === key));
  const [where, title, body, facts] = ARCH[key];
  const list = el("ul", "", facts.map((f) => `<li>${esc(f)}</li>`).join(""));
  archDetail.replaceChildren(el("span", "eye", esc(where)), el("span", "h3", esc(title)), el("p", "", esc(body)), list);
}
archButtons.forEach((b) => b.addEventListener("click", () => showArch(b.dataset.arch)));
showArch("index");

const fmt = document.getElementById("fmt");
FORMAT.forEach(([n, s], i) => {
  const seg = el("div", "seg", `<span>${n}</span><span class="mute">${s}</span>`);
  seg.style.borderLeft = `3px solid ${TONES[i]}`;
  fmt.appendChild(seg);
});

const mtx = document.getElementById("mtx");
LSP.forEach(([c, m]) => { mtx.append(el("div", "", `<b>${c}</b>`), el("div", "m", m)); });

tabs("ed-tabs", Object.keys(EDITORS), (name) => {
  document.getElementById("ed-code").textContent = EDITORS[name];
});

document.querySelectorAll("[data-copy]").forEach((b) => b.addEventListener("click", async () => {
  try {
    await navigator.clipboard.writeText(document.getElementById(b.dataset.copy).textContent);
    b.textContent = "Copied";
  } catch {
    b.textContent = "Copy failed";
  }
  setTimeout(() => { b.textContent = "Copy"; }, 1400);
}));
