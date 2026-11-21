<!-- translation-source: dd1f795a87ee6091c77465eb659661c234a70214 -->

> [!NOTE]
> هذه ترجمة لملف README الإنجليزي عند الإيداع [dd1f795](https://github.com/alex09x/rust-intelligence/blob/dd1f795a87ee6091c77465eb659661c234a70214/README.md). النسخة المرجعية والأحدث هي [ملف README باللغة الإنجليزية](https://github.com/alex09x/rust-intelligence/blob/main/README.md).

<p align="center">
  <img src="../site/assets/logo-minimal.svg" width="96" height="96" alt="ridx logo">
</p>

<h1 align="center">Rust Intelligence (<code>ridx</code>)</h1>

<p align="center">
  <b>ذكاء شيفرة Rust بعُشر الذاكرة.</b><br>
  خادم لغة لـ Rust يجيب بما حسبه rustc بالفعل.
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
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ru.md">Русский</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.zh-CN.md">简体中文</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.es.md">Español</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.pt-BR.md">Português (Brasil)</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ja.md">日本語</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.de.md">Deutsch</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.fr.md">Français</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ko.md">한국어</a> ·
  <b><a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ar.md">العربية</a></b>
</p>

<div dir="rtl">

يسجّل `ridx` التعريفات والمراجع والأنواع مرة واحدة، أثناء `cargo check`، في فهرس ثنائي، ثم يقرأ هذا الفهرس
مباشرة من القرص. يحصل محرّرك على التنقّل والتلميحات عند المرور والإكمال التلقائي والمراجع وإعادة التسمية من دون
مترجم ثانٍ مقيم في الذاكرة.

يعمل على Linux وmacOS. لا ينتمي إلى Rust Project ولا إلى Rust Foundation.

---

## لماذا

- **يحتفظ rust-analyzer في الذاكرة بتحليله الخاص لمساحة العمل كلها**: ‏1.5 GB على serde و
  11.3 GB على Bevy في القياس أدناه. وهو يكرّر حلّ الأسماء وتوسيع الماكرو واستنتاج الأنواع التي ينفّذها rustc
  على أي حال في كل `cargo check`.
- **معظم ما يُحلَّل هو الاعتماديات وstd**، وهي شيفرة لا يعدّلها أحد. يفهرس `ridx` كل اعتمادية مرة واحدة
  ويشاركها بين كل المشاريع وworktree على الجهاز.
- **ما تكتبه الآن يعمل أيضًا**: تُجاب التعديلات غير المحفوظة فورًا من طبقة في الذاكرة فوق الفهرس، وبعد 250 ms
  من آخر تعديل تُترجَم الصناديق (crates) المعدّلة من المخازن المؤقتة إلى فهرس للجلسة، فترى المراجع وإعادة التسمية
  الشيفرة الجديدة أيضًا. وتُبلَّغ أيضًا أخطاء المترجم فيه.

## القياسات

جلسة المحرّر نفسها على الخادمَين: تُفتح 40 ملفًا من كل مستودع واحدًا تلو الآخر، مع رموز المستند والرموز الدلالية
والطيّ والتلميحات المضمّنة، ثم المرور والتعريف وتعريف النوع والتظليل والمراجع والإكمال في مواضع مختارة، وتعديل واحد.
تشمل الذاكرة العمليات الفرعية.

| المستودع | ذروة الذاكرة، ridx / rust-analyzer | وقت المعالج للجلسة | من البدء حتى الجاهزية |
|---|---:|---:|---:|
| serde | 72 MB / 1.5 GB | 2.34 s / 6.97 s | 0.06 s / 3.27 s |
| tokio | 221 MB / 3.2 GB | 1.90 s / 35.9 s | 0.24 s / 4.77 s |
| rust-analyzer | 434 MB / 4.6 GB | 4.44 s / 66.7 s | 0.64 s / 5.22 s |
| bevy | 917 MB / 11.3 GB | 5.38 s / 170.5 s | 1.26 s / 15.5 s |

- ridx `f7105fcc`، ‏2026-11-14؛ تشغيل واحد (n = 1) على Linux 6.8 وAMD Ryzen 9 7950X و64 GB. النتائج الخام:
  [`bench/published/resources_f7105fcc.jsonl`](../bench/published/resources_f7105fcc.jsonl)، السكربت:
  [`bench/measure_resources.py`](../bench/measure_resources.py)؛ زمن الاستجابة لكل طريقة موجود في النتائج الخام
  وعلى الموقع.
- **غير محتسب**: يجيب ridx من فهرس مبني مسبقًا (`ridx index`، وهو `cargo check` مع مشغّل ridx)، ويستغرق ذلك
  دقائق في المرة الأولى على مساحة عمل كبيرة. أما rust-analyzer فيحلّل مساحة العمل في كل مرة يبدأ فيها، وهذا الوقت
  داخل أرقامه.
- **«الجاهزية»** هي الرد على `initialize` بالنسبة إلى ridx (تستمر الذواكر المخبئية في الإحماء في الخلفية بعده)،
  وبالنسبة إلى rust-analyzer نهاية تحليله مضافًا إليها 0.5 s بلا رسائل.
- **الذاكرة**: يشمل رقم ridx صفحات الفهرس المعيّنة من القرص، ويمكن للنواة التخلّي عنها عند الضغط ثم قراءتها
  مجددًا. أما رقم rust-analyzer فهو الكومة الخاصة به.
- ردّ rust-analyzer بخطأ على 26–31 طلبًا في كل مستودع، وridx على لا شيء؛ لم تُسجَّل أنواع الأخطاء.
  الطلبات التي انتهت بخطأ محتسبة في أرقام زمن الاستجابة.

يُقاس تطابق الإجابات مع rust-analyzer لكل طريقة من طرق LSP بمجموعة الاختبارات في [`bench/`](../bench/)
ويُسجَّل لكل إصدار في [`CHANGELOG.md`](../CHANGELOG.md). المصفوفة الكاملة للطرق الـ 78 في `v0.2.0` موجودة في
[`docs/BENCHMARK_MATRIX.md`](../docs/BENCHMARK_MATRIX.md) و
[`LSP_BENCHMARK_COMPARISON_REPORT.md`](../LSP_BENCHMARK_COMPARISON_REPORT.md).

---

## التثبيت

على Linux ‏(x86_64، arm64) وmacOS على Apple Silicon، مع [rustup](https://rustup.rs):

</div>

<div dir="ltr">

```bash
curl -fsSL https://rust-intelligence.com/install.sh | sh
```

</div>

<div dir="rtl">

يضيف المثبّت Rust 1.98.1، وهي سلسلة الأدوات التي يعمل داخلها `ridx-driver` (يرتبط بمكتبات ذلك المترجم؛ تحتفظ
مشاريعك بأي سلسلة أدوات، ويبني ridx الفهرس بالسلسلة الصحيحة)، وينزّل من GitHub أرشيف الإصدار لمنصتك، ويتحقق من
SHA-256 الخاص به، ويضع `ridx` و`ridx-driver` في `~/.local/bin` (يغيّره `RIDX_INSTALL_DIR`، ويثبّت
`RIDX_VERSION=v0.3.7` إصدارًا محددًا).

أو من [crates.io](https://crates.io/crates/ridx). ابنِ الملفين التنفيذيين بـ Rust 1.98.1: يرتبط `ridx-driver` بمكتبات ذلك المترجم نفسه، لذا يحتاج إلى `rustc-dev` وإلى هذين الخيارين، ويشغّل `ridx` بناء الفهرس بسلسلة الأدوات التي بُني بها. من نسخة مستنسخة من المستودع، استخدم `--path crates/ridx` و`--path crates/ridx-driver` بدل اسمي الحزمتين.

</div>

<div dir="ltr">

```bash
rustup toolchain install 1.98.1 --profile minimal --component rustc-dev,rust-src
cargo +1.98.1 install ridx
RUSTC_BOOTSTRAP=1 RUSTFLAGS="-C prefer-dynamic" cargo +1.98.1 install ridx-driver
```

</div>

<div dir="rtl">

ثم افتح مساحة عمل Cargo في محرّرك. إن لم يكن هناك فهرس بعد، يبنيه `ridx` عند أول استخدام ويعرض التقدّم في
المحرّر؛ ويعيد تحميل الفهرس عند انتهاء البناء ويعيد الفهرسة في الخلفية بعد الحفظ. يبنيه `ridx index .` من الطرفية
(`--all-features` و`--target <triple>` يختاران cfg).

إذا لم يعثر محرّر سطح المكتب على `ridx`، فأعطه المسار الكامل الذي يعيده `which ridx`.

## المحرّرات

يتصرّف `ridx` من دون أمر فرعي مثل `rust-analyzer` في سطر الأوامر (`--version` و`--stdio` و
`--log-file` و`--print-config-schema`)، لذا يمكن لمحرّر مُعدّ لـ rust-analyzer أن يشغّله بدلًا منه.

- **Zed**: تسجّل الإضافة [`editors/zed`](../editors/zed) الأمر `ridx lsp` بوصفه خادم اللغة
  `rust-intelligence` (لوحة الأوامر: `zed: install dev extension`)، ويختاره
  [`docs/editors/zed-settings.json`](../docs/editors/zed-settings.json) للغة Rust.
- **VS Code**: يوجّه [`docs/editors/vscode-settings.json`](../docs/editors/vscode-settings.json)
  إضافة rust-analyzer إلى `ridx` (`"rust-analyzer.server.path": "ridx"`).
- **Neovim**: [`docs/editors/neovim-lspconfig.lua`](../docs/editors/neovim-lspconfig.lua) (`cmd = { "ridx" }`،
  يُبحث عن الجذر أولًا عبر `Cargo.lock`، فيفتح عضو مساحة العمل فهرس مساحة العمل كلها).
- **Helix**: يعرّف [`docs/editors/helix-languages.toml`](../docs/editors/helix-languages.toml) خادم اللغة `ridx` ويختاره للغة Rust في `languages.toml`.

## من الطرفية

</div>

<div dir="ltr">

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

</div>

<div dir="rtl">

---

## طرق LSP

| الفئة | الطرق |
|---|---|
| التنقّل | `definition`, `declaration`, `implementation`, `typeDefinition` |
| المراجع | `references`, `documentHighlight` |
| الذكاء | `hover`, `completion` (مع الاستيراد التلقائي), `signatureHelp`, `inlayHint`, `semanticTokens` |
| التسلسلات الهرمية | `prepareCallHierarchy`, `incomingCalls`, `outgoingCalls`, `prepareTypeHierarchy`, `supertypes`, `subtypes` |
| إعادة الهيكلة | `prepareRename`, `rename`, `codeAction` (مساعدات آمنة للترجمة), `executeCommand` |
| التنسيق | `formatting`, `rangeFormatting` |
| الرموز | `documentSymbol`, `workspace/symbol` |
| التشخيصات | `publishDiagnostics` من `cargo check` في الخلفية عند الحفظ ومن ترجمة المخازن غير المحفوظة، وتشخيصات السحب |
| مساحات العمل | `didChangeWorkspaceFolders`, `didChangeWatchedFiles`, جذور متعددة |

## كيف يعمل

</div>

<p align="center"><img src="../site/assets/architecture.svg" width="900" alt="How ridx works: at build time cargo check runs ridx-driver inside rustc and writes .ridx shards to disk; at edit time ridx lsp maps them and answers the editor"></p>

<div dir="rtl">

يحلّ rustc أصلًا كل اسم ويوسّع كل ماكرو ويستنتج كل نوع في كل `cargo check`. يحتفظ `ridx` بهذه الإجابات
بدلًا من حسابها من جديد.

- **المترجم هو من يكتب الفهرس.** يشغّل `ridx index` الأمر `cargo check` مع `ridx-driver` غلافًا لـ rustc.
  بعد التحليل يمرّ المشغّل على HIR ونتائج فحص الأنواع في rustc ويكتب شظية `.ridx` واحدة لكل crate:
  التعريفات، وكل مرجع محلول (استدعاءات الدوال والحقول كما حلّها rustc، عبر الإلغاء التلقائي للمرجع واختيار
  الـ trait)، والأنواع، وكتل impl، والحقول، والمعاملات العامة، والماكرو، وعناصر `use`. ثم تستمر الترجمة
  كالمعتاد، فلا يتغير الفحص نفسه.
- **تُفهرس الاعتماديات مرة واحدة لكل جهاز.** تذهب شظية الاعتمادية إلى مخزن معنون بالمحتوى، مفتاحه مؤلف من
  الـ crate والإصدار والمنصة الهدف وأعلام cfg وبصمات ما تُرجمت مقابله؛ وإذا وُجدت فيه شظية صالحة لا يعيد
  المشغّل استخراج الـ crate. تتشارك المشاريع ونسخ worktree هذه الشظايا، وتُفهرس `std` و`core` و`alloc` مرة واحدة
  لكل سلسلة أدوات.
- **القراءة ربط بالذاكرة لا تحليل.** شظية v5 بترتيب little-endian وسجلاتها ثابتة الحجم في أقسام محاذاة على 8 بايت:
  مجمّع سلاسل نصية، والملفات (16 B)، والتعريفات (40 B) مع فهرس تجزئة FNV-1a، والمراجع مرتبة حسب الموضع
  (48 B) مع فهرس تجزئة للأهداف، وجداول للأنواع وكتل impl والحقول والمعاملات العامة. البحث هو سبر تجزئة أو
  بحث ثنائي أو مسح على صفحات مربوطة. الصفحات التي لا يطلبها أحد لا تُقرأ أبدًا، ويمكن للنواة التخلص من غيرها عند
  ضغط الذاكرة.
- **إعادة البناء تدريجية وآمنة.** بعد الحفظ يعيد الخادم الفهرسة في الخلفية؛ وإذا تغيّرت مصادر مساحة العمل فقط، يستدعي
  المشغّل مباشرةً على الصناديق المتغيّرة دون Cargo. الصندوق الذي فيه أخطاء ترجمة يُفهرَس من كل ما أمكن حلّه؛ والصندوق
  الذي لا يستطيع المترجم الوصول به إلى هذه المرحلة (خطأ في الصياغة) يحتفظ بآخر جزء فهرس سليم له. ومجموعة features
  أخرى أو هدف آخر يحصل على نسخة خاصة به بجانب الأولى.
- **الشيفرة المكتوبة بين عمليات البناء** تُطابَق سطرًا بسطر مع النص المترجَم، ويحلّل الشيفرة المعدّلة واجهةٌ أمامية
  صغيرة بلا مترجم ([`crates/ridx-syntax`](../crates/ridx-syntax): محلل معجمي، ومحلل نحوي، وحلّ الأسماء، واستنتاج
  الأنواع) تقرأ كل الأنواع والعناصر الأخرى من الفهرس. وبعد 250 ms من آخر تعديل، تُترجَم الصناديق التي تملك المخازن
  المعدّلة من نصها إلى فهرس للجلسة (لا يُكتب شيء في الملفات)، فيجيب عن المراجع وإعادة التسمية والتنقل حتى البناء التالي.

التصميم كاملًا، مع بنية الشظية وموضع كل جزء في الشيفرة:
[`docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md).

## قيود معروفة

- **تصل الشيفرة غير المحفوظة إلى الفهرس بتأخير.** تُترجَم الصناديق المعدّلة من المخازن المؤقتة بعد 250 ms من آخر
  تعديل، ويستغرق ذلك من جزء من الثانية إلى بضع ثوانٍ لصندوق كبير؛ وحتى ذلك الحين تأتي المراجع والتنقل من الفهرس السابق
  مطابَقًا على التعديل. والصناديق التي تعتمد على صندوق معدّل ترى التغيير بعد الحفظ.
- **cfg واحد.** مثل `cargo check`، يرى الفهرس هدفًا واحدًا ومجموعة ميزات واحدة؛ الشيفرة خلف
  `#[cfg]` أو ميزات أخرى لا تُفهرَس. اخترها عبر `ridx index --all-features` / `--target`.
- **Windows** غير مدعوم بعد.
- **المكتبة القياسية**: يعمل التنقّل والمرور إلى `std` و`core` و`alloc`؛ ويُبحث عن المراجع إليها داخل مساحة
  العمل فقط، ولا يمكن إعادة تسميتها.
- يُحتسب `vec![]` مرجعًا إلى `Vec`؛ ولا يحتسبه rust-analyzer.

---

## التطوير

لا يتجاوز أي ملف مصدر 400 سطر.

</div>

<div dir="ltr">

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

</div>

<div dir="rtl">

## الاستشهاد

إذا استخدمت ridx في بحث أو توثيق، فيُرجى الاستشهاد بالإصدار المؤرشف على Zenodo:
[10.5281/zenodo.24379816](https://doi.org/10.5281/zenodo.24379816) للمشروع (يشير إلى أحدث إصدار)، و[10.5281/zenodo.24415350](https://doi.org/10.5281/zenodo.24415350) للإصدار 0.3.11 تحديدًا.
يحتوي [`CITATION.cff`](../CITATION.cff) على البيانات الوصفية التي يستخدمها زر "Cite this repository" في GitHub.

</div>

<div dir="ltr">

```bibtex
@software{panasenko_2026_24415350,
  author    = {Panasenko, Alexander},
  title     = {Rust Intelligence (ridx) — a Rust language server answering from a compiler-built on-disk index},
  month     = nov,
  year      = 2026,
  publisher = {Zenodo},
  version   = {0.3.11},
  doi       = {10.5281/zenodo.24415350},
  url       = {https://doi.org/10.5281/zenodo.24415350},
}
```

</div>

<div dir="rtl">

## الترخيص

مرخّص بموجب أيٍّ من:

- Apache License, Version 2.0 ([LICENSE-APACHE](../LICENSE-APACHE) أو http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](../LICENSE-MIT) أو http://opensource.org/licenses/MIT)

حسب اختيارك.

</div>
