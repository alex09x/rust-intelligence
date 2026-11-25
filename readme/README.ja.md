<!-- translation-source: 3fa2b8b43e14f2a477be9a5de758418064665c67 -->

> [!NOTE]
> これはコミット [3fa2b8b](https://github.com/alex09x/rust-intelligence/blob/3fa2b8b43e14f2a477be9a5de758418064665c67/README.md) 時点の英語版 README の翻訳です。正式かつ最新の版は[英語版 README](https://github.com/alex09x/rust-intelligence/blob/main/README.md)です。

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
  <img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-1b1d20" alt="Platforms: Linux, macOS, Windows">
  <img src="https://img.shields.io/badge/protocol-LSP-1b1d20" alt="Language Server Protocol">
  <a href="https://doi.org/10.5281/zenodo.24379816"><img src="https://zenodo.org/badge/DOI/10.5281/zenodo.24379816.svg" alt="DOI: 10.5281/zenodo.24379816"></a>
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

Linux、macOS、Windows に対応。Rust Project および Rust Foundation とは関係ありません。

---

## なぜ ridx か

- **rust-analyzer はワークスペース全体の独自の解析結果をメモリに保持します**：下の計測では serde で 1.5 GB、
  Bevy で 11.3 GB。名前解決、マクロ展開、型推論を、rustc が `cargo check` のたびに行っているのと同じように繰り返します。
- **解析対象の大半は依存クレートと std**で、誰も編集しないコードです。`ridx` は依存クレートごとに一度だけ
  インデックスを作り、マシン上のすべてのプロジェクトと worktree で共有します。
- **入力中のコードにも対応**：未保存の編集には、インデックスの上に重ねたメモリ上のオーバーレイがすぐに答えます。最後の
  編集から 250 ms 後には編集されたクレートがバッファからセッションのインデックスへコンパイルされるので、参照や
  リネームも新しいコードを見ます。その中のコンパイラのエラーも報告されます。

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

Linux（x86_64、arm64）、macOS（Apple Silicon、Intel）、Windows（x86_64）で、
[rustup](https://rustup.rs) が必要です：

```bash
curl -fsSL https://rust-intelligence.com/install.sh | sh
```

Windows では PowerShell で：

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://rust-intelligence.com/install.ps1 | iex"
```

インストーラーは Rust 1.98.1（`ridx-driver` が内部で動くツールチェーン。そのコンパイラのライブラリにリンクします。
あなたのプロジェクトは任意のツールチェーンのままで構いません。ridx が正しいツールチェーンでインデックスを作ります）を
追加し、GitHub からプラットフォーム用のリリースアーカイブをダウンロードして SHA-256 を検証し、`ridx` と
`ridx-driver` を `~/.local/bin` に置きます（Windows では `%USERPROFILE%\.local\bin` に置き、PATH に追加します。
`RIDX_INSTALL_DIR` で変更、`RIDX_VERSION=v0.3.14` でリリースを固定）。

または [crates.io](https://crates.io/crates/ridx) から。2 つのバイナリはどちらも Rust 1.98.1 でビルドします。`ridx-driver` は
そのコンパイラ自身のライブラリにリンクするため `rustc-dev` と次の 2 つのフラグが必要で、`ridx` は自分をビルドした
ツールチェーンでインデックスを作ります。リポジトリのチェックアウトからは、クレート名の代わりに `--path crates/ridx`
と `--path crates/ridx-driver` を使います。

```bash
rustup toolchain install 1.98.1 --profile minimal --component rustc-dev,rust-src
cargo +1.98.1 install ridx
RUSTC_BOOTSTRAP=1 RUSTFLAGS="-C prefer-dynamic" cargo +1.98.1 install ridx-driver
```

あとはエディタで Cargo ワークスペースを開くだけです。インデックスがまだなければ、`ridx` が初回使用時に作成し、
進捗をエディタに表示します。ビルド完了時にインデックスを再読み込みし、保存後はバックグラウンドで再インデックスします。
シェルからは `ridx index .` で作成できます（`--all-features`、`--target <triple>` で cfg を選択）。

デスクトップのエディタが `ridx` を見つけられない場合は、`which ridx` が示すフルパスを指定してください。

## エディタ

サブコマンドなしの `ridx` はコマンドライン上で `rust-analyzer` と同じように振る舞う（`--version`、`--stdio`、
`--log-file`、`--print-config-schema`）ため、rust-analyzer 用に設定したエディタから代わりに起動できます。

- **VS Code**：[Rust Intelligence](https://marketplace.visualstudio.com/items?itemName=alex09x.rust-intelligence) 拡張
  （[`editors/vscode`](../editors/vscode)）が `ridx lsp` を起動し、`ridx` が見つからなければインストールを提案します。
  rust-analyzer 拡張を `ridx` に向けることもできます：
  [`docs/editors/vscode-settings.json`](../docs/editors/vscode-settings.json)（`"rust-analyzer.server.path": "ridx"`）。
- **Zed**：[`editors/zed`](../editors/zed) 拡張が `ridx lsp` を `rust-intelligence` 言語サーバーとして登録し、
  PATH に `ridx` がなければダウンロードします（コマンドパレット：`zed: install dev extension`）。
  [`docs/editors/zed-settings.json`](../docs/editors/zed-settings.json) で Rust に選択します。
- **Neovim**：[`docs/editors/neovim-lspconfig.lua`](../docs/editors/neovim-lspconfig.lua)（`cmd = { "ridx" }`、
  ルートはまず `Cargo.lock` で探すので、ワークスペースのメンバーを開いてもワークスペース全体のインデックスが使われます）。
- **Helix**: [`docs/editors/helix-languages.toml`](../docs/editors/helix-languages.toml) が言語サーバー `ridx` を宣言し、`languages.toml` で Rust 用に選択します。

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
| リファクタリング | `prepareRename`, `rename`, `codeAction` (コンパイルを壊さないアシスト), `executeCommand` |
| フォーマット | `formatting`, `rangeFormatting` |
| シンボル | `documentSymbol`, `workspace/symbol` |
| 診断 | 保存時のバックグラウンド `cargo check` と未保存バッファのコンパイルによる `publishDiagnostics`、プル型診断 |
| ワークスペース | `didChangeWorkspaceFolders`, `didChangeWatchedFiles`, マルチルート |

## 仕組み

<p align="center"><img src="../site/assets/architecture.svg" width="900" alt="How ridx works: at build time cargo check runs ridx-driver inside rustc and writes .ridx shards to disk; at edit time ridx lsp maps them and answers the editor"></p>

rustc は `cargo check` のたびに、すべての名前を解決し、すべてのマクロを展開し、すべての型を推論しています。`ridx`
はその答えを計算し直さずに保存します。

- **インデックスはコンパイラが書きます。** `ridx index` は `ridx-driver` を rustc ラッパーとして `cargo check`
  を実行します。解析の後、ドライバーは HIR と rustc の型検査結果をたどり、クレートごとに 1 つの `.ridx`
  シャードを書き出します。定義、解決済みのすべての参照(メソッド呼び出しとフィールドは自動参照外しと
  トレイト選択を経て rustc が解決したとおり)、型、impl、フィールド、ジェネリクス、マクロ、`use`
  項目です。その後コンパイルは通常どおり続くので、チェック自体は変わりません。
- **依存クレートはマシンごとに一度だけインデックスされます。** 依存のシャードは内容アドレス方式のストアに
  置かれ、キーはクレート、バージョン、ターゲット、cfg フラグ、ビルド時に参照した成果物のダイジェストから
  作られます。有効なシャードがあればドライバーはそのクレートを再抽出しません。プロジェクトや worktree は
  これらのシャードを共有し、`std`、`core`、`alloc` はツールチェーンごとに一度だけインデックスされます。
- **読み込みはパースではなくマッピングです。** v5 シャードはリトルエンディアンで、8 バイト境界に揃えた
  セクションに固定長レコードが並びます。文字列プール、ファイル(16 B)、FNV-1a ハッシュインデックス付きの定義
  (40 B)、位置順に並びターゲットのハッシュインデックスを持つ参照(48 B)、そして型、impl、フィールド、
  ジェネリクスのテーブルです。検索は、マップされたページ上でのハッシュの探索、二分探索、または走査です。誰も要求しない
  ページは一度も読まれず、それ以外のページもメモリが逼迫すればカーネルが破棄できます。
- **再ビルドはインクリメンタルで安全です。** 保存後、サーバーはバックグラウンドで再インデックスします。ワークスペースの
  ソースだけが変わった場合は、Cargo を介さず、変更されたクレートに対して直接ドライバを呼び出します。コンパイル
  エラーのあるクレートは解決できたものすべてからインデックスされ、コンパイラがそこまで進められないクレート
  （構文エラー）は最後に成功したシャードを保持します。別の feature の組み合わせやターゲットは、最初のものの隣に
  独自のバリアントを持ちます。
- **ビルドの合間に入力したコード**はコンパイル済みのテキストに行単位で対応付けられ、編集されたコードはコンパイラを
  使わない小さなフロントエンド（[`crates/ridx-syntax`](../crates/ridx-syntax)：レキサー、パーサー、名前解決、型推論）
  が解析し、それ以外の型と項目はすべてインデックスから読みます。最後の編集から 250 ms 後、編集されたバッファを
  持つクレートがそのテキストからセッションのインデックスへコンパイルされ（ファイルには何も書き込みません）、次の
  ビルドまで参照、リネーム、ナビゲーションに答えます。

シャードのレイアウトや各部分がコードのどこにあるかを含む全体の設計:
[`docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md)。

## 既知の制限

- **未保存のコードは少し遅れてインデックスに入ります。** 編集されたクレートは最後の編集から 250 ms 後にバッファから
  コンパイルされ、大きなクレートでは 1 秒未満から数秒かかります。それまでは、参照とナビゲーションは前のインデックスを
  編集に合わせて対応付けたものから返ります。編集されたクレートに依存するクレートは、保存後に変更を見ます。
- **cfg は 1 つだけ。** `cargo check` と同じく、インデックスは 1 つのターゲットと 1 つのフィーチャーセットだけを見ます。
  ほかの `#[cfg]` やフィーチャーの下のコードはインデックスされません。`ridx index --all-features` / `--target` で選びます。
- **ビルド済みバイナリ**は Linux x86_64 と arm64、Apple Silicon と Intel の macOS、Windows x86_64 向けです。
  それ以外のプラットフォームでは crates.io からインストールしてください。
- **標準ライブラリ**：`std`、`core`、`alloc` へのジャンプとホバーは動作します。それらへの参照はワークスペース内だけを
  検索し、リネームはできません。
- `vec![]` は `Vec` への参照として数えます。rust-analyzer は数えません。

---

## 開発

すべてのソースファイルは 400 行以内です。

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## 引用

研究やドキュメントで ridx を使う場合は、Zenodo にアーカイブされたリリースを引用してください：
プロジェクト全体には [10.5281/zenodo.24379816](https://doi.org/10.5281/zenodo.24379816)（最新版に解決されます）、0.3.13 そのものには [10.5281/zenodo.24487370](https://doi.org/10.5281/zenodo.24487370)。
[`CITATION.cff`](../CITATION.cff) には GitHub の「Cite this repository」が使うメタデータがあります。

```bibtex
@software{panasenko_2026_24487370,
  author    = {Panasenko, Alexander},
  title     = {Rust Intelligence (ridx) — a Rust language server answering from a compiler-built on-disk index},
  month     = nov,
  year      = 2026,
  publisher = {Zenodo},
  version   = {0.3.13},
  doi       = {10.5281/zenodo.24487370},
  url       = {https://doi.org/10.5281/zenodo.24487370},
}
```

## ライセンス

次のいずれかのライセンスで提供されます：

- Apache License, Version 2.0 ([LICENSE-APACHE](../LICENSE-APACHE) または http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](../LICENSE-MIT) または http://opensource.org/licenses/MIT)

どちらかを選択できます。
