<!-- translation-source: 738fe46bc60249e59fb9064428b6d11d56d32014 -->

> [!NOTE]
> Esta é uma tradução do README em inglês no commit [738fe46](https://github.com/alex09x/rust-intelligence/blob/738fe46bc60249e59fb9064428b6d11d56d32014/README.md). A versão canônica e mais atual é o [README em inglês](https://github.com/alex09x/rust-intelligence/blob/main/README.md).

<p align="center">
  <img src="../site/assets/logo-minimal.svg" width="96" height="96" alt="ridx logo">
</p>

<h1 align="center">Rust Intelligence (<code>ridx</code>)</h1>

<p align="center">
  <b>Inteligência de código Rust com um décimo da memória.</b><br>
  Um servidor de linguagem para Rust que responde com o que o rustc já calculou.
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
  <b><a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.pt-BR.md">Português (Brasil)</a></b> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ja.md">日本語</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.de.md">Deutsch</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.fr.md">Français</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ko.md">한국어</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ar.md">العربية</a>
</p>

`ridx` registra uma única vez, durante o `cargo check`, definições, referências e tipos em um índice binário,
e lê esse índice direto do disco. Seu editor ganha navegação, hover, autocompletar, referências e renomeação
sem um segundo compilador residente na memória.

Linux e macOS. Sem afiliação com o Rust Project nem com a Rust Foundation.

---

## Por quê

- **O rust-analyzer mantém na memória sua própria análise de todo o workspace**: 1.5 GB no serde e
  11.3 GB no Bevy na medição abaixo. Ele repete a resolução de nomes, a expansão de macros e a inferência de
  tipos que o rustc já faz a cada `cargo check`.
- **A maior parte do que é analisado são dependências e std**, código que ninguém edita. O `ridx` indexa cada
  dependência uma única vez e a compartilha entre todos os projetos e worktrees da máquina.
- **O que você está digitando também funciona**: edições não salvas são respondidas por uma camada em memória
  sobre o índice até o projeto compilar de novo.

## Medições

A mesma sessão de editor contra os dois servidores: 40 arquivos de cada repositório abertos um a um, com
símbolos do documento, tokens semânticos, dobramento e inlay hints; depois hover, definição, definição de tipo,
destaque, referências e autocompletar em posições amostradas, e uma edição. A memória inclui processos filhos.

| Repositório | Pico de memória, ridx / rust-analyzer | CPU da sessão | Da partida até pronto |
|---|---:|---:|---:|
| serde | 72 MB / 1.5 GB | 2.34 s / 6.97 s | 0.06 s / 3.27 s |
| tokio | 221 MB / 3.2 GB | 1.90 s / 35.9 s | 0.24 s / 4.77 s |
| rust-analyzer | 434 MB / 4.6 GB | 4.44 s / 66.7 s | 0.64 s / 5.22 s |
| bevy | 917 MB / 11.3 GB | 5.38 s / 170.5 s | 1.26 s / 15.5 s |

- ridx `f7105fcc`, 2026-11-14; uma execução (n = 1) em Linux 6.8, AMD Ryzen 9 7950X, 64 GB. Resultados brutos:
  [`bench/published/resources_f7105fcc.jsonl`](../bench/published/resources_f7105fcc.jsonl), script:
  [`bench/measure_resources.py`](../bench/measure_resources.py); a latência por método está nos resultados
  brutos e no site.
- **Não contabilizado**: o ridx responde a partir de um índice construído antes (`ridx index`, um `cargo check` com o
  driver do ridx), que na primeira vez leva minutos em um workspace grande. O rust-analyzer analisa o workspace
  toda vez que inicia, e esse tempo está nos números dele.
- **"Pronto"** é a resposta a `initialize` para o ridx (os caches continuam aquecendo em segundo plano depois)
  e, para o rust-analyzer, o fim da sua análise mais 0.5 s sem mensagens.
- **Memória**: o número do ridx inclui páginas do índice mapeadas do disco, que o kernel pode descartar sob
  pressão e ler de novo. O do rust-analyzer é o seu próprio heap.
- O rust-analyzer respondeu com erro a 26–31 requisições por repositório, o ridx a nenhuma; os tipos de erro não
  foram registrados. As requisições com erro estão incluídas nos números de latência.

A concordância das respostas com o rust-analyzer por método LSP é medida com a suíte em [`bench/`](../bench/)
e registrada a cada versão em [`CHANGELOG.md`](../CHANGELOG.md). A matriz completa de 78 métodos da `v0.2.0` está em
[`docs/BENCHMARK_MATRIX.md`](../docs/BENCHMARK_MATRIX.md) e
[`LSP_BENCHMARK_COMPARISON_REPORT.md`](../LSP_BENCHMARK_COMPARISON_REPORT.md).

---

## Instalação

Linux (x86_64, arm64) e macOS em Apple Silicon, com [rustup](https://rustup.rs):

```bash
curl -fsSL https://rust-intelligence.com/install.sh | sh
```

O instalador adiciona o Rust 1.98.1, a toolchain dentro da qual o `ridx-driver` roda (ele se liga às
bibliotecas desse compilador; seus projetos mantêm qualquer toolchain e o ridx constrói o índice com a certa),
baixa do GitHub o arquivo da versão para a sua plataforma, confere o SHA-256 e coloca `ridx` e `ridx-driver`
em `~/.local/bin` (`RIDX_INSTALL_DIR` muda o diretório, `RIDX_VERSION=v0.3.7` fixa uma versão).

Para compilar a partir do código-fonte:

```bash
rustup toolchain install 1.98.1 --component rustc-dev,llvm-tools-preview,rust-src,rustfmt,clippy
git clone https://github.com/alex09x/rust-intelligence
cd rust-intelligence
RUSTC_BOOTSTRAP=1 cargo +1.98.1 install --path crates/ridx --locked
RUSTC_BOOTSTRAP=1 cargo +1.98.1 install --path crates/ridx-driver --locked
```

Depois abra um workspace do Cargo no editor. Sem índice ainda, o `ridx` constrói um no primeiro uso e mostra o
progresso no editor; recarrega o índice quando a compilação termina e reindexa em segundo plano depois de cada
salvamento. `ridx index .` constrói o índice pelo terminal (`--all-features`, `--target <triple>` escolhem o cfg).

Se um editor de desktop não encontrar o `ridx`, informe o caminho completo dado por `which ridx`.

## Editores

O `ridx` sem subcomando se comporta como o `rust-analyzer` na linha de comando (`--version`, `--stdio`,
`--log-file`, `--print-config-schema`), então um editor configurado para o rust-analyzer pode iniciá-lo no lugar.

- **Zed**: a extensão [`editors/zed`](../editors/zed) registra `ridx lsp` como o servidor de linguagem
  `rust-intelligence` (paleta de comandos: `zed: install dev extension`), e
  [`docs/editors/zed-settings.json`](../docs/editors/zed-settings.json) o seleciona para Rust.
- **VS Code**: [`docs/editors/vscode-settings.json`](../docs/editors/vscode-settings.json) aponta a
  extensão rust-analyzer para o `ridx` (`"rust-analyzer.server.path": "ridx"`).
- **Neovim**: [`docs/editors/neovim-lspconfig.lua`](../docs/editors/neovim-lspconfig.lua) (`cmd = { "ridx" }`,
  a raiz é procurada primeiro por `Cargo.lock`, então um membro do workspace abre o índice do workspace).
- **Helix**: um `[language-server.ridx]` com `command = "ridx"`, `args = ["lsp"]` no `languages.toml`.

## Pelo terminal

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

## Métodos LSP

| Categoria | Métodos |
|---|---|
| Navegação | `definition`, `declaration`, `implementation`, `typeDefinition` |
| Referências | `references`, `documentHighlight` |
| Inteligência | `hover`, `completion` (com importação automática), `signatureHelp`, `inlayHint`, `semanticTokens` |
| Hierarquias | `prepareCallHierarchy`, `incomingCalls`, `outgoingCalls`, `prepareTypeHierarchy`, `supertypes`, `subtypes` |
| Refatoração | `prepareRename`, `rename`, `codeAction` (assistências verificadas por compilação), `executeCommand` |
| Formatação | `formatting`, `rangeFormatting` |
| Símbolos | `documentSymbol`, `workspace/symbol` |
| Diagnósticos | `publishDiagnostics` de um `cargo check` em segundo plano ao salvar, diagnósticos pull |
| Workspaces | `didChangeWorkspaceFolders`, `didChangeWatchedFiles`, múltiplas raízes |

## Como funciona

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

O shard `.ridx` v5 é little-endian, com registros de tamanho fixo alinhados a 8 bytes: um cabeçalho, um pool de
strings, a tabela de arquivos (16 B por entrada), definições (40 B) com um índice hash FNV-1a, tabelas de owner e
deref para cadeias de receptores, referências ordenadas por posição (48 B) com um índice hash de alvos, e tabelas
de tipos, impl, campos e genéricos. As buscas são binárias sobre memória mapeada; nada é parseado para o heap e
os arquivos de shard são fechados assim que mapeados.

## Limitações conhecidas

- **As respostas vêm da última compilação.** O código digitado depois é respondido pelo buffer aberto e se atualiza
  quando o projeto compila de novo; um crate que não compila mantém o último índice válido.
- **Um só cfg.** Como o `cargo check`, o índice vê um target e um conjunto de features; o código atrás de outros
  `#[cfg]` ou features não é indexado. Escolha-os com `ridx index --all-features` / `--target`.
- **Windows** ainda não é suportado.
- **Biblioteca padrão**: navegação e hover para `std`, `core` e `alloc` funcionam; referências a elas são
  procuradas só no workspace, e elas não podem ser renomeadas.
- **Chamadas recebidas** lista quem chama onde a função é invocada (`name(` ou `name::<…>(`); o rust-analyzer
  também lista funções passadas como valores, como um sistema dado a `add_systems`.
- `vec![]` conta como referência a `Vec`; o rust-analyzer não conta.

---

## Desenvolvimento

Nenhum arquivo-fonte passa de 400 linhas.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Licença

Licenciado sob qualquer uma de:

- Apache License, Version 2.0 ([LICENSE-APACHE](../LICENSE-APACHE) ou http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](../LICENSE-MIT) ou http://opensource.org/licenses/MIT)

à sua escolha.
