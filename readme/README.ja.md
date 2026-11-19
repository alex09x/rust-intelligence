<!-- translation-source: 738fe46bc60249e59fb9064428b6d11d56d32014 -->

> [!NOTE]
> これはコミット [738fe46](https://github.com/alex09x/rust-intelligence/blob/738fe46bc60249e59fb9064428b6d11d56d32014/README.md) 時点の英語版 README の翻訳です。正式かつ最新の版は[英語版 README](https://github.com/alex09x/rust-intelligence/blob/main/README.md)です。

<p align="center">
  <img src="../site/assets/logo-minimal.svg" width="96" height="96" alt="ridx logo">
</p>

<h1 align="center">Rust Intelligence (<code>ridx</code>)</h1>

<p align="center">
  <b>10 分の 1 のメモリで動く Rust コードインテリジェンス。</b><br>
  rustc がすでに解析した結果を使って答える Rust 用の言語サーバー。
</p>

<p align="center">
  <a href="https://rust-intelligence.com"><img src="https://img.shields.io/badge/site-rust--intelligence.com-f74c00" alt="Website"></a>
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-c9ced4" alt="License: MIT OR Apache-2.0">
  <img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS-1b1d20" alt="Platforms: Linux, macOS">
  <img src="https://img.shields.io/badge/LSP-3.18-1b1d20" alt="LSP 3.18">
</p>

<p align="center">
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/README.md">English</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ru.md">Русский</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.zh-CN.md">简体中文</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.es.md">Español</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.pt-BR.md">Português (Brasil)</a> ·
  <b><a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ja.md">日本語</a></b> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.de.md">Deutsch</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.fr.md">Français</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ko.md">한국어</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ar.md">العربية</a>
</p>

`ridx` は `cargo check` の実行中に定義・参照・型を一度だけバイナリインデックスに記録し、そのインデックスを
ディスクから直接読み込みます。エディタは、2 つ目のコンパイラをメモリに常駐させることなく、ジャンプ、ホバー、
補完、参照検索、リネームを利用できます。

Linux と macOS に対応。Rust Project および Rust Foundation とは関係ありません。

---

## なぜ ridx か

- **rust-analyzer はワークスペース全体の独自の解析結果をメモリに保持します**：下の計測では serde で 1.5 GB、
  Bevy で 11.3 GB。名前解決、マクロ展開、型推論を、rustc が `cargo check` のたびに行っているのと同じように繰り返します。
- **解析対象の大半は依存クレートと std**で、誰も編集しないコードです。`ridx` は依存クレートごとに一度だけ
  インデックスを作り、マシン上のすべてのプロジェクトと worktree で共有します。
- **入力中のコードにも対応**：未保存の編集には、プロジェクトが再びビルドされるまで、インデックスの上に重ねた
  メモリ上のオーバーレイが答えます。

## 計測

両方のサーバーに同じエディタセッションを実行：各リポジトリの 40 ファイルを 1 つずつ開き、ドキュメントシンボル、
セマンティックトークン、折りたたみ、インレイヒントを要求し、続いてサンプリングした位置でホバー、定義、型定義、
ハイライト、参照、補完を要求し、1 回編集します。メモリには子プロセスを含みます。

| リポジトリ | ピークメモリ、ridx / rust-analyzer | セッションの CPU 時間 | 起動から準備完了まで |
|---|---:|---:|---:|
| serde | 72 MB / 1.5 GB | 2.34 s / 6.97 s | 0.06 s / 3.27 s |
| tokio | 221 MB / 3.2 GB | 1.90 s / 35.9 s | 0.24 s / 4.77 s |
| rust-analyzer | 434 MB / 4.6 GB | 4.44 s / 66.7 s | 0.64 s / 5.22 s |
| bevy | 917 MB / 11.3 GB | 5.38 s / 170.5 s | 1.26 s / 15.5 s |

- ridx `f7105fcc`、2026-11-14。Linux 6.8、AMD Ryzen 9 7950X、64 GB で 1 回実行（n = 1）。生の結果：
  [`bench/published/resources_f7105fcc.jsonl`](../bench/published/resources_f7105fcc.jsonl)、スクリプト：
  [`bench/measure_resources.py`](../bench/measure_resources.py)。メソッドごとのレイテンシは生の結果とサイトにあります。
- **含まれないもの**：ridx は事前に作ったインデックス（`ridx index`、ridx ドライバー付きの `cargo check`）から
  答えます。大きなワークスペースでは初回に数分かかります。rust-analyzer は起動のたびにワークスペースを解析し、
  その時間は rust-analyzer の数値に含まれています。
- **「準備完了」** は、ridx では `initialize` への応答（その後もキャッシュはバックグラウンドで温まります）、
  rust-analyzer では解析の終了に加えてメッセージのない 0.5 s です。
- **メモリ**：ridx の数値にはディスクからマップされたインデックスのページが含まれ、カーネルはメモリ不足時に
  それを破棄して再読み込みできます。rust-analyzer の数値は自身のヒープです。
- rust-analyzer はリポジトリごとに 26–31 件のリクエストにエラーを返し、ridx は 0 件でした。エラーの種類は
  記録していません。エラーになったリクエストもレイテンシの数値に含まれます。

LSP メソッドごとの rust-analyzer との回答の一致度は [`bench/`](../bench/) のスイートで計測し、
リリースごとに [`CHANGELOG.md`](../CHANGELOG.md) に記録しています。`v0.2.0` の 78 メソッド全体のマトリクスは
[`docs/BENCHMARK_MATRIX.md`](../docs/BENCHMARK_MATRIX.md) と
[`LSP_BENCHMARK_COMPARISON_REPORT.md`](../LSP_BENCHMARK_COMPARISON_REPORT.md) にあります。

---

## インストール

Linux（x86_64、arm64）と Apple Silicon の macOS で、[rustup](https://rustup.rs) が必要です：

```bash
curl -fsSL https://rust-intelligence.com/install.sh | sh
```

インストーラーは Rust 1.98.1（`ridx-driver` が内部で動くツールチェーン。そのコンパイラのライブラリにリンクします。
あなたのプロジェクトは任意のツールチェーンのままで構いません。ridx が正しいツールチェーンでインデックスを作ります）を
追加し、GitHub からプラットフォーム用のリリースアーカイブをダウンロードして SHA-256 を検証し、`ridx` と
`ridx-driver` を `~/.local/bin` に置きます（`RIDX_INSTALL_DIR` で変更、`RIDX_VERSION=v0.3.7` でリリースを固定）。

ソースからビルドする場合：

```bash
rustup toolchain install 1.98.1 --component rustc-dev,llvm-tools-preview,rust-src,rustfmt,clippy
git clone https://github.com/alex09x/rust-intelligence
cd rust-intelligence
RUSTC_BOOTSTRAP=1 cargo +1.98.1 install --path crates/ridx --locked
RUSTC_BOOTSTRAP=1 cargo +1.98.1 install --path crates/ridx-driver --locked
```

あとはエディタで Cargo ワークスペースを開くだけです。インデックスがまだなければ、`ridx` が初回使用時に作成し、
進捗をエディタに表示します。ビルド完了時にインデックスを再読み込みし、保存後はバックグラウンドで再インデックスします。
シェルからは `ridx index .` で作成できます（`--all-features`、`--target <triple>` で cfg を選択）。

デスクトップのエディタが `ridx` を見つけられない場合は、`which ridx` が示すフルパスを指定してください。

## エディタ

サブコマンドなしの `ridx` はコマンドライン上で `rust-analyzer` と同じように振る舞う（`--version`、`--stdio`、
`--log-file`、`--print-config-schema`）ため、rust-analyzer 用に設定したエディタから代わりに起動できます。

- **Zed**：[`editors/zed`](../editors/zed) 拡張が `ridx lsp` を `rust-intelligence` 言語サーバーとして登録し
  （コマンドパレット：`zed: install dev extension`）、
  [`docs/editors/zed-settings.json`](../docs/editors/zed-settings.json) で Rust に選択します。
- **VS Code**：[`docs/editors/vscode-settings.json`](../docs/editors/vscode-settings.json) で
  rust-analyzer 拡張を `ridx` に向けます（`"rust-analyzer.server.path": "ridx"`）。
- **Neovim**：[`docs/editors/neovim-lspconfig.lua`](../docs/editors/neovim-lspconfig.lua)（`cmd = { "ridx" }`、
  ルートはまず `Cargo.lock` で探すので、ワークスペースのメンバーを開いてもワークスペース全体のインデックスが使われます）。
- **Helix**：`languages.toml` に `command = "ridx"`、`args = ["lsp"]` を持つ `[language-server.ridx]` を追加します。

## シェルから

```bash
ridx index .                     # build or refresh the index
ridx def . my_crate::run         # go to definition, by path or file:line:col
ridx refs . src/main.rs:12:15    # references across the workspace and its dependencies
ridx hover . src/main.rs:12:15
ridx type-at . src/main.rs:12:15
ridx symbols . Config
ridx inspect .                   # shards, crates and record counts
ridx compare -c all . my_crate::run   # side by side with rust-analyzer
```

---

## LSP メソッド

| カテゴリ | メソッド |
|---|---|
| ナビゲーション | `definition`, `declaration`, `implementation`, `typeDefinition` |
| 参照 | `references`, `documentHighlight` |
| インテリジェンス | `hover`, `completion` (自動インポート付き), `signatureHelp`, `inlayHint`, `semanticTokens` |
| 階層 | `prepareCallHierarchy`, `incomingCalls`, `outgoingCalls`, `prepareTypeHierarchy`, `supertypes`, `subtypes` |
| リファクタリング | `prepareRename`, `rename`, `codeAction` (コンパイルで検証済みのアシスト), `executeCommand` |
| フォーマット | `formatting`, `rangeFormatting` |
| シンボル | `documentSymbol`, `workspace/symbol` |
| 診断 | 保存時のバックグラウンド `cargo check` による `publishDiagnostics`、プル型診断 |
| ワークスペース | `didChangeWorkspaceFolders`, `didChangeWatchedFiles`, マルチルート |

## 仕組み

```
 editor ──JSON-RPC (stdio)──► ridx lsp
                               ├─ OverlayManager (RAM): open buffers, live edits
                               ├─ WorkspaceIndex (mmap): shards of this workspace
                               └─ flycheck thread: cargo check on save
                                        │
              ┌─────────────────────────┴─────────────────────────┐
   ~/.cache/ridx/v5/objects/                          target/ridx/variants/<id>/
   dependencies, once per source hash and target      the workspace's own crates
   shared by every project                            + a manifest scoped to Cargo.lock
                                        ▲
                         ridx-driver (rustc wrapper inside cargo check)
```

`.ridx` v5 シャードはリトルエンディアンで、8 バイト境界に揃えた固定長レコードからなります：ヘッダー、文字列プール、
ファイルテーブル（1 エントリ 16 B）、FNV-1a ハッシュインデックス付きの定義（40 B）、レシーバーチェーン用の owner
と deref のテーブル、位置順に並びターゲットのハッシュインデックスを持つ参照（48 B）、そして型・impl・フィールド・
ジェネリクスのテーブル。検索はマップされたメモリ上の二分探索で、ヒープへのパースは行わず、シャードファイルは
マップ後すぐに閉じます。

## 既知の制限

- **回答は直近のビルドに基づきます。** その後に入力したコードは開いているバッファから答え、プロジェクトが
  再びビルドされると追いつきます。ビルドに失敗したクレートは最後に成功したインデックスを保持します。
- **cfg は 1 つだけ。** `cargo check` と同じく、インデックスは 1 つのターゲットと 1 つのフィーチャーセットだけを見ます。
  ほかの `#[cfg]` やフィーチャーの下のコードはインデックスされません。`ridx index --all-features` / `--target` で選びます。
- **Windows** はまだサポートしていません。
- **標準ライブラリ**：`std`、`core`、`alloc` へのジャンプとホバーは動作します。それらへの参照はワークスペース内だけを
  検索し、リネームはできません。
- **着信呼び出し** は関数が呼び出されている箇所（`name(` または `name::<…>(`）の呼び出し元を列挙します。rust-analyzer は
  `add_systems` に渡すシステムのように、値として渡された関数も列挙します。
- `vec![]` は `Vec` への参照として数えます。rust-analyzer は数えません。

---

## 開発

すべてのソースファイルは 400 行以内です。

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## ライセンス

次のいずれかのライセンスで提供されます：

- Apache License, Version 2.0 ([LICENSE-APACHE](../LICENSE-APACHE) または http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](../LICENSE-MIT) または http://opensource.org/licenses/MIT)

どちらかを選択できます。
