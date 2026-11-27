<!-- translation-source: a23435e39ef0f3e231c89e3860254dfdddb82f38 -->

> [!NOTE]
> Esta é uma tradução do README em inglês no commit [a23435e](https://github.com/alex09x/rust-intelligence/blob/a23435e39ef0f3e231c89e3860254dfdddb82f38/README.md). A versão canônica e mais atual é o [README em inglês](https://github.com/alex09x/rust-intelligence/blob/main/README.md).

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
  <img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-1b1d20" alt="Platforms: Linux, macOS, Windows">
  <img src="https://img.shields.io/badge/protocol-LSP-1b1d20" alt="Language Server Protocol">
  <a href="https://doi.org/10.5281/zenodo.24379816"><img src="https://zenodo.org/badge/DOI/10.5281/zenodo.24379816.svg" alt="DOI: 10.5281/zenodo.24379816"></a>
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

Linux, macOS e Windows. Sem afiliação com o Rust Project nem com a Rust Foundation.

---

## Por quê

- **O rust-analyzer mantém na memória sua própria análise de todo o workspace**: 1.5 GB no serde e
  11.3 GB no Bevy na medição abaixo. Ele repete a resolução de nomes, a expansão de macros e a inferência de
  tipos que o rustc já faz a cada `cargo check`.
- **A maior parte do que é analisado são dependências e std**, código que ninguém edita. O `ridx` indexa cada
  dependência uma única vez e a compartilha entre todos os projetos e worktrees da máquina.
- **O que você digita continua funcionando**: uma camada em memória sobre o índice responde na hora às
  edições não salvas, e 250 ms após a última edição os crates editados são compilados a partir dos buffers em
  um índice da sessão, então referências e renomeação também enxergam o código novo. Os erros do compilador nele também são informados.

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

Linux (x86_64, arm64), macOS (Apple Silicon, Intel) e Windows (x86_64), com
[rustup](https://rustup.rs):

```bash
curl -fsSL https://rust-intelligence.com/install.sh | sh
```

No Windows, no PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://rust-intelligence.com/install.ps1 | iex"
```

O instalador adiciona o Rust 1.98.1, a toolchain dentro da qual o `ridx-driver` roda (ele se liga às
bibliotecas desse compilador; seus projetos mantêm qualquer toolchain e o ridx constrói o índice com a certa),
baixa do GitHub o arquivo da versão para a sua plataforma, confere o SHA-256 e coloca `ridx` e `ridx-driver`
em `~/.local/bin` (no Windows, `%USERPROFILE%\.local\bin`, que ele adiciona ao seu PATH; `RIDX_INSTALL_DIR`
muda o diretório, `RIDX_VERSION=v0.3.14` fixa uma versão).

Ou pelo [crates.io](https://crates.io/crates/ridx). Compile os dois binários com o Rust 1.98.1: o `ridx-driver` se
liga às bibliotecas desse compilador, por isso precisa do `rustc-dev` e destas duas opções, e o `ridx` constrói o
índice com a toolchain com que foi compilado. A partir de um clone do repositório, use `--path crates/ridx` e
`--path crates/ridx-driver` no lugar dos nomes dos crates.

```bash
rustup toolchain install 1.98.1 --profile minimal --component rustc-dev,rust-src
cargo +1.98.1 install ridx
RUSTC_BOOTSTRAP=1 RUSTFLAGS="-C prefer-dynamic" cargo +1.98.1 install ridx-driver
```

Depois abra um workspace do Cargo no editor. Sem índice ainda, o `ridx` constrói um no primeiro uso e mostra o
progresso no editor; recarrega o índice quando a compilação termina e reindexa em segundo plano depois de cada
salvamento. `ridx index .` constrói o índice pelo terminal (`--all-features`, `--target <triple>` escolhem o cfg).

Se um editor de desktop não encontrar o `ridx`, informe o caminho completo dado por `which ridx`.

## Editores

O `ridx` sem subcomando se comporta como o `rust-analyzer` na linha de comando (`--version`, `--stdio`,
`--log-file`, `--print-config-schema`), então um editor configurado para o rust-analyzer pode iniciá-lo no lugar.

- **VS Code**: a extensão [Rust Intelligence](https://marketplace.visualstudio.com/items?itemName=alex09x.rust-intelligence)
  ([`editors/vscode`](../editors/vscode)) executa `ridx lsp` e oferece instalar o `ridx` quando não o encontra.
  Ou aponte a extensão rust-analyzer para o `ridx`:
  [`docs/editors/vscode-settings.json`](../docs/editors/vscode-settings.json) (`"rust-analyzer.server.path": "ridx"`).
- **Zed**: a extensão [`editors/zed`](../editors/zed) registra `ridx lsp` como o servidor de linguagem
  `rust-intelligence` e baixa o `ridx` quando ele não está no PATH (paleta de comandos: `zed: install dev
  extension`); [`docs/editors/zed-settings.json`](../docs/editors/zed-settings.json) o seleciona para Rust.
- **Neovim**: [`docs/editors/neovim-lspconfig.lua`](../docs/editors/neovim-lspconfig.lua) (`cmd = { "ridx" }`,
  a raiz é procurada primeiro por `Cargo.lock`, então um membro do workspace abre o índice do workspace).
- **Helix**: [`docs/editors/helix-languages.toml`](../docs/editors/helix-languages.toml) declara o servidor de linguagem `ridx` e o seleciona para Rust em `languages.toml`.

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
| Refatoração | `prepareRename`, `rename`, `codeAction` (assistências seguras para a compilação), `executeCommand` |
| Formatação | `formatting`, `rangeFormatting` |
| Símbolos | `documentSymbol`, `workspace/symbol` |
| Diagnósticos | `publishDiagnostics` de um `cargo check` em segundo plano ao salvar e da compilação dos buffers não salvos, diagnósticos pull |
| Workspaces | `didChangeWorkspaceFolders`, `didChangeWatchedFiles`, múltiplas raízes |

## Como funciona

<p align="center"><img src="../site/assets/architecture.svg" width="900" alt="How ridx works: at build time cargo check runs ridx-driver inside rustc and writes .ridx shards to disk; at edit time ridx lsp maps them and answers the editor"></p>

O rustc já resolve cada nome, expande cada macro e infere cada tipo a cada `cargo check`. O `ridx`
guarda essas respostas em vez de calculá-las de novo.

- **O índice é escrito pelo compilador.** `ridx index` executa `cargo check` com `ridx-driver` como
  wrapper do rustc. Depois da análise, o driver percorre o HIR e os resultados da verificação de tipos do
  rustc e grava um shard `.ridx` por crate: definições, cada referência resolvida (chamadas de método e
  campos como o rustc os resolveu, passando por auto-deref e seleção de traits), tipos, impls, campos,
  genéricos, macros e itens `use`. Em seguida a compilação continua normalmente, então a verificação em si
  não muda.
- **Dependências são indexadas uma vez por máquina.** O shard de uma dependência vai para um armazenamento
  endereçado por conteúdo, com chave formada por crate, versão, plataforma-alvo, flags de cfg e os digests
  daquilo com que foi compilada; se já houver ali um shard válido, o driver não extrai o crate de novo. Projetos e
  worktrees compartilham esses shards, e `std`, `core` e `alloc` são indexados uma vez por toolchain.
- **Ler é mapear, não analisar.** O shard v5 é little-endian, com registros de tamanho fixo em seções
  alinhadas a 8 bytes: um pool de strings, arquivos (16 B), definições (40 B) com índice hash FNV-1a, referências
  ordenadas por posição (48 B) com índice hash de destinos, e tabelas de tipos, impls, campos e genéricos.
  Uma busca é uma sondagem de hash, uma busca binária ou uma varredura sobre páginas mapeadas. Páginas que ninguém pede
  nunca são lidas, e o kernel pode descartar as outras quando falta memória.
- **As recompilações são incrementais e seguras.** Depois de salvar, o servidor reindexa em segundo plano;
  quando só fontes do workspace mudaram, ele chama o driver diretamente nos crates alterados, sem Cargo. Um
  crate com erros de compilação é indexado a partir de tudo o que foi resolvido; um que o compilador não
  consegue levar até lá (um erro de sintaxe) mantém seu último shard válido. Outro conjunto de features ou
  outro target ganha sua própria variante ao lado da primeira.
- **O código digitado entre builds** é mapeado linha a linha sobre o texto compilado, e o código editado é
  analisado por um pequeno front end sem compilador ([`crates/ridx-syntax`](../crates/ridx-syntax): lexer,
  parser, resolução de nomes, inferência de tipos) que lê todos os outros tipos e itens do índice. 250 ms após
  a última edição, os crates dos buffers editados são compilados a partir do texto deles em um índice da
  sessão (nada é gravado nos arquivos), que responde referências, renomeação e navegação até o próximo build.

O design completo, com o layout do shard e onde cada parte fica no código:
[`docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md).

## Limitações conhecidas

- **Código não salvo chega ao índice com atraso.** Os crates editados são compilados a partir dos buffers
  250 ms após a última edição, o que leva de uma fração de segundo a alguns segundos em um crate grande; até
  lá, referências e navegação vêm do índice anterior, ajustado à edição. Crates que dependem de um editado
  veem a mudança depois de salvar.
- **Um só cfg.** Como o `cargo check`, o índice vê um target e um conjunto de features; o código atrás de outros
  `#[cfg]` ou features não é indexado. Escolha-os com `ridx index --all-features` / `--target`.
- **Há binários pré-compilados** para Linux x86_64 e arm64, macOS em Apple Silicon e Intel e Windows x86_64;
  nas demais plataformas, instale pelo crates.io.
- **Biblioteca padrão**: navegação e hover para `std`, `core` e `alloc` funcionam; referências a elas são
  procuradas só no workspace, e elas não podem ser renomeadas.
- `vec![]` conta como referência a `Vec`; o rust-analyzer não conta.

---

## Desenvolvimento

Nenhum arquivo-fonte passa de 400 linhas.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Como citar

Se você usar o ridx em pesquisa ou documentação, cite a versão arquivada no Zenodo:
[10.5281/zenodo.24379816](https://doi.org/10.5281/zenodo.24379816) para o projeto (aponta para a versão mais recente), [10.5281/zenodo.24576729](https://doi.org/10.5281/zenodo.24576729) para exatamente a 0.3.16.
[`CITATION.cff`](../CITATION.cff) contém os metadados usados pelo "Cite this repository" do GitHub.

```bibtex
@software{panasenko_2026_24576729,
  author    = {Panasenko, Alexander},
  title     = {Rust Intelligence (ridx) — a Rust language server answering from a compiler-built on-disk index},
  month     = nov,
  year      = 2026,
  publisher = {Zenodo},
  version   = {0.3.16},
  doi       = {10.5281/zenodo.24576729},
  url       = {https://doi.org/10.5281/zenodo.24576729},
}
```

## Licença

Licenciado sob qualquer uma de:

- Apache License, Version 2.0 ([LICENSE-APACHE](../LICENSE-APACHE) ou http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](../LICENSE-MIT) ou http://opensource.org/licenses/MIT)

à sua escolha.
