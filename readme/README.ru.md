<!-- translation-source: ccd5c36fdcf2f9210dffb784900861d2efe035ba -->

> [!NOTE]
> Это перевод англоязычного README на коммите [ccd5c36](https://github.com/alex09x/rust-intelligence/blob/ccd5c36fdcf2f9210dffb784900861d2efe035ba/README.md). Каноническая и самая актуальная версия — [README на английском](https://github.com/alex09x/rust-intelligence/blob/main/README.md).

<p align="center">
  <img src="../site/assets/logo-minimal.svg" width="96" height="96" alt="ridx logo">
</p>

<h1 align="center">Rust Intelligence (<code>ridx</code>)</h1>

<p align="center">
  <b>Анализ Rust-кода за десятую часть памяти.</b><br>
  Языковой сервер для Rust, который отвечает тем, что rustc уже вычислил.
</p>

<p align="center">
  <a href="https://rust-intelligence.com"><img src="https://img.shields.io/badge/site-rust--intelligence.com-f74c00" alt="Website"></a>
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-c9ced4" alt="License: MIT OR Apache-2.0">
  <img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS-1b1d20" alt="Platforms: Linux, macOS">
  <img src="https://img.shields.io/badge/protocol-LSP-1b1d20" alt="Language Server Protocol">
  <a href="https://doi.org/10.5281/zenodo.24379816"><img src="https://zenodo.org/badge/DOI/10.5281/zenodo.24379816.svg" alt="DOI: 10.5281/zenodo.24379816"></a>
</p>

<p align="center">
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/README.md">English</a> ·
  <b><a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ru.md">Русский</a></b> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.zh-CN.md">简体中文</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.es.md">Español</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.pt-BR.md">Português (Brasil)</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ja.md">日本語</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.de.md">Deutsch</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.fr.md">Français</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ko.md">한국어</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ar.md">العربية</a>
</p>

`ridx` один раз, во время `cargo check`, записывает определения, ссылки и типы в бинарный индекс и читает
этот индекс прямо с диска. Редактор получает навигацию, подсказки при наведении, автодополнение, поиск ссылок
и переименование без второго компилятора в памяти.

Linux и macOS. Проект не связан с Rust Project и Rust Foundation.

---

## Зачем

- **rust-analyzer держит в памяти собственный анализ всего воркспейса**: 1.5 GB на serde и
  11.3 GB на Bevy в замере ниже. Он заново выполняет разрешение имён, раскрытие макросов и вывод типов,
  которые rustc и так делает при каждом `cargo check`.
- **Большая часть анализируемого кода — зависимости и std**, которые никто не редактирует. `ridx` индексирует
  каждую зависимость один раз и делит её между всеми проектами и worktree на машине.
- **То, что вы набираете, тоже работает**: на несохранённые правки сразу отвечает слой в памяти поверх
  индекса, а через 250 мс после последней правки изменённые крейты компилируются из буферов в индекс
  сессии, так что ссылки и переименование тоже видят новый код.

## Замеры

Одна и та же сессия редактора против обоих серверов: по 40 файлов каждого репозитория открываются по очереди,
с символами документа, семантической подсветкой, сворачиванием, inlay-подсказками, затем hover, переходом к
определению и к типу, подсветкой вхождений, ссылками и автодополнением в выбранных позициях, и одной правкой.
Память учитывает дочерние процессы.

| Репозиторий | Пик памяти, ridx / rust-analyzer | CPU за сессию | От старта до готовности |
|---|---:|---:|---:|
| serde | 72 MB / 1.5 GB | 2.34 s / 6.97 s | 0.06 s / 3.27 s |
| tokio | 221 MB / 3.2 GB | 1.90 s / 35.9 s | 0.24 s / 4.77 s |
| rust-analyzer | 434 MB / 4.6 GB | 4.44 s / 66.7 s | 0.64 s / 5.22 s |
| bevy | 917 MB / 11.3 GB | 5.38 s / 170.5 s | 1.26 s / 15.5 s |

- ridx `f7105fcc`, 2026-11-14; один прогон (n = 1) на Linux 6.8, AMD Ryzen 9 7950X, 64 GB. Сырые результаты:
  [`bench/published/resources_f7105fcc.jsonl`](../bench/published/resources_f7105fcc.jsonl), скрипт:
  [`bench/measure_resources.py`](../bench/measure_resources.py); задержки по методам — в сырых результатах и
  на сайте.
- **Не учтено**: ridx отвечает из индекса, построенного заранее (`ridx index` — это `cargo check` с
  драйвером ridx); в первый раз на большом воркспейсе это занимает минуты. rust-analyzer анализирует
  воркспейс при каждом запуске, и это время входит в его цифры.
- **«Готов»** — для ridx это ответ на `initialize` (кэши после него догреваются в фоне), для rust-analyzer —
  конец его анализа плюс 0.5 s без сообщений.
- **Память**: у ridx в цифру входят страницы индекса, отображённые с диска; ядро может выгрузить их при
  нехватке памяти и прочитать снова. У rust-analyzer это его собственная куча.
- rust-analyzer ответил ошибкой на 26–31 запрос в каждом репозитории, ridx — ни на один; виды ошибок не
  записывались. Запросы с ошибками входят в цифры задержек.

Совпадение ответов с rust-analyzer по каждому методу LSP измеряется набором в [`bench/`](../bench/)
и записывается для каждого релиза в [`CHANGELOG.md`](../CHANGELOG.md). Полная матрица из 78 методов для `v0.2.0` — в
[`docs/BENCHMARK_MATRIX.md`](../docs/BENCHMARK_MATRIX.md) и
[`LSP_BENCHMARK_COMPARISON_REPORT.md`](../LSP_BENCHMARK_COMPARISON_REPORT.md).

---

## Установка

Linux (x86_64, arm64) и macOS на Apple Silicon, с [rustup](https://rustup.rs):

```bash
curl -fsSL https://rust-intelligence.com/install.sh | sh
```

Установщик ставит Rust 1.98.1 — toolchain, внутри которого работает `ridx-driver` (он линкуется с
библиотеками этого компилятора; ваши проекты остаются на любом своём toolchain, ridx сам запускает сборку
индекса на нужном), скачивает с GitHub архив релиза для вашей платформы, проверяет его SHA-256 и кладёт
`ridx` и `ridx-driver` в `~/.local/bin` (`RIDX_INSTALL_DIR` меняет каталог, `RIDX_VERSION=v0.3.7` фиксирует релиз).

Или из [crates.io](https://crates.io/crates/ridx). Оба бинарника собирайте на Rust 1.98.1: `ridx-driver` линкуется с
библиотеками самого этого компилятора, поэтому ему нужны `rustc-dev` и эти два флага, а `ridx` строит индекс
toolchain'ом, которым его собрали. Из клона репозитория вместо имён крейтов укажите `--path crates/ridx` и
`--path crates/ridx-driver`.

```bash
rustup toolchain install 1.98.1 --profile minimal --component rustc-dev,rust-src
cargo +1.98.1 install ridx
RUSTC_BOOTSTRAP=1 RUSTFLAGS="-C prefer-dynamic" cargo +1.98.1 install ridx-driver
```

Затем откройте Cargo-воркспейс в редакторе. Если индекса ещё нет, `ridx` строит его при первом использовании и
показывает прогресс в редакторе; когда сборка заканчивается, он перезагружает индекс, а после сохранений
переиндексирует в фоне. Из терминала индекс строит `ridx index .` (`--all-features`, `--target <triple>`
выбирают cfg).

Если десктопный редактор не находит `ridx`, укажите ему полный путь из `which ridx`.

## Редакторы

`ridx` без подкоманды ведёт себя в командной строке как `rust-analyzer` (`--version`, `--stdio`,
`--log-file`, `--print-config-schema`), поэтому редактор, настроенный на rust-analyzer, может запускать его вместо него.

- **Zed**: расширение [`editors/zed`](../editors/zed) регистрирует `ridx lsp` как языковой сервер
  `rust-intelligence` (палитра команд: `zed: install dev extension`), а
  [`docs/editors/zed-settings.json`](../docs/editors/zed-settings.json) выбирает его для Rust.
- **VS Code**: [`docs/editors/vscode-settings.json`](../docs/editors/vscode-settings.json) направляет
  расширение rust-analyzer на `ridx` (`"rust-analyzer.server.path": "ridx"`).
- **Neovim**: [`docs/editors/neovim-lspconfig.lua`](../docs/editors/neovim-lspconfig.lua) (`cmd = { "ridx" }`,
  корень ищется сначала по `Cargo.lock`, поэтому крейт воркспейса открывает индекс всего воркспейса).
- **Helix**: [`docs/editors/helix-languages.toml`](../docs/editors/helix-languages.toml) объявляет языковой сервер `ridx` и выбирает его для Rust в `languages.toml`.

## Из терминала

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

## Методы LSP

| Категория | Методы |
|---|---|
| Навигация | `definition`, `declaration`, `implementation`, `typeDefinition` |
| Ссылки | `references`, `documentHighlight` |
| Подсказки | `hover`, `completion` (с автоимпортом), `signatureHelp`, `inlayHint`, `semanticTokens` |
| Иерархии | `prepareCallHierarchy`, `incomingCalls`, `outgoingCalls`, `prepareTypeHierarchy`, `supertypes`, `subtypes` |
| Рефакторинг | `prepareRename`, `rename`, `codeAction` (правки, не ломающие компиляцию), `executeCommand` |
| Форматирование | `formatting`, `rangeFormatting` |
| Символы | `documentSymbol`, `workspace/symbol` |
| Диагностика | `publishDiagnostics` из фонового `cargo check` при сохранении, pull-диагностика |
| Воркспейсы | `didChangeWorkspaceFolders`, `didChangeWatchedFiles`, несколько корней |

## Как это устроено

<p align="center"><img src="../site/assets/architecture.svg" width="900" alt="How ridx works: at build time cargo check runs ridx-driver inside rustc and writes .ridx shards to disk; at edit time ridx lsp maps them and answers the editor"></p>

rustc и так на каждом `cargo check` разрешает все имена, раскрывает все макросы и выводит все типы. `ridx`
сохраняет эти ответы, а не вычисляет их заново.

- **Индекс пишет компилятор.** `ridx index` запускает `cargo check` с `ridx-driver` в роли обёртки rustc.
  После анализа драйвер обходит HIR и результаты проверки типов rustc и пишет по одному шарду `.ridx` на
  крейт: определения, каждую разрешённую ссылку (вызовы методов и поля — так, как их разрешил rustc, через
  auto-deref и выбор трейта), типы, impl, поля, обобщённые параметры, макросы и `use`. Затем компиляция идёт
  как обычно, так что сама проверка не меняется.
- **Зависимости индексируются один раз на машину.** Шард зависимости кладётся в хранилище с адресацией по
  содержимому, с ключом из крейта, версии, целевой платформы, флагов cfg и хэшей того, с чем он собран; если
  там уже есть годный шард, драйвер не извлекает крейт заново. Проекты и worktree делят эти шарды, а `std`, `core`
  и `alloc` индексируются один раз на тулчейн.
- **Чтение — это отображение в память, а не разбор.** Шард v5 — little-endian, записи фиксированного размера
  в секциях, выровненных по 8 байтам: пул строк, файлы (16 B), определения (40 B) с хэш-индексом FNV-1a, ссылки,
  отсортированные по позиции (48 B), с хэш-индексом целей, и таблицы типов, impl, полей и обобщённых
  параметров. Поиск — хэш-проба, двоичный поиск или просмотр отображённых страниц. Страницы, которые никто не
  запрашивал, не читаются вовсе, а остальные ядро может выгрузить при нехватке памяти.
- **Пересборка инкрементальная и безопасная.** После сохранения сервер переиндексирует в фоне; если
  изменились только исходники воркспейса, он вызывает драйвер на изменённых крейтах напрямую, без Cargo.
  Крейт с ошибками компиляции индексируется по всему, что разрешилось; крейт, который компилятор не может
  довести до этого (синтаксическая ошибка), сохраняет последний удачный шард. Другой набор features или
  целевая платформа получает свой вариант рядом с первым.
- **Код, набранный между сборками,** отображается на скомпилированный текст построчно, а изменённый код
  анализирует небольшой фронтенд без компилятора ([`crates/ridx-syntax`](../crates/ridx-syntax): лексер,
  парсер, разрешение имён, вывод типов), который читает все остальные типы и элементы из индекса. Через
  250 мс после последней правки крейты, которым принадлежат изменённые буферы, компилируются из их текста
  в индекс сессии (в файлы ничего не пишется); он отвечает на ссылки, переименование и навигацию до
  следующей сборки.

Устройство целиком, с раскладкой шарда и тем, где какая часть лежит в коде:
[`docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md).

## Известные ограничения

- **Несохранённый код попадает в индекс с задержкой.** Изменённые крейты компилируются из буферов через
  250 мс после последней правки, это занимает от доли секунды до нескольких секунд для большого крейта; до
  этого ссылки и навигация берутся из предыдущего индекса с поправкой на правку. Крейты, зависящие от
  изменённого, видят изменение после сохранения.
- **Один cfg.** Как и `cargo check`, индекс видит одну цель и один набор фич; код под другими
  `#[cfg]` или фичами не индексируется. Они выбираются через `ridx index --all-features` / `--target`.
- **Windows** пока не поддерживается.
- **Стандартная библиотека**: навигация и hover в `std`, `core` и `alloc` работают; ссылки на них ищутся
  только в воркспейсе, и переименовать их нельзя.
- `vec![]` считается ссылкой на `Vec`; rust-analyzer так не считает.

---

## Разработка

Каждый исходный файл — не длиннее 400 строк.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Цитирование

Если вы используете ridx в исследованиях или документации, пожалуйста, цитируйте архивный релиз в Zenodo:
[10.5281/zenodo.24379816](https://doi.org/10.5281/zenodo.24379816) — для проекта (ведёт на последнюю версию), [10.5281/zenodo.24397432](https://doi.org/10.5281/zenodo.24397432) — ровно для 0.3.10.
[`CITATION.cff`](../CITATION.cff) содержит метаданные, которые использует кнопка GitHub «Cite this repository».

```bibtex
@software{panasenko_2026_24397432,
  author    = {Panasenko, Alexander},
  title     = {Rust Intelligence (ridx) — a Rust language server answering from a compiler-built on-disk index},
  month     = nov,
  year      = 2026,
  publisher = {Zenodo},
  version   = {0.3.10},
  doi       = {10.5281/zenodo.24397432},
  url       = {https://doi.org/10.5281/zenodo.24397432},
}
```

## Лицензия

Лицензируется под одной из лицензий:

- Apache License, Version 2.0 ([LICENSE-APACHE](../LICENSE-APACHE) или http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](../LICENSE-MIT) или http://opensource.org/licenses/MIT)

по вашему выбору.
