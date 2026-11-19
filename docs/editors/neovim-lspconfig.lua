-- rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
-- Copyright (c) 2026 Alexander Panasenko
--
-- Contact: alex@prod.codes
-- Author: https://prod.codes/about/
-- Project: https://github.com/alex09x/rust-intelligence
-- SPDX-License-Identifier: MIT OR Apache-2.0

-- Drop-in replacement configuration for Neovim (nvim-lspconfig / Neovim 0.10+ native LSP).
-- Replace `rust-analyzer` with `ridx` (invoked directly without subcommands over stdio).

local ok, lspconfig = pcall(require, "lspconfig")
if ok then
  lspconfig.rust_analyzer.setup({
    cmd = { "ridx" },
  })
else
  vim.api.nvim_create_autocmd("FileType", {
    pattern = "rust",
    callback = function(args)
      vim.lsp.start({
        name = "ridx",
        cmd = { "ridx" },
        -- Cargo.lock first: a workspace member's own Cargo.toml is not the workspace root.
        root_dir = vim.fs.root(args.buf, { "Cargo.lock" }) or vim.fs.root(args.buf, { "Cargo.toml" }),
      })
    end,
  })
end
