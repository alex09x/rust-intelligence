/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

// Runs `ridx lsp` as the language server of Rust files. Everything an editor shows (completion, hover, diagnostics,
// index progress through `$/progress`) comes from ridx over LSP; the extension only finds or installs the binary.

import * as vscode from "vscode";
import { LanguageClient, LanguageClientOptions, ServerOptions } from "vscode-languageclient/node";
import { findRidx, installRidx, releaseTarget, ridxVersion, TOOLCHAIN } from "./install";

const RUST_ANALYZER = "rust-lang.rust-analyzer";
let client: LanguageClient | undefined;
let output: vscode.OutputChannel;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  output = vscode.window.createOutputChannel("Rust Intelligence");
  context.subscriptions.push(
    output,
    vscode.commands.registerCommand("rustIntelligence.restartServer", () => restart(context)),
    vscode.commands.registerCommand("rustIntelligence.installServer", () => install(context).then(() => restart(context))),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration("rustIntelligence.server.path")) void restart(context);
    }),
  );
  warnAboutRustAnalyzer();
  await start(context);
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}

function configuredPath(): string | undefined {
  const path = vscode.workspace.getConfiguration("rustIntelligence").get<string>("server.path")?.trim();
  return path ? path : undefined;
}

async function start(context: vscode.ExtensionContext): Promise<void> {
  let ridx = findRidx(configuredPath(), context.globalStorageUri.fsPath);
  if (!ridx) {
    ridx = await offerInstall(context);
    if (!ridx) return;
  }
  output.appendLine(`starting ${ridx} lsp (${(await ridxVersion(ridx)) ?? "version unknown"})`);
  const serverOptions: ServerOptions = { command: ridx, args: ["lsp"] };
  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "rust" }],
    outputChannel: output,
  };
  client = new LanguageClient("rustIntelligence", "Rust Intelligence", serverOptions, clientOptions);
  await client.start();
}

async function restart(context: vscode.ExtensionContext): Promise<void> {
  const old = client;
  client = undefined;
  await old?.stop();
  await start(context);
}

/** Asks before installing anything; returns the installed `ridx`, or undefined. */
async function offerInstall(context: vscode.ExtensionContext): Promise<string | undefined> {
  const configured = configuredPath();
  if (configured) {
    void vscode.window.showErrorMessage(`Rust Intelligence: rustIntelligence.server.path is set to ${configured}, which does not exist.`);
    return undefined;
  }
  const { target, why } = releaseTarget();
  if (!target) {
    void vscode.window.showErrorMessage(`Rust Intelligence: ridx is not installed, and ${why}. Run \`cargo install ridx ridx-driver --locked\`, then restart the server.`);
    return undefined;
  }
  const choice = await vscode.window.showInformationMessage(
    `Rust Intelligence needs ridx. Install the latest release from GitHub (checked against its SHA-256) and Rust ${TOOLCHAIN} for ridx-driver through rustup?`,
    "Install",
    "Set path…",
  );
  if (choice === "Set path…") {
    await vscode.commands.executeCommand("workbench.action.openSettings", "rustIntelligence.server.path");
    return undefined;
  }
  return choice === "Install" ? install(context) : undefined;
}

async function install(context: vscode.ExtensionContext): Promise<string | undefined> {
  return vscode.window.withProgress(
    { location: vscode.ProgressLocation.Notification, title: "Rust Intelligence" },
    async (progress) => {
      try {
        const ridx = await installRidx(context.globalStorageUri.fsPath, (step) => {
          output.appendLine(step);
          progress.report({ message: step });
        });
        output.appendLine(`installed ${ridx}`);
        return ridx;
      } catch (e) {
        output.appendLine(`install failed: ${(e as Error).message}`);
        void vscode.window.showErrorMessage(`Rust Intelligence: install failed: ${(e as Error).message}`);
        return undefined;
      }
    },
  );
}

/** With rust-analyzer's extension also on, both servers answer every Rust file: completions and diagnostics twice. */
function warnAboutRustAnalyzer(): void {
  if (!vscode.extensions.getExtension(RUST_ANALYZER)) return;
  void vscode.window
    .showWarningMessage(
      "The rust-analyzer extension is enabled too, so Rust files get two language servers (completions and diagnostics twice). Disable it for this workspace to use ridx alone.",
      "Show rust-analyzer",
    )
    .then((choice) => {
      if (choice) void vscode.commands.executeCommand("workbench.extensions.search", `@id:${RUST_ANALYZER}`);
    });
}
