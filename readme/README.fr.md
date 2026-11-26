<!-- translation-source: b2617c197ca30ad6ef4348e88121b8f4f03cafbe -->

> [!NOTE]
> Ceci est une traduction du README anglais au commit [b2617c1](https://github.com/alex09x/rust-intelligence/blob/b2617c197ca30ad6ef4348e88121b8f4f03cafbe/README.md). La version de référence, et la plus à jour, est le [README en anglais](https://github.com/alex09x/rust-intelligence/blob/main/README.md).

<p align="center">
  <img src="../site/assets/logo-minimal.svg" width="96" height="96" alt="ridx logo">
</p>

<h1 align="center">Rust Intelligence (<code>ridx</code>)</h1>

<p align="center">
  <b>L'intelligence de code Rust pour un dixième de la mémoire.</b><br>
  Un serveur de langage pour Rust qui répond avec ce que rustc a déjà calculé.
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
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ja.md">日本語</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.de.md">Deutsch</a> ·
  <b><a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.fr.md">Français</a></b> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ko.md">한국어</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ar.md">العربية</a>
</p>

`ridx` enregistre une seule fois, pendant `cargo check`, les définitions, références et types dans un index binaire,
puis lit cet index directement sur le disque. Votre éditeur obtient navigation, survol, complétion, références et
renommage sans un second compilateur résident en mémoire.

Linux, macOS et Windows. Sans lien avec le Rust Project ni la Rust Foundation.

---

## Pourquoi

- **rust-analyzer garde en mémoire sa propre analyse de tout le workspace** : 1.5 GB sur serde et
  11.3 GB sur Bevy dans la mesure ci-dessous. Il refait la résolution de noms, l'expansion des macros et
  l'inférence de types que rustc effectue de toute façon à chaque `cargo check`.
- **L'essentiel de ce qui est analysé, ce sont les dépendances et std**, du code que personne ne modifie. `ridx`
  indexe chaque dépendance une seule fois et la partage entre tous les projets et worktrees de la machine.
- **Ce que vous tapez fonctionne aussi** : une couche en mémoire au-dessus de l'index répond aussitôt aux
  modifications non enregistrées, et 250 ms après la dernière modification les crates modifiés sont compilés
  depuis les buffers dans un index de la session, si bien que les références et le renommage voient aussi le
  nouveau code. Les erreurs du compilateur y sont aussi signalées.

## Mesures

La même session d'éditeur contre les deux serveurs : 40 fichiers de chaque dépôt ouverts un à un, avec symboles du
document, jetons sémantiques, repliage et inlay hints, puis survol, définition, définition de type, surlignage,
références et complétion à des positions échantillonnées, et une modification. La mémoire inclut les processus enfants.

| Dépôt | Mémoire maximale, ridx / rust-analyzer | CPU de la session | Du démarrage à prêt |
|---|---:|---:|---:|
| serde | 72 MB / 1.5 GB | 2.34 s / 6.97 s | 0.06 s / 3.27 s |
| tokio | 221 MB / 3.2 GB | 1.90 s / 35.9 s | 0.24 s / 4.77 s |
| rust-analyzer | 434 MB / 4.6 GB | 4.44 s / 66.7 s | 0.64 s / 5.22 s |
| bevy | 917 MB / 11.3 GB | 5.38 s / 170.5 s | 1.26 s / 15.5 s |

- ridx `f7105fcc`, 2026-11-14 ; une exécution (n = 1) sur Linux 6.8, AMD Ryzen 9 7950X, 64 GB. Résultats bruts :
  [`bench/published/resources_f7105fcc.jsonl`](../bench/published/resources_f7105fcc.jsonl), script :
  [`bench/measure_resources.py`](../bench/measure_resources.py) ; la latence par méthode figure dans les résultats
  bruts et sur le site.
- **Non compté** : ridx répond à partir d'un index construit au préalable (`ridx index`, un `cargo check` avec le
  driver ridx), ce qui prend quelques minutes la première fois sur un gros workspace. rust-analyzer analyse le
  workspace à chaque démarrage, et ce temps figure dans ses chiffres.
- **« Prêt »** désigne la réponse à `initialize` pour ridx (les caches continuent de chauffer en arrière-plan
  ensuite) et, pour rust-analyzer, la fin de son analyse plus 0.5 s sans message.
- **Mémoire** : le chiffre de ridx inclut des pages de l'index mappées depuis le disque, que le noyau peut libérer
  sous pression et relire. Celui de rust-analyzer est son propre tas.
- rust-analyzer a répondu par une erreur à 26–31 requêtes par dépôt, ridx à aucune ; la nature des erreurs n'a pas
  été enregistrée. Les requêtes en erreur sont incluses dans les chiffres de latence.

La concordance des réponses avec rust-analyzer par méthode LSP est mesurée avec la suite de [`bench/`](../bench/)
et consignée à chaque version dans [`CHANGELOG.md`](../CHANGELOG.md). La matrice complète des 78 méthodes de `v0.2.0` se trouve dans
[`docs/BENCHMARK_MATRIX.md`](../docs/BENCHMARK_MATRIX.md) et
[`LSP_BENCHMARK_COMPARISON_REPORT.md`](../LSP_BENCHMARK_COMPARISON_REPORT.md).

---

## Installation

Linux (x86_64, arm64), macOS (Apple Silicon, Intel) et Windows (x86_64), avec
[rustup](https://rustup.rs) :

```bash
curl -fsSL https://rust-intelligence.com/install.sh | sh
```

Sous Windows, dans PowerShell :

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://rust-intelligence.com/install.ps1 | iex"
```

L'installeur ajoute Rust 1.98.1, la toolchain dans laquelle s'exécute `ridx-driver` (il se lie aux bibliothèques de
ce compilateur ; vos projets gardent n'importe quelle toolchain, ridx construit l'index avec la bonne), télécharge
depuis GitHub l'archive de la version pour votre plateforme, vérifie son SHA-256 et place `ridx` et `ridx-driver`
dans `~/.local/bin` (sous Windows, `%USERPROFILE%\.local\bin`, qu'il ajoute à votre PATH ; `RIDX_INSTALL_DIR`
le change, `RIDX_VERSION=v0.3.14` fixe une version).

Ou depuis [crates.io](https://crates.io/crates/ridx). Compilez les deux binaires avec Rust 1.98.1 : `ridx-driver` se
lie aux bibliothèques de ce compilateur, il lui faut donc `rustc-dev` et ces deux options, et `ridx` construit
l'index avec la toolchain qui l'a compilé. Depuis un clone du dépôt, utilisez `--path crates/ridx` et
`--path crates/ridx-driver` à la place des noms de crates.

```bash
rustup toolchain install 1.98.1 --profile minimal --component rustc-dev,rust-src
cargo +1.98.1 install ridx
RUSTC_BOOTSTRAP=1 RUSTFLAGS="-C prefer-dynamic" cargo +1.98.1 install ridx-driver
```

Ouvrez ensuite un workspace Cargo dans votre éditeur. S'il n'y a pas encore d'index, `ridx` en construit un à la
première utilisation et affiche la progression dans l'éditeur ; il recharge l'index à la fin de la compilation et
réindexe en arrière-plan après chaque enregistrement. `ridx index .` le construit depuis le terminal
(`--all-features`, `--target <triple>` choisissent le cfg).

Si un éditeur de bureau ne trouve pas `ridx`, donnez-lui le chemin complet renvoyé par `which ridx`.

## Éditeurs

Sans sous-commande, `ridx` se comporte comme `rust-analyzer` en ligne de commande (`--version`, `--stdio`,
`--log-file`, `--print-config-schema`), donc un éditeur configuré pour rust-analyzer peut le lancer à la place.

- **VS Code** : l'extension [Rust Intelligence](https://marketplace.visualstudio.com/items?itemName=alex09x.rust-intelligence)
  ([`editors/vscode`](../editors/vscode)) lance `ridx lsp` et propose d'installer `ridx` si elle ne le trouve pas.
  Ou faites pointer l'extension rust-analyzer vers `ridx` :
  [`docs/editors/vscode-settings.json`](../docs/editors/vscode-settings.json) (`"rust-analyzer.server.path": "ridx"`).
- **Zed** : l'extension [`editors/zed`](../editors/zed) enregistre `ridx lsp` comme serveur de langage
  `rust-intelligence` et télécharge `ridx` s'il n'est pas dans le PATH (palette de commandes : `zed: install dev
  extension`) ; [`docs/editors/zed-settings.json`](../docs/editors/zed-settings.json) le sélectionne pour Rust.
- **Neovim** : [`docs/editors/neovim-lspconfig.lua`](../docs/editors/neovim-lspconfig.lua) (`cmd = { "ridx" }`,
  la racine est cherchée d'abord par `Cargo.lock`, ainsi un membre du workspace ouvre l'index du workspace).
- **Helix** : [`docs/editors/helix-languages.toml`](../docs/editors/helix-languages.toml) déclare le serveur de langage `ridx` et le sélectionne pour Rust dans `languages.toml`.

## Depuis le terminal

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

## Méthodes LSP

| Catégorie | Méthodes |
|---|---|
| Navigation | `definition`, `declaration`, `implementation`, `typeDefinition` |
| Références | `references`, `documentHighlight` |
| Intelligence | `hover`, `completion` (avec import automatique), `signatureHelp`, `inlayHint`, `semanticTokens` |
| Hiérarchies | `prepareCallHierarchy`, `incomingCalls`, `outgoingCalls`, `prepareTypeHierarchy`, `supertypes`, `subtypes` |
| Refactorisation | `prepareRename`, `rename`, `codeAction` (assistances sûres pour la compilation), `executeCommand` |
| Formatage | `formatting`, `rangeFormatting` |
| Symboles | `documentSymbol`, `workspace/symbol` |
| Diagnostics | `publishDiagnostics` d'un `cargo check` en arrière-plan à l'enregistrement et de la compilation des tampons non enregistrés, diagnostics pull |
| Workspaces | `didChangeWorkspaceFolders`, `didChangeWatchedFiles`, multi-racine |

## Fonctionnement

<p align="center"><img src="../site/assets/architecture.svg" width="900" alt="How ridx works: at build time cargo check runs ridx-driver inside rustc and writes .ridx shards to disk; at edit time ridx lsp maps them and answers the editor"></p>

rustc résout déjà chaque nom, développe chaque macro et infère chaque type à chaque `cargo check`. `ridx`
conserve ces réponses au lieu de les recalculer.

- **C'est le compilateur qui écrit l'index.** `ridx index` lance `cargo check` avec `ridx-driver` comme
  wrapper de rustc. Après l'analyse, le driver parcourt le HIR et les résultats de la vérification de types
  de rustc et écrit un shard `.ridx` par crate : définitions, chaque référence résolue (appels de méthode et
  champs tels que rustc les a résolus, via l'auto-déréférencement et la sélection de traits), types, impls,
  champs, génériques, macros et éléments `use`. La compilation continue ensuite normalement, la
  vérification elle-même ne change donc pas.
- **Les dépendances sont indexées une fois par machine.** Le shard d'une dépendance va dans un magasin
  adressé par contenu, avec une clé formée du crate, de la version, de la cible, des flags cfg et des
  empreintes de ce contre quoi elle a été compilée ; si un shard valide s'y trouve, le driver n'extrait pas le
  crate à nouveau. Projets et worktrees partagent ces shards, et `std`, `core` et `alloc` sont indexés une fois par
  toolchain.
- **Lire, c'est mapper, pas analyser.** Le shard v5 est little-endian, avec des enregistrements de taille
  fixe dans des sections alignées sur 8 octets : un pool de chaînes, les fichiers (16 B), les définitions (40 B) avec un index
  de hachage FNV-1a, les références triées par position (48 B) avec un index de hachage des cibles, et des
  tables de types, d'impls, de champs et de génériques. Une recherche est une sonde de hachage, une
  recherche dichotomique ou un parcours sur des pages mappées. Les pages que personne ne demande ne sont jamais lues, et le
  noyau peut libérer les autres en cas de manque de mémoire.
- **Les recompilations sont incrémentales et sûres.** Après un enregistrement, le serveur réindexe en
  arrière-plan ; quand seules des sources du workspace ont changé, il appelle le driver directement sur les
  crates modifiés, sans Cargo. Un crate avec des erreurs de compilation est indexé à partir de tout ce qui a
  été résolu ; un crate que le compilateur ne peut pas mener jusque-là (une erreur de syntaxe) garde son
  dernier shard valide. Un autre jeu de features ou une autre cible obtient sa propre variante à côté de la
  première.
- **Le code tapé entre deux compilations** est mis en correspondance ligne à ligne avec le texte compilé, et
  le code modifié est analysé par un petit front end sans compilateur ([`crates/ridx-syntax`](../crates/ridx-syntax) :
  lexer, parser, résolution des noms, inférence de types) qui lit tous les autres types et éléments dans
  l'index. 250 ms après la dernière modification, les crates des buffers modifiés sont compilés depuis leur
  texte dans un index de la session (rien n'est écrit dans les fichiers), qui répond aux références, au
  renommage et à la navigation jusqu'à la compilation suivante.

La conception complète, avec la structure d'un shard et l'emplacement de chaque partie dans le code :
[`docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md).

## Limites connues

- **Le code non enregistré atteint l'index avec un délai.** Les crates modifiés sont compilés depuis les
  buffers 250 ms après la dernière modification, ce qui prend d'une fraction de seconde à quelques secondes
  pour un gros crate ; d'ici là, les références et la navigation viennent de l'index précédent, ajusté à la
  modification. Les crates qui dépendent d'un crate modifié voient le changement après l'enregistrement.
- **Un seul cfg.** Comme `cargo check`, l'index voit une cible et un jeu de features ; le code derrière d'autres
  `#[cfg]` ou features n'est pas indexé. Choisissez-les avec `ridx index --all-features` / `--target`.
- **Des binaires précompilés** existent pour Linux x86_64 et arm64, macOS sur Apple Silicon et Intel, et
  Windows x86_64 ; sur les autres plateformes, installez depuis crates.io.
- **Bibliothèque standard** : la navigation et le survol vers `std`, `core` et `alloc` fonctionnent ; les
  références vers eux sont cherchées dans le workspace seulement, et ils ne peuvent pas être renommés.
- `vec![]` compte comme une référence à `Vec` ; rust-analyzer ne le compte pas.

---

## Développement

Aucun fichier source ne dépasse 400 lignes.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Citer

Si vous utilisez ridx dans un travail de recherche ou une documentation, citez la version archivée sur Zenodo :
[10.5281/zenodo.24379816](https://doi.org/10.5281/zenodo.24379816) pour le projet (il pointe vers la dernière version), [10.5281/zenodo.24546413](https://doi.org/10.5281/zenodo.24546413) pour exactement 0.3.14.
[`CITATION.cff`](../CITATION.cff) contient les métadonnées qu'utilise « Cite this repository » de GitHub.

```bibtex
@software{panasenko_2026_24546413,
  author    = {Panasenko, Alexander},
  title     = {Rust Intelligence (ridx) — a Rust language server answering from a compiler-built on-disk index},
  month     = nov,
  year      = 2026,
  publisher = {Zenodo},
  version   = {0.3.14},
  doi       = {10.5281/zenodo.24546413},
  url       = {https://doi.org/10.5281/zenodo.24546413},
}
```

## Licence

Sous licence, au choix :

- Apache License, Version 2.0 ([LICENSE-APACHE](../LICENSE-APACHE) ou http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](../LICENSE-MIT) ou http://opensource.org/licenses/MIT)

selon votre préférence.
