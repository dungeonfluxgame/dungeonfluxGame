# Dependency license admission policy

Date: 2026-10-01  
Task/attempt: `B-G01-D03/a1`  
Input revision: `502f2940e32aa8b1fc71a182c4859189152cb3d2`

## Decision

Keep the project declaration `MIT` as-is. Admit a third-party dependency only after recording its exact package/version, source identity, declared SPDX expression or license-file evidence, selected license branch (where there is a choice), applicable text/notice/source obligations, and the evidence location/hash in the release inventory. Each supported target/build profile must point to that same pinned inventory. A lockfile or manifest is an inventory input, not proof of legal approval.

A route may be marked approved only for an explicitly selected and reviewed permissive license branch whose required notices, attribution, license text, source offer or corresponding source distribution, and other stated conditions have an owner and delivery location. This policy does not interpret a license for a particular distribution as legal advice. A copyleft or network-copyleft branch, custom/LicenseRef or exception not separately reviewed, ambiguous/non-SPDX declaration, missing declaration and missing license text, or unavailable provenance stays **HOLD** until a separate recorded rights decision approves that exact package, use, distribution and obligations. A package on hold blocks the distribution that contains it; it is not silently omitted from the inventory.

Parse valid SPDX expressions with a conforming parser and the pinned SPDX license/exception list. Never identify licenses by substring. `OR` means alternatives: select and record one branch, then satisfy that branch's terms; it does not automatically approve the expression. `AND` means cumulative requirements: every branch must be approved and every obligation completed. Parentheses and `WITH` exceptions retain their SPDX meaning and require the relevant exception text/evidence. Unknown syntax, parser errors, and SPDX identifiers outside the reviewed allowlist are HOLD. A human must review exact license texts and repository notices; metadata alone cannot establish that the declared terms match the shipped files.

The allowlist is a decision input controlled by the coordinator/release rights owner, not inferred from the word “permissive.” The starting route candidates are MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC and Zlib, each subject to exact-text and obligation review. Unlicense, BSL, Unicode-3.0, LLVM-exception combinations, and any other unfamiliar identifier are unresolved until individually added by a documented decision. SPDX validity is not policy approval.

This is dependency-source licensing only. Game rules/books/catalog rights and source citations, user imports, generated text/images/audio, provider terms, and commercial/service rights require separate asset/content/provider rights records. Dependency approval does not grant those rights or constitute full G01 completion, commercial clearance, or legal advice. The roadmap separately makes complete rights evidence a release gate.

## Current locked inventory observation

At input revision `502f294`, root `Cargo.toml` declares edition 2024, resolver 3, project `MIT`, six workspace members and a `[patch.crates-io]` redirect of `h2` to `crates/df-rpc-bridge/vendor/h2`. `Cargo.lock` pins `h2 0.4.19` as a path package without a registry source/checksum; the vendored normalized manifest declares `MIT`, repository `https://github.com/hyperium/h2`, and version `0.4.19`. The vendored `LICENSE` contains the 2017 h2 authors' MIT notice. This is a license-text observation, not source lineage proof. The original upstream revision, vendoring date, local patch diff, retained upstream notices, and build/distribution packaging obligations were not established here. Until those are recorded and the patch reviewed against upstream, `h2` provenance is HOLD. Preserve its LICENSE in distributed source/artifacts and include its attribution with the third-party notices when it is admitted.

The root checkout has no top-level `LICENSE` file. The Cargo package declarations inherit workspace `MIT`; this discrepancy requires a project-level follow-up to decide whether a root license file should be added. No file was added in this decision-only task.

The bounded locked/offline Cargo 1.98.1 metadata observations resolved 173 package records in the unfiltered graph (including six workspace packages and target/optional/dev dependency records), 153 for `aarch64-apple-darwin`, and 143 for `wasm32-unknown-unknown`. The platform-filtered commands used Cargo metadata defaults; they are a snapshot of the current lockfile and currently selected features, not an exhaustive feature-matrix build. All three report no package with an empty/missing `license` and no `license_file`; this does not mean all records are approved. Observed declarations include ordinary MIT/Apache alternatives, cumulative expressions such as `MIT AND BSD-3-Clause` and `(MIT OR Apache-2.0) AND Unicode-3.0`, two `MIT OR Apache-2.0 OR LGPL-2.1-or-later` expressions, five `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`, `Unlicense OR MIT`, `Zlib`, BSL, and legacy slash forms such as `MIT/Apache-2.0` and `Apache-2.0 / MIT`. For the two LGPL alternatives, the permissive MIT or Apache branch can be selected only if the inventory records the exact selected branch and its notices; the LGPL branch remains unselected/HOLD. Slash forms are not silently normalized. The Unicode cumulative term, exception branch, BSL and Unlicense need individual review. Thus the current inventory has unresolved records and no release profile is certified by this metadata run.

Cargo metadata with `--filter-platform aarch64-apple-darwin` and `--filter-platform wasm32-unknown-unknown` records separate target-filtered current package graphs, but it does not enumerate license texts, inspect source-file headers, confirm actual vendored diffs, or prove shipped notices/source. It also does not cover unselected features or establish that every resolved package is compiled into a final artifact. Native and WASM closures must be refreshed for every supported feature profile when G01 freezes those profiles; every included dependency must then pass this policy. No artifact build or full dependency license audit is claimed.

## Audit results (2026-10-25)

The target-specific audit used the current locked workspace, Cargo 1.98.1, offline metadata and `cargo tree --offline -e normal,features -p <root> --target <target>`. It covers normal dependency closures for `df-server`, `df-runner`, and native `df-tools` on `aarch64-pc-windows-msvc`; `df-account-app` and the shared TV/phone `df-tools` client on `wasm32-unknown-unknown`. The inventory records the profile, package/version/source, declared expression, deterministic selected branch and reason, resolved features, status, cached license-text paths and hashes, and NOTICE paths and hashes. Its five profiles contain 588 rows: 63 runner, 154 server, 235 native tools, 27 account WASM, and 109 TV/phone WASM. Feature trees are retained under `artifacts/tmp/lanes/LICAUDIT/`. These are target and root-package normal dependency sets, not every feature combination or dev-dependency graph. The older risk-review inventory found 377 package records and no GPL/AGPL/MPL declaration; that observation does not certify these current distributions.

The generator writes five profile-specific `THIRD_PARTY_NOTICES-*.txt` files. It deduplicates identical license texts by SHA-256, records package attribution, appends source `NOTICE` files, and lists every profile package with its declaration, selected branch and audit status so missing or held packages remain visible. These are notice inputs, not release approval or proof that packaging includes them. The inventory currently has 462 allow-list candidates, 19 selected-branch candidates with non-allowlisted terms, 72 rows with missing license text, and 35 rows held for unresolved declarations (23 Unicode-3.0 rows and 12 legacy slash rows). Five third-party packages lack an identified text in the cached source: `gloo-timers 0.3.0`, `opentelemetry 0.31.0`, `opentelemetry-proto 0.31.0`, `opentelemetry_sdk 0.31.0`, and `tonic-prost 0.14.6`; workspace packages also lack a root `LICENSE`. No target profile is cleared. The project owner must decide the project's MIT distribution/license-file treatment and resolve missing third-party texts before distribution.

Selected but owner-unreviewed non-allowlisted terms include `Unlicense OR MIT` (MIT selected), `0BSD OR MIT OR Apache-2.0` (MIT selected), BSL-1.0 alternatives for `ryu 1.0.23` and `whoami 1.6.1`, and unselected MIT-0 in `aws-lc-sys 0.45.0`. The inventory records choices and rationale; selection is not approval. `r-efi` and LGPL expressions are absent from these five current normal closures; if a supported target or feature profile later includes `r-efi`, it needs a fresh exact-version text and obligation review. Unicode-3.0 packages remain HOLD, including `unicode-ident`'s cumulative `(MIT OR Apache-2.0) AND Unicode-3.0`; MIT/Apache cannot discharge its Unicode term. Legacy slash declarations including `MIT/Apache-2.0` and `Apache-2.0 / MIT` remain HOLD and were not normalized. `quick-error 2.0.1` has the legacy `MIT/Apache-2.0` declaration and is therefore held in the current server and TV/phone closures.

The vendored h2 already has `crates/df-rpc-bridge/vendor/h2/PATCH.md`, so no second provenance file was created. It records upstream `hyperium/h2` 0.4.19, commit `d57d1b852fec9dda6d42d3454502006d52104da8`, published archive SHA-256 `ef8e5e5a340588f4452631496976cf8636d4a7ecf600239fdc27615d2530bc16`, omission of upstream examples/tests/benches/lockfile, and the local changes: `web-time = 1.1.0` plus `web_time::Instant` substitutions in `src/proto/streams/recv.rs` and `src/proto/streams/stream.rs` for wasm32. The registry source directory and `.crate` archive for h2 0.4.19 were not present in this Cargo cache, so an independent byte-level diff was not possible. The cache comparison and physical-device suspend-time qualification remain open review items.

`development/scripts/check_dependency_licenses.py` is a standard-library CI check. It regenerates current target closures offline and compares package/profile/version/expression/selected-branch/status records to the checked-in inventory; a newly resolved package outside the allow-list, an unreviewed inventory addition, a changed declaration/choice, or a stale row fails. Its fail-closed expression reader accepts operators and allow-list identifiers needed for this inventory; it is not a conforming SPDX parser or a substitute for the pinned SPDX identifier list required above. Held syntax must be reviewed with that parser before an owner admits it. Branch selection among allowed alternatives is deterministic and recorded; generated selection does not grant legal approval or discharge notice/attribution obligations. Run `python development/scripts/check_dependency_licenses.py`; use `--generate` only when deliberately refreshing the inventory and notices for review.

Remaining decisions for the owner/release rights reviewer: choose and document the project's own MIT distribution/license-file treatment; obtain or identify exact missing texts for the workspace and Apache-2.0 packages; resolve Unicode-3.0 and malformed legacy expressions with source evidence and an SPDX-aware review; confirm the runtime target set and feature profiles before release; compare h2 with the exact registry archive when available; review exact notice text/attribution and wire each generated notice into server/browser packaging. This audit changes none of those rights decisions.

## Bounded executable decision example

The following standard-library-only model illustrates the admission rule. It is not an SPDX parser or production admission tool. `Approved` carries explicit booleans for reviewed source provenance and completed obligations; cumulative terms hold if either record is incomplete, OR selection returns the selected branch index, and any held route blocks distribution.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Evidence {
    provenance_recorded: bool,
    obligations_recorded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    Approved(Evidence),
    Hold(&'static str),
}

impl Route {
    fn admitted(self) -> bool {
        matches!(
            self,
            Self::Approved(Evidence {
                provenance_recorded: true,
                obligations_recorded: true
            })
        )
    }
}

fn and(parts: &[Route]) -> Route {
    if parts.iter().all(|part| part.admitted()) {
        Route::Approved(Evidence {
            provenance_recorded: true,
            obligations_recorded: true,
        })
    } else {
        Route::Hold("at least one obligation or provenance record is unresolved")
    }
}

fn choose_or(selected: usize, branches: &[Route]) -> (usize, Route) {
    (
        selected,
        branches
            .get(selected)
            .copied()
            .unwrap_or(Route::Hold("invalid selection")),
    )
}

fn distribution_allowed(routes: &[Route]) -> bool {
    routes.iter().all(|route| route.admitted())
}

fn main() {
    let mit = Route::Approved(Evidence {
        provenance_recorded: true,
        obligations_recorded: true,
    });
    let missing_provenance = Route::Approved(Evidence {
        provenance_recorded: false,
        obligations_recorded: true,
    });
    let missing_obligation = Route::Approved(Evidence {
        provenance_recorded: true,
        obligations_recorded: false,
    });
    let copyleft = Route::Hold("copyleft route needs separate recorded decision");
    let custom = Route::Hold("custom or unknown terms");
    let missing = Route::Hold("missing license evidence");

    assert_eq!(choose_or(1, &[copyleft, mit]), (1, mit));
    assert_eq!(and(&[mit, mit]), mit);
    assert_eq!(
        and(&[mit, custom]),
        Route::Hold("at least one obligation or provenance record is unresolved")
    );
    assert!(!and(&[missing_provenance]).admitted());
    assert!(!and(&[missing_obligation]).admitted());
    assert_eq!(choose_or(0, &[custom, copyleft]), (0, custom));
    assert_eq!(choose_or(2, &[mit]), (2, Route::Hold("invalid selection")));
    assert_eq!(missing, Route::Hold("missing license evidence"));
    assert!(!distribution_allowed(&[mit, missing]));
    assert!(distribution_allowed(&[mit]));
}
```

This exact example was formatted, compiled, and run from a temporary source file with the cached Rust 1.98.1 `rustfmt` and `rustc`; all assertions passed. Its explicit `Route` values model evidence already reviewed by a human; it does not prove that strings are valid SPDX, validate provenance, infer obligations, or certify any real dependency.

## Source references and evidence

- Rust toolchain and targets: [planning/toolchain-targets.md](toolchain-targets.md); pin is stable Rust 1.98.1, edition 2024, Cargo.lock committed.
- Integration/release gates and prohibition on dependency installation by planning: [planning/implementation-roadmap.md](implementation-roadmap.md), especially “Service release stages and expensive prerequisites.”
- Source/catalog/commercial rights remain separate: [planning/rules-support.md](rules-support.md), “Paid rights and novice adjudication.”
- Current source inputs: `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `crates/df-rpc-bridge/vendor/h2/Cargo.toml`, and `crates/df-rpc-bridge/vendor/h2/LICENSE`, all at input revision above. SHA-256: Cargo.toml `a8f2d7317d401a25eb5ef4315dd0b42184f8f947d4cf82b055fd64c4656a5f05`; Cargo.lock `c29d0c7cd8d60e7d63aa16803713de3bd8145ed21e614b50121c0d0f55fdd947`; vendored h2 manifest `94e50ea2e4efc0f1a5c206ca0a79ba7cee99ff73416761db4cafb478aafc3818`; h2 LICENSE `b21623012e6c453d944b0342c515b631cfcbf30704c2621b291526b69c10724d`.
- Offline metadata JSON and resource output: `/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/B-G01-D03-a1/metadata.json`, `metadata-native.json`, `metadata-wasm.json`, and matching `*-time.txt` files. Cargo exited 0 for each; the unfiltered command took 0.29 seconds with maximum resident set 75,677,696 bytes. Target-filtered graphs have 153 native and 143 WASM package records. No downloads or builds were run. Metadata output is retained in the assigned scratch root, outside Git.
- Model source/binary: `/Users/earlcameron/Desktop/dungeonflux/artifacts/tmp/B-G01-D03-a1/decision.rs` and `decision`; the source is also reproduced above. `rustfmt +1.98.1` and `rustc +1.98.1 --edition=2024` completed successfully; executable exit 0. Temporary Rust source was not added to the repository.
- SPDX license-expression concepts: [SPDX License List](https://spdx.org/licenses/) and [SPDX 2.3 License Expressions](https://spdx.github.io/spdx-spec/v2.3/SPDX-license-expressions/) (accessed 2026-10-01). SPDX defines `OR` as a choice and `AND` as simultaneous requirements; the cited older specification supports the operators' stated distinction, while this policy does not rely on it for a particular license's legal effect.
- Cargo package license metadata and nonstandard `license-file`: [Cargo manifest reference](https://doc.rust-lang.org/cargo/reference/manifest.html) and [Cargo metadata reference](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html) (accessed 2026-10-01). Declared metadata is package-provided; this policy therefore requires text/provenance review as well.

## Original acceptance criteria

- approved routes
- The named outcome has actual source/build-bound evidence; unsupported, pending, failed and unperformed checks remain explicit.

## Original verification criteria

- Freeze the cited source decision and a bounded contract example; compare approved routes. Retain decision, alternatives and unresolved facts.
- Exact executable commands: TBD at G01 and scoped prerequisite resolution; this planned procedure is not a claim that Rust/browser/provider checks ran.

## Remaining work and check boundary

The policy route is selected, and the bounded source/model evidence is retained. Unresolved package terms and the `h2` source lineage/patch review keep distribution admission open. A complete SPDX-aware package/text/notice audit, exact native/WASM closures, and coordinator approval of resulting routes remain outstanding. This source decision is not an integrated acceptance or full G01 result.
