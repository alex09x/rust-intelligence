/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

const BENCH = {
  "Tokio": { crates: 42, def: "8 µs", refs: "74 µs", hover: "16 µs", startup: "0.31 ms" },
  "prod-code": { crates: 358, def: "12 µs", refs: "210 µs", hover: "24 µs" },
  "sonic-rs": { crates: 29, def: "9 µs", refs: "88 µs", hover: "19 µs" },
  "BTCR": { crates: 16, def: "66 µs", refs: "4,264 µs", hover: "167 µs" },
};

const VS = {
  def: { ridx: 8e-6, ra: 0.33, ridxLabel: "8 µs", raLabel: "330 ms", note: "Go-to-Definition latency on Tokio. Bar length on a log scale." },
  heap: { ridx: 0, ra: 3.3e9, ridxLabel: "0.00 MB", raLabel: "3.3 GB", note: "Resident background heap after indexing Tokio." },
};

const ARCH = {
  driver: "<b>Compile-time extraction.</b> ridx-driver runs as RUSTC_WRAPPER, so symbols, spans and resolved references come from the compiler itself rather than a second analyzer.",
  cas: "<b>2-Tier content-addressed cache</b> in ~/.cache/ridx/v2/. Per-crate artifacts are keyed by content hash and shared across workspaces and branches; unchanged crates are never re-extracted.",
  mmap: "<b>Zero-copy mmap kernel.</b> The flat index is memory-mapped and queried in place: no deserialization and no heap-resident symbol tables, so idle memory stays at zero.",
  overlay: "<b>OverlayManager.</b> Unsaved editor buffers sit on top of the on-disk index, so answers reflect what is in the editor, and edits are applied as transactional WorkspaceEdits.",
};

const EDITORS = {
  "VS Code": `// 1. Wrapper script ~/.local/bin/ridx-lsp (chmod +x):
//      #!/bin/sh
//      exec ridx lsp "$@"
// 2. settings.json (rust-analyzer extension pointed at ridx)
{
  "rust-analyzer.server.path": "~/.local/bin/ridx-lsp"
}`,
  "Neovim": `-- init.lua
vim.lsp.config("ridx", {
  cmd = { "ridx", "lsp" },
  filetypes = { "rust" },
  root_markers = { "Cargo.toml" },
})
vim.lsp.enable("ridx")`,
  "Helix": `# ~/.config/helix/languages.toml
[language-server.ridx]
command = "ridx"
args = ["lsp"]

[[language]]
name = "rust"
language-servers = ["ridx"]`,
  "Zed": `// ~/.config/zed/settings.json
{
  "lsp": {
    "rust-analyzer": {
      "binary": { "path": "ridx", "arguments": ["lsp"] }
    }
  }
}`,
};

function activate(group, el) {
  group.forEach((b) => b.classList.toggle("active", b === el));
}

function tabs(containerId, keys, onSelect) {
  const box = document.getElementById(containerId);
  const buttons = keys.map((k) => {
    const b = document.createElement("button");
    b.className = "chip";
    b.textContent = k;
    b.addEventListener("click", () => { activate(buttons, b); onSelect(k); });
    box.appendChild(b);
    return b;
  });
  activate(buttons, buttons[0]);
  onSelect(keys[0]);
}

// Logo concept switcher
const logoButtons = [...document.querySelectorAll("[data-logo]")];
logoButtons.forEach((b) => b.addEventListener("click", () => {
  activate(logoButtons, b);
  const src = `assets/logo-${b.dataset.logo}.svg`;
  document.getElementById("hero-logo").src = src;
  document.getElementById("brand-logo").src = src;
}));

// Hero comparison (log scale so 8 µs is still visible against 330 ms)
function showVs(key) {
  const v = VS[key];
  const floor = 1e-6;
  const scale = (x) => (x <= 0 ? 0.5 : Math.max(2, (Math.log10(x / floor) / Math.log10(v.ra / floor)) * 100));
  document.getElementById("vs-ridx").style.width = `${scale(v.ridx)}%`;
  document.getElementById("vs-ra").style.width = "100%";
  document.getElementById("vs-ridx-v").textContent = v.ridxLabel;
  document.getElementById("vs-ra-v").textContent = v.raLabel;
  document.getElementById("vs-note").textContent = v.note;
}
const vsButtons = [...document.querySelectorAll("[data-vs]")];
vsButtons.forEach((b) => b.addEventListener("click", () => { activate(vsButtons, b); showVs(b.dataset.vs); }));
showVs("def");

// Benchmark matrix
tabs("bench-tabs", Object.keys(BENCH), (name) => {
  const r = BENCH[name];
  const cells = [["Go-to-Definition", r.def], ["Find References", r.refs], ["Hover", r.hover]];
  if (r.startup) cells.push(["Startup", r.startup]);
  document.getElementById("bench-metrics").innerHTML =
    `<div class="card metric"><b>${r.crates}</b><span>crates indexed</span></div>` +
    cells.map(([k, v]) => `<div class="card metric"><b>${v}</b><span>${k}</span></div>`).join("");
});

// Architecture
const archButtons = [...document.querySelectorAll("[data-arch]")];
const archDetail = document.getElementById("arch-detail");
archButtons.forEach((b) => b.addEventListener("click", () => { activate(archButtons, b); archDetail.innerHTML = ARCH[b.dataset.arch]; }));
archDetail.innerHTML = ARCH.driver;

// Editors
tabs("editor-tabs", Object.keys(EDITORS), (name) => {
  document.getElementById("editor-code").textContent = EDITORS[name];
});

// Copy to clipboard
document.querySelectorAll("[data-copy]").forEach((b) => b.addEventListener("click", async () => {
  const text = document.getElementById(b.dataset.copy).textContent;
  try {
    await navigator.clipboard.writeText(text);
    b.textContent = "Copied";
  } catch {
    b.textContent = "Copy failed";
  }
  setTimeout(() => { b.textContent = "Copy"; }, 1500);
}));
