# js-intl decision map

Ownership: `crates/amazing-js/src/builtins_intl*.rs` (ECMA-402 `Intl.*`
constructors, BCP-47/UTS-35 locale machinery, CLDR-derived data tables).

## JSI-6: matcher-aware locale resolution, BCP-47 getCanonicalLocales rewrite

Commit `ebc41bc1`. `intl_lookup`/`intl_lookup_one` split "lookup" (pure BCP-47
truncation) from "best fit" (CLDR-likely-subtags-aware) locale matching, and
every constructor threads its own `localeMatcher` option through. Rewrote
`intl_canonical_tag` (UTS-35 §3.2.1 canonicalization) into
`intl_canonicalize_language_id`/`intl_canonicalize_transformed_extension`/
`intl_canonicalize_unicode_extension`, generated `builtins_intl_bcp47_data.rs`
(BCP-47 keyword/type registry from `cldr-bcp47/bcp47/*.json`) and extended
`INTL_REGION_ALIASES`/added `INTL_SUBDIVISION_ALIASES` from
`cldr-core/supplemental/aliases.json`. intl402: 1276 -> 1286 files, zero
losses. Two cross-lane blockers filed (Proxy missing; a pre-existing
`amazing-regex` resource-limit bug in `testIntl.js`'s self-check regex).

## JSI-7: close the intl402 gap, cluster and fix in bulk

Base: `dev` tip `bf11ad70`. The coordinator's brief authorized porting from
`lucid-softworks/lumen` @ `f6c9860d44fa52249cfd647122edea184d32c863` (MIT) per
AGENTS.md's "Porting from other engines" section. **Nothing was actually
copied from lumen** -- `intl/tags.rs` was read in full (confirms this lane's
own from-scratch JSI-6 design independently converged on a near-identical
architecture) and `intl/data.rs`/`cldr_likely.rs` were skimmed, but every fix
below came from direct CLDR-data/spec debugging once the actual failures were
clustered, not from lumen's implementation. No attribution header or
`THIRD_PARTY_NOTICES.md` row is needed as a result.

### Clustering finding: intl402's "weakest area" framing is mostly Temporal

On `bf11ad70`, intl402 is 1309/3357 (2048 failing). Of those, **1892 (92%)**
are under `intl402/Temporal/*`, entirely gated by js-stdlib's
`Temporal.Instant`/`Temporal.ZonedDateTime` and non-"iso8601"-calendar gaps
(already logged in `questions.md`), not by any Intl-constructor bug. This
lane's genuinely addressable surface was the remaining ~156 files, clustered
by constructor then by root cause (`questions.md` has the full breakdown).

### Fixes landed (21 files gained, zero losses; before/after measured below)

- **`intrinsic_constructors` registration** (`builtins_intl.rs`:
  `intl_register_intrinsic`, called from every `install_intl_*`): every
  `Intl.X` service constructor is now registered the same way
  `capture_intrinsic_constructors` (`interp_realms.rs`) registers direct
  globals (Array, RegExp, ...), so `constructor_fallback_prototype` can find
  it. Fixes `GetPrototypeFromConstructor`'s cross-realm fallback
  (`Reflect.construct(Intl.X, [], newTarget)` with a non-object
  `newTarget.prototype`) for all 10 `proto-from-ctor-realm.js` files. Also
  used by `intl_construct_service` (`builtins_intl_locale_methods.rs`) and
  `intl_chain_construct`/`intl_collator_ctor`'s no-`new` path
  (`builtins_intl.rs`/`builtins_intl_collator.rs`) to reach the %Collator%/
  %NumberFormat%/%DateTimeFormat% intrinsic directly instead of a mutable
  `Get(Intl, name)` lookup, fixing all 4 `taint-Intl-*.js` files
  (`testIntl.js`'s `taintDataProperty` replaces `Intl.Collator` etc. with a
  throwing/undefined accessor; these bridges must be unaffected).
- **`intl_unicode_keyword`** (`builtins_intl.rs`): now truncates at a
  private-use `-x-` singleton before searching for `-u-`, so
  `"de-x-u-co-phonebk"`'s private-use sequence is never misread as a real
  Unicode extension (`Collator/unicode-ext-seq-in-private-tag.js`).
  Pre-existing bug, not a JSI-7 regression, just newly observable once "co"
  extraction was added.
- **Collator** (`builtins_intl_collator.rs`): reads and resolves the `co`
  Unicode-extension keyword against a per-locale `intl_collation_supported`
  table (matches `Intl/supportedValuesOf/collations-accepted-by-Collator.js`'s
  own documented locale/collation mapping), fixing `resolved-collation-
  unicode-extensions-and-options.js`. German phonebook collation (`co-phonebk`)
  now expands ä/ö/ü to "ae"/"oe"/"ue" during key generation
  (`non-normative-phonebook.js`). `ignorePunctuation` now defaults to `true`
  for Thai (`ignorePunctuation-default.js`) and punctuation-stripping now
  applies consistently to the accent/case tiebreak comparisons, not just the
  primary key (`ignorePunctuation.js`). "case"-only sensitivity now strips
  diacritics before comparing case, matching UCA's case-only level
  (`non-normative-sensitivity.js`).
- **NumberFormat data** (`builtins_intl_unit_data.rs`, `builtins_intl_data.rs`):
  fixed ja/zh-Hant kilometer/mile-per-hour "long" unit prefixes (had a
  spurious trailing space); added `en-IN` (Indian digit grouping with English
  symbols) and `pt-PT` (narrow-no-break-space grouping, suffix currency
  placement) locale rows -- **appended at the end**, never inserted mid-list,
  because `INTL_CURRENCY_DATA`'s per-locale `symbols`/`narrow_symbols`/`names`
  arrays are aligned to `INTL_NUMBER_DATA`'s row *position*, not its `locale`
  field; a mid-list insert silently shifts every later locale's currency
  symbol/name by one slot (caught as a regression, then fixed properly). The
  three lookup call sites (`builtins_intl_number.rs` x2,
  `builtins_intl_display.rs` x1) were also hardened with `.get(index)` +
  fallback to index 0 instead of a hard `[index]`, so a locale appended after
  a currency's array was generated degrades to English symbols/names instead
  of panicking.
- **PluralRules data** (`builtins_intl_data.rs`): added `gv` (Manx) and `sl`
  (Slovenian) cardinal plural rules and `IntlNumberData` rows (needed purely
  so `intl_lookup` can resolve to them at all) --
  `plural-categories-order.js`.

### Cross-lane findings (not fixed here; full detail in `questions.md`)

- `amazing-regex`: the `RangeError: RegExp resource limit` bug from JSI-6
  turns out to block **33** intl402 files, not ~17 -- every one of them
  verified independently correct when run directly. Highest-leverage single
  fix available to any lane on intl402's raw number.
- `interp.rs`/`interp_realms.rs` (js-lang/js-binary/js-runtime, as owned):
  the exact `proto-from-ctor-realm.js` pattern this ticket fixed for Intl
  also fails for RegExp, Set, SharedArrayBuffer, String, SuppressedError,
  TypedArrayConstructors (x6), WeakMap, WeakRef, WeakSet (~19 files) --
  likely the same "constructor never registered in `intrinsic_constructors`"
  root cause.
- `interp.rs` (js-runtime): `Interpreter::to_number` uses ToPrimitive hint
  "default" (`self.primitive(...)`) instead of "number"
  (`self.primitive_number_hint(...)`, already defined a few lines above it),
  an engine-wide ECMA-262 7.1.4 ToNumber bug, not Intl-specific.
- `interp.rs` (js-lang/js-runtime/js-vm): generators and tagged templates are
  unimplemented at the language level, blocking 6 intl402 files whose test
  source uses them directly (unrelated to any Intl algorithm).

### Gate

Base `bf11ad70` + this ticket's commit. venus-003, `scripts/test262.py --only
intl402`:

- Before: 1309/3357 pass.
- After: 1330/3357 pass. Gained 21, lost 0 (verified via full pass-set diff,
  not just the aggregate count).

Workspace: `cargo nextest run --workspace --locked --no-fail-fast` — 3381
tests run, 3381 passed (3 slow), 17 skipped, 0 failed. `cargo test --workspace
--locked --doc` — every crate `ok`, including `amazing_js` (1 passed) and
`amazing_html` (1 passed).
