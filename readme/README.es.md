<!-- translation-source: b2617c197ca30ad6ef4348e88121b8f4f03cafbe -->

> [!NOTE]
> Esta es una traducción del README en inglés en el commit [b2617c1](https://github.com/alex09x/rust-intelligence/blob/b2617c197ca30ad6ef4348e88121b8f4f03cafbe/README.md). La versión canónica y más actualizada es el [README en inglés](https://github.com/alex09x/rust-intelligence/blob/main/README.md).

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
  <img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-1b1d20" alt="Platforms: Linux, macOS, Windows">
  <img src="https://img.shields.io/badge/protocol-LSP-1b1d20" alt="Language Server Protocol">
  <a href="https://doi.org/10.5281/zenodo.24379816"><img src="https://zenodo.org/badge/DOI/10.5281/zenodo.24379816.svg" alt="DOI: 10.5281/zenodo.24379816"></a>
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

Linux, macOS y Windows. Sin afiliación con el Rust Project ni con la Rust Foundation.

---

## Por qué

- **rust-analyzer mantiene en memoria su propio análisis de todo el workspace**: 1.5 GB en serde y
  11.3 GB en Bevy en la medición de abajo. Repite la resolución de nombres, la expansión de macros y la
  inferencia de tipos que rustc ya hace en cada `cargo check`.
- **La mayor parte de lo analizado son dependencias y std**, código que nadie edita. `ridx` indexa cada
  dependencia una sola vez y la comparte entre todos los proyectos y worktrees de la máquina.
- **Lo que escribe sigue funcionando**: una capa en memoria sobre el índice responde de inmediato a las
  ediciones sin guardar, y 250 ms después de la última edición los crates editados se compilan desde los
  búferes en un índice de la sesión, así que las referencias y el renombrado también ven el código nuevo. También se informan los errores del compilador en él.

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

Linux (x86_64, arm64), macOS (Apple Silicon, Intel) y Windows (x86_64), con
[rustup](https://rustup.rs):

```bash
curl -fsSL https://rust-intelligence.com/install.sh | sh
```

En Windows, en PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://rust-intelligence.com/install.ps1 | iex"
```

El instalador añade Rust 1.98.1, la toolchain dentro de la que se ejecuta `ridx-driver` (se enlaza con las
bibliotecas de ese compilador; tus proyectos conservan cualquier toolchain y ridx construye el índice con la
correcta), descarga de GitHub el archivo de la versión para tu plataforma, comprueba su SHA-256 y coloca
`ridx` y `ridx-driver` en `~/.local/bin` (en Windows, `%USERPROFILE%\.local\bin`, que añade a tu PATH;
`RIDX_INSTALL_DIR` lo cambia, `RIDX_VERSION=v0.3.14` fija una versión).

O desde [crates.io](https://crates.io/crates/ridx). Compila los dos binarios con Rust 1.98.1: `ridx-driver` se enlaza
con las bibliotecas de ese compilador, así que necesita `rustc-dev` y estas dos opciones, y `ridx` construye el
índice con la toolchain con la que se compiló. Desde un clon del repositorio, usa `--path crates/ridx` y
`--path crates/ridx-driver` en lugar de los nombres de los crates.

```bash
rustup toolchain install 1.98.1 --profile minimal --component rustc-dev,rust-src
cargo +1.98.1 install ridx
RUSTC_BOOTSTRAP=1 RUSTFLAGS="-C prefer-dynamic" cargo +1.98.1 install ridx-driver
```

Después abre un workspace de Cargo en tu editor. Si todavía no hay índice, `ridx` lo construye en el primer uso
y muestra el progreso en el editor; recarga el índice al terminar la compilación y reindexa en segundo plano
tras cada guardado. `ridx index .` lo construye desde la terminal (`--all-features`, `--target <triple>` eligen
el cfg).

Si un editor de escritorio no encuentra `ridx`, dale la ruta completa que devuelve `which ridx`.

## Editores

`ridx` sin subcomando se comporta como `rust-analyzer` en la línea de comandos (`--version`, `--stdio`,
`--log-file`, `--print-config-schema`), así que un editor configurado para rust-analyzer puede iniciarlo en su lugar.

- **VS Code**: la extensión [Rust Intelligence](https://marketplace.visualstudio.com/items?itemName=alex09x.rust-intelligence)
  ([`editors/vscode`](../editors/vscode)) ejecuta `ridx lsp` y ofrece instalar `ridx` si no lo encuentra.
  O apunta la extensión rust-analyzer a `ridx`:
  [`docs/editors/vscode-settings.json`](../docs/editors/vscode-settings.json) (`"rust-analyzer.server.path": "ridx"`).
- **Zed**: la extensión [`editors/zed`](../editors/zed) registra `ridx lsp` como el servidor de lenguaje
  `rust-intelligence` y descarga `ridx` si no está en el PATH (paleta de comandos: `zed: install dev
  extension`); [`docs/editors/zed-settings.json`](../docs/editors/zed-settings.json) lo selecciona para Rust.
- **Neovim**: [`docs/editors/neovim-lspconfig.lua`](../docs/editors/neovim-lspconfig.lua) (`cmd = { "ridx" }`,
  la raíz se busca primero por `Cargo.lock`, así un miembro del workspace abre el índice del workspace).
- **Helix**: [`docs/editors/helix-languages.toml`](../docs/editors/helix-languages.toml) declara el servidor de lenguaje `ridx` y lo selecciona para Rust en `languages.toml`.

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
| Refactorización | `prepareRename`, `rename`, `codeAction` (asistencias seguras para la compilación), `executeCommand` |
| Formato | `formatting`, `rangeFormatting` |
| Símbolos | `documentSymbol`, `workspace/symbol` |
| Diagnósticos | `publishDiagnostics` de un `cargo check` en segundo plano al guardar y de la compilación de los búferes sin guardar, diagnósticos pull |
| Workspaces | `didChangeWorkspaceFolders`, `didChangeWatchedFiles`, multirraíz |

## Cómo funciona

<p align="center"><img src="../site/assets/architecture.svg" width="900" alt="How ridx works: at build time cargo check runs ridx-driver inside rustc and writes .ridx shards to disk; at edit time ridx lsp maps them and answers the editor"></p>

rustc ya resuelve cada nombre, expande cada macro e infiere cada tipo en cada `cargo check`. `ridx`
guarda esas respuestas en lugar de calcularlas de nuevo.

- **El índice lo escribe el compilador.** `ridx index` ejecuta `cargo check` con `ridx-driver` como
  envoltorio de rustc. Tras el análisis, el driver recorre el HIR y los resultados de la comprobación de
  tipos de rustc y escribe un shard `.ridx` por crate: definiciones, cada referencia resuelta (llamadas a
  métodos y campos tal como los resolvió rustc, a través de auto-deref y la selección de traits), tipos,
  impl, campos, genéricos, macros y elementos `use`. Después la compilación sigue como siempre, así que la
  comprobación en sí no cambia.
- **Las dependencias se indexan una vez por máquina.** El shard de una dependencia va a un almacén
  direccionado por contenido, con una clave formada por crate, versión, plataforma de destino, flags de cfg
  y los resúmenes de aquello contra lo que se compiló; si ahí hay un shard válido, el driver no vuelve a extraer el crate.
  Proyectos y worktrees comparten esos shards, y `std`, `core` y `alloc` se indexan una vez por toolchain.
- **Leer es mapear, no analizar.** El shard v5 es little-endian con registros de tamaño fijo en secciones
  alineadas a 8 bytes: un pool de cadenas, archivos (16 B), definiciones (40 B) con un índice hash FNV-1a, referencias
  ordenadas por posición (48 B) con un índice hash de destinos, y tablas de tipos, impl, campos y genéricos.
  Una búsqueda es un sondeo hash, una búsqueda binaria o un recorrido sobre páginas mapeadas. Las páginas que nadie pide
  nunca se leen, y el kernel puede descartar las demás si falta memoria.
- **Las reconstrucciones son incrementales y seguras.** Tras guardar, el servidor reindexa en segundo plano;
  si solo cambiaron fuentes del workspace, llama al driver directamente sobre los crates cambiados, sin Cargo.
  Un crate con errores de compilación se indexa con todo lo que se resolvió; uno que el compilador no puede
  llevar tan lejos (un error de sintaxis) conserva su último shard válido. Otro conjunto de features u otro
  target obtiene su propia variante junto a la primera.
- **El código escrito entre compilaciones** se asigna línea a línea al texto compilado, y el código editado
  lo analiza un pequeño front end sin compilador ([`crates/ridx-syntax`](../crates/ridx-syntax): lexer,
  parser, resolución de nombres, inferencia de tipos) que lee todos los demás tipos e ítems del índice.
  250 ms después de la última edición, los crates de los búferes editados se compilan desde su texto en un
  índice de la sesión (no se escribe nada en los archivos), que responde referencias, renombrado y navegación
  hasta la siguiente compilación.

El diseño completo, con la estructura del shard y dónde vive cada parte en el código:
[`docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md).

## Limitaciones conocidas

- **El código sin guardar llega al índice con retraso.** Los crates editados se compilan desde los búferes
  250 ms después de la última edición, lo que tarda desde una fracción de segundo hasta unos segundos en un
  crate grande; hasta entonces las referencias y la navegación vienen del índice anterior, ajustado a la
  edición. Los crates que dependen de uno editado ven el cambio al guardar.
- **Un solo cfg.** Como `cargo check`, el índice ve un solo target y un conjunto de features; el código tras otros
  `#[cfg]` o features no se indexa. Se eligen con `ridx index --all-features` / `--target`.
- **Hay binarios precompilados** para Linux x86_64 y arm64, macOS en Apple Silicon e Intel y Windows x86_64;
  en otras plataformas, instala desde crates.io.
- **Biblioteca estándar**: la navegación y el hover hacia `std`, `core` y `alloc` funcionan; las referencias a
  ellas se buscan solo en el workspace y no se pueden renombrar.
- `vec![]` cuenta como referencia a `Vec`; rust-analyzer no lo cuenta.

---

## Desarrollo

Ningún archivo fuente supera las 400 líneas.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Cómo citar

Si usas ridx en investigación o documentación, cita la versión archivada en Zenodo:
[10.5281/zenodo.24379816](https://doi.org/10.5281/zenodo.24379816) para el proyecto (apunta a la última versión), [10.5281/zenodo.24571342](https://doi.org/10.5281/zenodo.24571342) para exactamente 0.3.15.
[`CITATION.cff`](../CITATION.cff) contiene los metadatos que usa "Cite this repository" de GitHub.

```bibtex
@software{panasenko_2026_24571342,
  author    = {Panasenko, Alexander},
  title     = {Rust Intelligence (ridx) — a Rust language server answering from a compiler-built on-disk index},
  month     = nov,
  year      = 2026,
  publisher = {Zenodo},
  version   = {0.3.15},
  doi       = {10.5281/zenodo.24571342},
  url       = {https://doi.org/10.5281/zenodo.24571342},
}
```

## Licencia

Con licencia bajo cualquiera de:

- Apache License, Version 2.0 ([LICENSE-APACHE](../LICENSE-APACHE) o http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](../LICENSE-MIT) o http://opensource.org/licenses/MIT)

a tu elección.
