/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

// Pilot benchmark telemetry from bench.json / LSP_BENCHMARK_COMPARISON_REPORT.md (seconds).
let WORKSPACES = {
  "serde": {
    meta: "serde · 3 crates · warm cache · booster (32 cores)",
    target: "Pilot benchmark · 300 sampled positions · warm latencies (post-index)",
    rows: [
      ["Go to definition (warm p50)", 43e-6, "43 µs", 131e-6, "131 µs"],
      ["Go to definition (tail p95)", 365e-6, "365 µs", 6.89e-3, "6.89 ms"],
      ["Find references (warm p50)", 94e-6, "94 µs", 1.53e-3, "1.53 ms"],
      ["Find references (tail p95)", 1.55e-3, "1.55 ms", 34.2e-3, "34.20 ms"],
      ["Hover (warm p50)", 68e-6, "68 µs", 104e-6, "104 µs"],
      ["Hover (tail p95)", 112e-6, "112 µs", 221e-6, "221 µs"],
    ],
  },
  "ripgrep": {
    meta: "ripgrep · 7 crates · warm cache · booster (32 cores)",
    target: "Pilot benchmark · 300 sampled positions · warm latencies (post-index)",
    rows: [
      ["Go to definition (warm p50)", 50e-6, "50 µs", 186e-6, "186 µs"],
      ["Go to definition (tail p95)", 484e-6, "484 µs", 5.98e-3, "5.98 ms"],
      ["Find references (warm p50)", 103e-6, "103 µs", 1.01e-3, "1.01 ms"],
      ["Find references (tail p95)", 4.31e-3, "4.31 ms", 13.36e-3, "13.36 ms"],
      ["Hover (warm p50)", 78e-6, "78 µs", 132e-6, "132 µs"],
      ["Hover (tail p95)", 102e-6, "102 µs", 252e-6, "252 µs"],
    ],
  },
  "Tokio": {
    meta: "Tokio · 42 crates · 880 files · booster (32 cores)",
    target: "Pilot benchmark · 300 sampled positions · warm latencies (post-index)",
    rows: [
      ["Go to definition (warm p50)", 95e-6, "95 µs", 181e-6, "181 µs"],
      ["Go to definition (tail p95)", 201e-6, "201 µs", 29.29e-3, "29.29 ms"],
      ["Find references (warm p50)", 573e-6, "573 µs", 5.08e-3, "5.08 ms"],
      ["Find references (tail p95)", 2.31e-3, "2.31 ms", 73.39e-3, "73.39 ms"],
      ["Hover (warm p50)", 1.30e-3, "1.30 ms", 150e-6, "150 µs"],
      ["Hover (tail p95)", 1.50e-3, "1.50 ms", 373e-6, "373 µs"],
    ],
  },
  "bevy": {
    meta: "bevy · 147 crates · 2,800 files · booster (32 cores)",
    target: "Pilot benchmark · 300 sampled positions · warm latencies (post-index)",
    rows: [
      ["Go to definition (warm p50)", 245e-6, "245 µs", 280e-6, "280 µs"],
      ["Go to definition (tail p95)", 590e-6, "590 µs", 148.02e-3, "148.02 ms"],
      ["Find references (warm p50)", 3.88e-3, "3.88 ms", 13.02e-3, "13.02 ms"],
      ["Find references (tail p95)", 36.84e-3, "36.84 ms", 167.81e-3, "167.81 ms"],
      ["Hover (warm p50)", 3.96e-3, "3.96 ms", 215e-6, "215 µs"],
      ["Hover (tail p95)", 11.21e-3, "11.21 ms", 545e-6, "545 µs"],
    ],
  },
  "rust-analyzer": {
    meta: "rust-analyzer · 261 crates · 3,050 files · booster (32 cores)",
    target: "Pilot benchmark · 300 sampled positions · warm latencies (post-index)",
    rows: [
      ["Go to definition (warm p50)", 174e-6, "174 µs", 314e-6, "314 µs"],
      ["Go to definition (tail p95)", 607e-6, "607 µs", 53.28e-3, "53.28 ms"],
      ["Find references (warm p50)", 654e-6, "654 µs", 1.57e-3, "1.57 ms"],
      ["Find references (tail p95)", 5.97e-3, "5.97 ms", 133.05e-3, "133.05 ms"],
      ["Hover (warm p50)", 436e-6, "436 µs", 228e-6, "228 µs"],
      ["Hover (tail p95)", 969e-6, "969 µs", 515e-6, "515 µs"],
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
  box.replaceChildren();
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

function renderWorkspaces(wsData) {
  tabs("ws-tabs", Object.keys(wsData), (name) => {
    const ws = wsData[name];
    document.getElementById("ws-meta").textContent = ws.meta;
    document.getElementById("ws-target").textContent = ws.target;
    const rows = document.getElementById("ws-rows");
    rows.replaceChildren(...ws.rows.map(([cap, , rl, , al]) => el("div", "cap",
      `<b>${cap}</b>
       <div class="bar-row"><span class="mono rust">ridx</span><div class="track"><div class="fr" style="width:0"></div></div><b class="mono">${rl}</b></div>
       <div class="bar-row"><span class="mono mute">rust-analyzer</span><div class="track"><div class="fa" style="width:0"></div></div><b class="mono mute">${al}</b></div>`)));
    requestAnimationFrame(() => {
      ws.rows.forEach(([, r, , a], i) => {
        const cap = rows.children[i];
        if (cap) {
          cap.querySelector(".fr").style.width = `${pct(r)}%`;
          cap.querySelector(".fa").style.width = `${pct(a)}%`;
        }
      });
    });
  });
}

renderWorkspaces(WORKSPACES);

fetch("bench.json")
  .then((r) => (r.ok ? r.json() : null))
  .then((data) => {
    if (data && data.workspaces) {
      renderWorkspaces(data.workspaces);
    }
  })
  .catch(() => {});

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
