<!-- translation-source: 46dfbadec54a2f01ab5d8cb2b05bfbed4c7f2540 -->

> [!NOTE]
> Ceci est une traduction du README anglais au commit [46dfbad](https://github.com/alex09x/rust-intelligence/blob/46dfbadec54a2f01ab5d8cb2b05bfbed4c7f2540/README.md). La version de référence, et la plus à jour, est le [README en anglais](https://github.com/alex09x/rust-intelligence/blob/main/README.md).

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
  <img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS-1b1d20" alt="Platforms: Linux, macOS">
  <img src="https://img.shields.io/badge/LSP-3.18-1b1d20" alt="LSP 3.18">
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

Linux et macOS. Sans lien avec le Rust Project ni la Rust Foundation.

---

## Pourquoi

- **rust-analyzer garde en mémoire sa propre analyse de tout le workspace** : 1.5 GB sur serde et
  11.3 GB sur Bevy dans la mesure ci-dessous. Il refait la résolution de noms, l'expansion des macros et
  l'inférence de types que rustc effectue de toute façon à chaque `cargo check`.
- **L'essentiel de ce qui est analysé, ce sont les dépendances et std**, du code que personne ne modifie. `ridx`
  indexe chaque dépendance une seule fois et la partage entre tous les projets et worktrees de la machine.
- **Ce que vous tapez fonctionne aussi** : les modifications non enregistrées sont servies par une couche en
  mémoire au-dessus de l'index jusqu'à la prochaine compilation du projet.

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

Linux (x86_64, arm64) et macOS sur Apple Silicon, avec [rustup](https://rustup.rs) :

```bash
curl -fsSL https://rust-intelligence.com/install.sh | sh
```

L'installeur ajoute Rust 1.98.1, la toolchain dans laquelle s'exécute `ridx-driver` (il se lie aux bibliothèques de
ce compilateur ; vos projets gardent n'importe quelle toolchain, ridx construit l'index avec la bonne), télécharge
depuis GitHub l'archive de la version pour votre plateforme, vérifie son SHA-256 et place `ridx` et `ridx-driver`
dans `~/.local/bin` (`RIDX_INSTALL_DIR` le change, `RIDX_VERSION=v0.3.7` fixe une version).

Pour compiler depuis les sources :

```bash
rustup toolchain install 1.98.1 --component rustc-dev,llvm-tools-preview,rust-src,rustfmt,clippy
git clone https://github.com/alex09x/rust-intelligence
cd rust-intelligence
RUSTC_BOOTSTRAP=1 cargo +1.98.1 install --path crates/ridx --locked
RUSTC_BOOTSTRAP=1 cargo +1.98.1 install --path crates/ridx-driver --locked
```

Ouvrez ensuite un workspace Cargo dans votre éditeur. S'il n'y a pas encore d'index, `ridx` en construit un à la
première utilisation et affiche la progression dans l'éditeur ; il recharge l'index à la fin de la compilation et
réindexe en arrière-plan après chaque enregistrement. `ridx index .` le construit depuis le terminal
(`--all-features`, `--target <triple>` choisissent le cfg).

Si un éditeur de bureau ne trouve pas `ridx`, donnez-lui le chemin complet renvoyé par `which ridx`.

## Éditeurs

Sans sous-commande, `ridx` se comporte comme `rust-analyzer` en ligne de commande (`--version`, `--stdio`,
`--log-file`, `--print-config-schema`), donc un éditeur configuré pour rust-analyzer peut le lancer à la place.

- **Zed** : l'extension [`editors/zed`](../editors/zed) enregistre `ridx lsp` comme serveur de langage
  `rust-intelligence` (palette de commandes : `zed: install dev extension`), et
  [`docs/editors/zed-settings.json`](../docs/editors/zed-settings.json) le sélectionne pour Rust.
- **VS Code** : [`docs/editors/vscode-settings.json`](../docs/editors/vscode-settings.json) fait pointer
  l'extension rust-analyzer vers `ridx` (`"rust-analyzer.server.path": "ridx"`).
- **Neovim** : [`docs/editors/neovim-lspconfig.lua`](../docs/editors/neovim-lspconfig.lua) (`cmd = { "ridx" }`,
  la racine est cherchée d'abord par `Cargo.lock`, ainsi un membre du workspace ouvre l'index du workspace).
- **Helix** : un `[language-server.ridx]` avec `command = "ridx"`, `args = ["lsp"]` dans `languages.toml`.

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
| Refactorisation | `prepareRename`, `rename`, `codeAction` (assistances vérifiées par compilation), `executeCommand` |
| Formatage | `formatting`, `rangeFormatting` |
| Symboles | `documentSymbol`, `workspace/symbol` |
| Diagnostics | `publishDiagnostics` d'un `cargo check` en arrière-plan à l'enregistrement, diagnostics pull |
| Workspaces | `didChangeWorkspaceFolders`, `didChangeWatchedFiles`, multi-racine |

## Fonctionnement

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

Le shard `.ridx` v5 est little-endian, avec des enregistrements de taille fixe alignés sur 8 octets : un en-tête, un
pool de chaînes, la table des fichiers (16 B par entrée), les définitions (40 B) avec un index de hachage FNV-1a,
des tables owner et deref pour les chaînes de récepteurs, les références triées par position (48 B) avec un index
de hachage des cibles, et des tables de types, d'impl, de champs et de génériques. Les recherches sont binaires en
mémoire mappée ; rien n'est analysé dans le tas, et les fichiers de shard sont fermés une fois mappés.

## Limites connues

- **Les réponses viennent de la dernière compilation.** Le code tapé depuis est servi par le buffer ouvert et
  rattrapé à la prochaine compilation ; un crate qui ne compile pas garde son dernier index valide.
- **Un seul cfg.** Comme `cargo check`, l'index voit une cible et un jeu de features ; le code derrière d'autres
  `#[cfg]` ou features n'est pas indexé. Choisissez-les avec `ridx index --all-features` / `--target`.
- **Windows** n'est pas encore pris en charge.
- **Bibliothèque standard** : la navigation et le survol vers `std`, `core` et `alloc` fonctionnent ; les
  références vers eux sont cherchées dans le workspace seulement, et ils ne peuvent pas être renommés.
- **Les appels entrants** listent un appelant là où la fonction est appelée (`name(` ou `name::<…>(`) ;
  rust-analyzer liste aussi les fonctions passées comme valeurs, par exemple un système donné à `add_systems`.
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
[10.5281/zenodo.24379816](https://doi.org/10.5281/zenodo.24379816) pour le projet (il pointe vers la dernière version), [10.5281/zenodo.24379817](https://doi.org/10.5281/zenodo.24379817) pour exactement 0.3.7.
[`CITATION.cff`](../CITATION.cff) contient les métadonnées qu'utilise « Cite this repository » de GitHub.

```bibtex
@software{panasenko_2026_24379817,
  author    = {Panasenko, Alexander},
  title     = {Rust Intelligence (ridx) — a Rust language server answering from a compiler-built on-disk index},
  month     = nov,
  year      = 2026,
  publisher = {Zenodo},
  version   = {0.3.7},
  doi       = {10.5281/zenodo.24379817},
  url       = {https://doi.org/10.5281/zenodo.24379817},
}
```

## Licence

Sous licence, au choix :

- Apache License, Version 2.0 ([LICENSE-APACHE](../LICENSE-APACHE) ou http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](../LICENSE-MIT) ou http://opensource.org/licenses/MIT)

selon votre préférence.
