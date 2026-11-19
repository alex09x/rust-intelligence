<!-- translation-source: 738fe46bc60249e59fb9064428b6d11d56d32014 -->

> [!NOTE]
> Esta es una traducción del README en inglés en el commit [738fe46](https://github.com/alex09x/rust-intelligence/blob/738fe46bc60249e59fb9064428b6d11d56d32014/README.md). La versión canónica y más actualizada es el [README en inglés](https://github.com/alex09x/rust-intelligence/blob/main/README.md).

<p align="center">
  <img src="../site/assets/logo-minimal.svg" width="96" height="96" alt="ridx logo">
</p>

<h1 align="center">Rust Intelligence (<code>ridx</code>)</h1>

<p align="center">
  <b>Inteligencia de código Rust con una décima parte de la memoria.</b><br>
  Un servidor de lenguaje para Rust que responde con lo que rustc ya calculó.
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
  <b><a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.es.md">Español</a></b> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.pt-BR.md">Português (Brasil)</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ja.md">日本語</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.de.md">Deutsch</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.fr.md">Français</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ko.md">한국어</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ar.md">العربية</a>
</p>

`ridx` registra una sola vez, durante `cargo check`, las definiciones, referencias y tipos en un índice binario,
y lee ese índice directamente del disco. Tu editor obtiene navegación, hover, autocompletado, referencias y
renombrado sin un segundo compilador residente en memoria.

Linux y macOS. Sin afiliación con el Rust Project ni con la Rust Foundation.

---

## Por qué

- **rust-analyzer mantiene en memoria su propio análisis de todo el workspace**: 1.5 GB en serde y
  11.3 GB en Bevy en la medición de abajo. Repite la resolución de nombres, la expansión de macros y la
  inferencia de tipos que rustc ya hace en cada `cargo check`.
- **La mayor parte de lo analizado son dependencias y std**, código que nadie edita. `ridx` indexa cada
  dependencia una sola vez y la comparte entre todos los proyectos y worktrees de la máquina.
- **Lo que estás escribiendo también funciona**: las ediciones sin guardar se responden desde una capa en
  memoria sobre el índice hasta que el proyecto vuelve a compilar.

## Mediciones

La misma sesión de editor contra ambos servidores: se abren uno a uno 40 archivos de cada repositorio, con
símbolos del documento, tokens semánticos, plegado e inlay hints; luego hover, definición, definición de tipo,
resaltado, referencias y autocompletado en posiciones muestreadas, y una edición. La memoria incluye los
procesos hijos.

| Repositorio | Memoria máxima, ridx / rust-analyzer | CPU de la sesión | Del arranque a listo |
|---|---:|---:|---:|
| serde | 72 MB / 1.5 GB | 2.34 s / 6.97 s | 0.06 s / 3.27 s |
| tokio | 221 MB / 3.2 GB | 1.90 s / 35.9 s | 0.24 s / 4.77 s |
| rust-analyzer | 434 MB / 4.6 GB | 4.44 s / 66.7 s | 0.64 s / 5.22 s |
| bevy | 917 MB / 11.3 GB | 5.38 s / 170.5 s | 1.26 s / 15.5 s |

- ridx `f7105fcc`, 2026-11-14; una ejecución (n = 1) en Linux 6.8, AMD Ryzen 9 7950X, 64 GB. Resultados en bruto:
  [`bench/published/resources_f7105fcc.jsonl`](../bench/published/resources_f7105fcc.jsonl), script:
  [`bench/measure_resources.py`](../bench/measure_resources.py); la latencia por método está en los resultados
  en bruto y en el sitio.
- **No contabilizado**: ridx responde desde un índice construido de antemano (`ridx index`, un `cargo check` con el
  driver de ridx), que la primera vez tarda minutos en un workspace grande. rust-analyzer analiza el workspace
  cada vez que arranca, y ese tiempo está en sus cifras.
- **«Listo»** es la respuesta a `initialize` para ridx (las cachés siguen calentándose en segundo plano después)
  y, para rust-analyzer, el fin de su análisis más 0.5 s sin mensajes.
- **Memoria**: la cifra de ridx incluye páginas del índice mapeadas desde disco, que el kernel puede descartar
  bajo presión y volver a leer. La de rust-analyzer es su propio heap.
- rust-analyzer respondió con error a 26–31 peticiones por repositorio, ridx a ninguna; no se registraron los
  tipos de error. Las peticiones con error están incluidas en las cifras de latencia.

La coincidencia de las respuestas con rust-analyzer por método LSP se mide con la batería de [`bench/`](../bench/)
y se registra en cada versión en [`CHANGELOG.md`](../CHANGELOG.md). La matriz completa de 78 métodos de `v0.2.0` está en
[`docs/BENCHMARK_MATRIX.md`](../docs/BENCHMARK_MATRIX.md) y
[`LSP_BENCHMARK_COMPARISON_REPORT.md`](../LSP_BENCHMARK_COMPARISON_REPORT.md).

---

## Instalación

Linux (x86_64, arm64) y macOS en Apple Silicon, con [rustup](https://rustup.rs):

```bash
curl -fsSL https://rust-intelligence.com/install.sh | sh
```

El instalador añade Rust 1.98.1, la toolchain dentro de la que se ejecuta `ridx-driver` (se enlaza con las
bibliotecas de ese compilador; tus proyectos conservan cualquier toolchain y ridx construye el índice con la
correcta), descarga de GitHub el archivo de la versión para tu plataforma, comprueba su SHA-256 y coloca
`ridx` y `ridx-driver` en `~/.local/bin` (`RIDX_INSTALL_DIR` lo cambia, `RIDX_VERSION=v0.3.7` fija una versión).

Para compilar desde el código fuente:

```bash
rustup toolchain install 1.98.1 --component rustc-dev,llvm-tools-preview,rust-src,rustfmt,clippy
git clone https://github.com/alex09x/rust-intelligence
cd rust-intelligence
RUSTC_BOOTSTRAP=1 cargo +1.98.1 install --path crates/ridx --locked
RUSTC_BOOTSTRAP=1 cargo +1.98.1 install --path crates/ridx-driver --locked
```

Después abre un workspace de Cargo en tu editor. Si todavía no hay índice, `ridx` lo construye en el primer uso
y muestra el progreso en el editor; recarga el índice al terminar la compilación y reindexa en segundo plano
tras cada guardado. `ridx index .` lo construye desde la terminal (`--all-features`, `--target <triple>` eligen
el cfg).

Si un editor de escritorio no encuentra `ridx`, dale la ruta completa que devuelve `which ridx`.

## Editores

`ridx` sin subcomando se comporta como `rust-analyzer` en la línea de comandos (`--version`, `--stdio`,
`--log-file`, `--print-config-schema`), así que un editor configurado para rust-analyzer puede iniciarlo en su lugar.

- **Zed**: la extensión [`editors/zed`](../editors/zed) registra `ridx lsp` como el servidor de lenguaje
  `rust-intelligence` (paleta de comandos: `zed: install dev extension`), y
  [`docs/editors/zed-settings.json`](../docs/editors/zed-settings.json) lo selecciona para Rust.
- **VS Code**: [`docs/editors/vscode-settings.json`](../docs/editors/vscode-settings.json) apunta la
  extensión rust-analyzer a `ridx` (`"rust-analyzer.server.path": "ridx"`).
- **Neovim**: [`docs/editors/neovim-lspconfig.lua`](../docs/editors/neovim-lspconfig.lua) (`cmd = { "ridx" }`,
  la raíz se busca primero por `Cargo.lock`, así un miembro del workspace abre el índice del workspace).
- **Helix**: un `[language-server.ridx]` con `command = "ridx"`, `args = ["lsp"]` en `languages.toml`.

## Desde la terminal

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

| Categoría | Métodos |
|---|---|
| Navegación | `definition`, `declaration`, `implementation`, `typeDefinition` |
| Referencias | `references`, `documentHighlight` |
| Inteligencia | `hover`, `completion` (con importación automática), `signatureHelp`, `inlayHint`, `semanticTokens` |
| Jerarquías | `prepareCallHierarchy`, `incomingCalls`, `outgoingCalls`, `prepareTypeHierarchy`, `supertypes`, `subtypes` |
| Refactorización | `prepareRename`, `rename`, `codeAction` (asistencias verificadas por compilación), `executeCommand` |
| Formato | `formatting`, `rangeFormatting` |
| Símbolos | `documentSymbol`, `workspace/symbol` |
| Diagnósticos | `publishDiagnostics` de un `cargo check` en segundo plano al guardar, diagnósticos pull |
| Workspaces | `didChangeWorkspaceFolders`, `didChangeWatchedFiles`, multirraíz |

## Cómo funciona

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

El shard `.ridx` v5 es little-endian, con registros de tamaño fijo alineados a 8 bytes: una cabecera, un pool de
cadenas, la tabla de archivos (16 B por entrada), definiciones (40 B) con un índice hash FNV-1a, tablas de owner y
deref para cadenas de receptores, referencias ordenadas por posición (48 B) con un índice hash de destinos, y
tablas de tipos, impl, campos y genéricos. Las búsquedas son binarias sobre memoria mapeada; nada se parsea al
heap y los archivos de shard se cierran una vez mapeados.

## Limitaciones conocidas

- **Las respuestas vienen de la última compilación.** El código escrito después se responde desde el buffer
  abierto y se pone al día cuando el proyecto vuelve a compilar; un crate que no compila conserva su último índice válido.
- **Un solo cfg.** Como `cargo check`, el índice ve un solo target y un conjunto de features; el código tras otros
  `#[cfg]` o features no se indexa. Se eligen con `ridx index --all-features` / `--target`.
- **Windows** aún no está soportado.
- **Biblioteca estándar**: la navegación y el hover hacia `std`, `core` y `alloc` funcionan; las referencias a
  ellas se buscan solo en el workspace y no se pueden renombrar.
- **Llamadas entrantes** lista a quien llama donde la función se invoca (`name(` o `name::<…>(`); rust-analyzer
  también lista funciones pasadas como valores, como un sistema dado a `add_systems`.
- `vec![]` cuenta como referencia a `Vec`; rust-analyzer no lo cuenta.

---

## Desarrollo

Ningún archivo fuente supera las 400 líneas.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Licencia

Con licencia bajo cualquiera de:

- Apache License, Version 2.0 ([LICENSE-APACHE](../LICENSE-APACHE) o http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](../LICENSE-MIT) o http://opensource.org/licenses/MIT)

a tu elección.
