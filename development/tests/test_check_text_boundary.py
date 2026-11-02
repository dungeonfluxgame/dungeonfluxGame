"""Adversarial tests for development/check-text-boundary.py.

Each fixture is written to a temporary directory and scanned with the script's library
functions. Run from the repository root with:
    python -m unittest development/tests/test_check_text_boundary.py -v
"""
from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest import mock

SCRIPT = Path(__file__).resolve().parent.parent / "check-text-boundary.py"
REPO_ROOT = SCRIPT.parent.parent
_spec = importlib.util.spec_from_file_location("check_text_boundary", SCRIPT)
guard = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(guard)

FIXTURE_RS = "crates/fixture/src/lib.rs"


def rust(src, rel=FIXTURE_RS):
    """Classes and symbols found in one Rust fixture, as sorted (class, symbol) pairs."""
    return sorted((cls, sym) for _, cls, sym in guard.analyze_rust(rel, textwrap.dedent(src)))


def tree(files):
    """Write fixture files into a temporary root and evaluate the guard over them."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        for rel, body in files.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(textwrap.dedent(body), encoding="utf-8")
        return guard.evaluate(root)


def run_cli(argv):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        code = guard.main(argv)
    return code, out.getvalue()


class RustAliasAndPathTests(unittest.TestCase):
    def test_alias_construction_flags(self):
        found = rust("""
            use df_providers::NativeTextProvider as P;
            fn build() { let _p = P::new(config); }
        """)
        self.assertIn(("import", "NativeTextProvider"), found)
        self.assertIn(("call", "NativeTextProvider"), found)

    def test_multiline_qualified_paths_flag(self):
        found = rust("""
            fn build() {
                let p = df_providers::registered_text_provider(
                    route, config, key, limits,
                );
            }
            fn other() {
                let q = text_response
                    ::NativeTextProvider
                    ::new(route);
            }
        """)
        self.assertIn(("call", "registered_text_provider"), found)
        self.assertIn(("call", "NativeTextProvider"), found)

    def test_factory_reexport_alias_flags(self):
        found = rust("""
            pub use text_response::registered_text_provider as make_text;
            pub use crate::provider::NativePlateJudge;
        """)
        self.assertIn(("import", "registered_text_provider"), found)
        self.assertIn(("import", "NativePlateJudge"), found)

    def test_type_alias_of_banned_name_flags_use_site(self):
        found = rust("""
            type Judge = df_providers::NativePlateJudge;
            fn f(j: Judge) -> Judge { j }
        """)
        self.assertIn(("identifier", "NativePlateJudge"), found)

    def test_line_and_block_comment_mentions_do_not_flag(self):
        self.assertEqual(rust("""
            // NativeTextProvider was retired; registered_text_provider(x) is gone.
            /* SystemOneChoiceProvider::new(x) in a block comment */
            fn f() {}
        """), [])

    def test_string_mention_is_not_code(self):
        self.assertEqual(rust("""
            fn f() -> &'static str { "NativeTextProvider::new(x) registered_text_provider(y)" }
        """), [])

    def test_longer_identifiers_do_not_match(self):
        self.assertEqual(rust("""
            fn f() {
                let c = NativeTextProviderConfig::default();
                let _x: NativeTextProviderConfigs = c;
            }
        """), [])

    def test_codex_identifier_other_than_text_call_does_not_flag(self):
        self.assertEqual(rust("""
            use crate::codex_cli;
            fn f() {
                let ok = codex_cli::available();
                let text = "hi";
                let n = text.len();
            }
        """), [])

    def test_codex_cli_text_call_flags_qualified_and_aliased(self):
        self.assertIn(("call", "codex_cli::text"),
                      rust("fn f() { let r = codex_cli::text(&task).await; }"))
        aliased = rust("""
            use crate::codex_cli::text as run;
            fn f() { run(&task); }
        """)
        self.assertIn(("import", "codex_cli::text"), aliased)
        self.assertIn(("call", "codex_cli::text"), aliased)

    def test_submit_flags_only_where_native_provider_is_named(self):
        self.assertIn(("call", "NativeTextProvider.submit"), rust("""
            use df_providers::NativeTextProvider;
            fn f(p: &Provider) { p.submit(req); }
        """))
        self.assertEqual(rust("fn f(form: &Form) { form.submit(req); }"), [])


class RustNestingAndMacroTests(unittest.TestCase):
    def test_cfg_test_nested_module_still_flags_calls(self):
        found = rust("""
            #[cfg(test)]
            mod tests {
                mod nested {
                    fn t() { let _ = NativeTextProvider::new(x); }
                }
            }
        """)
        self.assertIn(("call", "NativeTextProvider"), found)

    def test_macro_body_still_flags(self):
        found = rust("""
            macro_rules! make {
                () => { df_providers::registered_text_provider(route) };
            }
        """)
        self.assertIn(("call", "registered_text_provider"), found)


class RustLiteralTests(unittest.TestCase):
    def test_endpoint_string_flags_as_literal_evidence(self):
        self.assertEqual(rust('const E: &str = "https://api.openai.com/v1/responses";'),
                         [("literal", "/v1/responses")])

    def test_endpoint_in_cfg_test_is_test_literal(self):
        self.assertEqual(rust("""
            #[cfg(test)]
            mod tests { const E: &str = "/v1/chat/completions"; }
        """), [("test-literal", "/v1/chat/completions")])

    def test_endpoint_in_test_attribute_fn_is_test_literal(self):
        self.assertEqual(rust("""
            #[test]
            fn fixture() { let _ = "/v1/systemone"; }
        """), [("test-literal", "/v1/systemone")])

    def test_endpoint_in_test_file_is_test_literal(self):
        found = guard.analyze_rust("crates/fixture/tests/provider.rs",
                                   'const E: &str = "/v1/responses";')
        self.assertEqual([(c, s) for _, c, s in found], [("test-literal", "/v1/responses")])

    def test_endpoint_in_doc_comment_is_test_literal(self):
        self.assertEqual(rust("""
            /// Posts to /v1/systemone for classification.
            fn f() {}
        """), [("test-literal", "/v1/systemone")])

    def test_endpoint_in_plain_line_comment_is_ignored(self):
        self.assertEqual(rust("// formerly /v1/responses\nfn f() {}"), [])

    def test_single_literal_plus_join_flags(self):
        self.assertEqual(rust('fn u(base: &str) -> String { base.to_string() + "/v1/responses" }'),
                         [("literal", "/v1/responses")])

    def test_concat_macro_split_endpoint_flags(self):
        self.assertEqual(rust('const E: &str = concat!("/v1/", "responses");'),
                         [("literal", "/v1/responses")])

    def test_separate_constants_in_format_are_flagged_as_split_literal(self):
        self.assertEqual(rust("""
            const A: &str = "/v1/";
            const B: &str = "responses";
            fn u() -> String { format!("{A}{B}") }
        """), [("split-literal", "/v1/responses")])

    def test_unrelated_oauth_payment_and_media_hosts_do_not_flag(self):
        self.assertEqual(rust("""
            const T: &str = "https://oauth2.googleapis.com/token";
            const P: &str = "https://api.stripe.com/v1/charges";
            const M: &str = "https://api.openai.com/v1/images/edits";
            const B: &str = "https://api.openai.com/v1/embeddings";
        """), [])


class SplitLiteralTests(unittest.TestCase):
    def split(self, body):
        return [sym for cls, sym in rust(body) if cls == "split-literal"]

    def test_positional_format_over_constants_flags(self):
        self.assertEqual(self.split("""
            const HEAD: &str = "/v1/";
            static TAIL: &str = "chat/completions";
            fn u() -> String { format!("{}{}", HEAD, TAIL) }
        """), ["/v1/chat/completions"])

    def test_split_at_any_boundary_flags(self):
        self.assertEqual(self.split("""
            const A: &str = "/v1/res";
            const B: &str = "ponses";
            fn u() -> String { format!("{A}{B}") }
        """), ["/v1/responses"])

    def test_three_way_split_with_literal_middle_flags(self):
        self.assertEqual(self.split("""
            const A: &str = "/v1";
            const C: &str = "completions";
            fn u() -> String { format!("{A}/chat/{C}") }
        """), ["/v1/chat/completions"])

    def test_plus_over_constants_flags(self):
        self.assertEqual(self.split("""
            const A: &str = "/v1/";
            const B: &str = "systemone";
            fn u(base: &str) -> String { base.to_string() + A + B }
        """), ["/v1/systemone"])

    def test_let_bound_fragments_and_to_string_flag(self):
        self.assertEqual(self.split("""
            fn u() -> String {
                let prefix = "/v1/";
                let leaf = "responses";
                prefix.to_string() + &leaf.to_string()
            }
        """), ["/v1/responses"])

    def test_push_str_across_statements_flags(self):
        self.assertEqual(self.split("""
            const A: &str = "/v1/";
            const B: &str = "responses";
            fn u() -> String {
                let mut url = String::new();
                url.push_str(A);
                url.push_str(B);
                url
            }
        """), ["/v1/responses"])

    def test_join_chain_and_array_join_flag(self):
        self.assertEqual(self.split("""
            const A: &str = "/v1";
            const B: &str = "responses";
            fn u(base: &Url) { base.join(A).join(B); }
        """), ["/v1/responses"])
        self.assertEqual(self.split("""
            const A: &str = "/v1/";
            const B: &str = "responses";
            fn u() -> String { [A, B].join("") }
        """), ["/v1/responses"])

    def test_constants_in_different_functions_do_not_flag(self):
        self.assertEqual(self.split("""
            const A: &str = "/v1/";
            const B: &str = "responses";
            fn one() -> &'static str { A }
            fn two() -> &'static str { B }
        """), [])

    def test_unrelated_v1_constant_alone_does_not_flag(self):
        self.assertEqual(rust("""
            const API: &str = "/v1/";
            fn u(name: &str) -> String { format!("{API}{name}") }
        """), [])

    def test_responses_constant_alone_does_not_flag(self):
        self.assertEqual(rust("""
            const KEY: &str = "responses";
            fn u(base: &str) -> String { format!("{base}/{KEY}") }
        """), [])

    def test_v1_prefix_with_unrelated_leaf_does_not_flag(self):
        self.assertEqual(rust("""
            const A: &str = "/v1/";
            const B: &str = "images/edits";
            fn u() -> String { format!("{A}{B}") }
        """), [])

    def test_known_limit_cross_file_constants_are_not_expanded(self):
        self.assertEqual(rust("""
            use crate::routes::{HEAD, TAIL};
            fn u() -> String { format!("{HEAD}{TAIL}") }
        """), [])

    def test_split_literal_in_tree_fails_unless_allowlisted(self):
        src = """
            const A: &str = "/v1/";
            const B: &str = "responses";
            fn u() -> String { format!("{A}{B}") }
        """
        result = tree({FIXTURE_RS: src})
        self.assertEqual([(v["class"], v["symbol"]) for v in result["violations"]],
                         [("split-literal", "/v1/responses")])


class ScriptTests(unittest.TestCase):
    def test_python_requests_post_to_responses_flags(self):
        result = tree({"development/probe.py": """
            import requests
            requests.post("https://api.openai.com/v1/responses", json={})
        """})
        self.assertEqual([(v["class"], v["symbol"]) for v in result["violations"]],
                         [("egress", "requests.post")])

    def test_python_requests_post_without_endpoint_does_not_flag(self):
        self.assertEqual(guard.analyze_script("development/probe.py", textwrap.dedent("""
            import requests
            requests.post("https://oauth2.googleapis.com/token", data={})
        """)), [])

    def test_python_urlopen_to_chat_completions_flags(self):
        hits = guard.analyze_script("development/probe.py", textwrap.dedent("""
            from urllib.request import urlopen
            urlopen("https://api.openai.com/v1/chat/completions")
        """))
        self.assertEqual([(c, s) for _, c, s in hits], [("egress", "urlopen")])

    def test_shell_curl_to_responses_flags(self):
        hits = guard.analyze_script("development/probe.sh",
                                    'curl -s https://api.openai.com/v1/responses \\\n  -d @body.json\n')
        self.assertEqual([(c, s) for _, c, s in hits], [("egress", "curl")])

    def test_powershell_invoke_restmethod_to_responses_flags(self):
        hits = guard.analyze_script("development/probe.ps1",
                                    'Invoke-RestMethod -Uri "https://api.openai.com/v1/responses" -Method Post\n')
        self.assertEqual([(c, s) for _, c, s in hits], [("egress", "invoke-restmethod")])

    def test_curl_without_text_endpoint_does_not_flag(self):
        self.assertEqual(guard.analyze_script("development/probe.sh",
                                              "curl -fsSL https://example.com/install.sh\n"), [])

    def test_guard_script_does_not_flag_itself(self):
        self.assertEqual(guard.analyze_script("development/check-text-boundary.py",
                                              SCRIPT.read_text(encoding="utf-8")), [])


class EgressAndProcessTests(unittest.TestCase):
    def test_indirect_command_spawn_in_codex_module_flags(self):
        self.assertIn(("process", "Command::new"), rust("""
            use crate::codex_cli;
            fn spawn() {
                let bin = codex_cli::binary();
                let c = Command::new(bin);
            }
        """))

    def test_command_naming_codex_flags(self):
        self.assertIn(("process", "Command::new"),
                      rust('fn spawn() { let c = Command::new("codex"); }'))

    def test_generic_command_spawn_does_not_flag(self):
        self.assertEqual(rust("""
            use std::process::Command;
            fn g(exe: &str) { Command::new("git").arg("status"); let _ = Command::new(exe); }
        """), [])

    def test_variable_alias_of_codex_flags(self):
        for body in ('let bin = "codex"; let c = Command::new(bin);',
                     'let bin = "codex"; let c = Command::new(&bin);',
                     'let bin = env::var("CODEX_BIN").unwrap_or_default(); Command::new(bin);',
                     'let a = "/usr/bin/codex"; let b = a; Command::new(b.as_str());',
                     'const EXE: &str = "curl"; Command::new(EXE).arg(url);'):
            self.assertIn(("process", "Command::new"), rust(f"fn spawn() {{ {body} }}"), body)

    def test_command_alias_import_flags(self):
        self.assertIn(("process", "Command::new"), rust("""
            use std::process::Command as Cmd;
            fn spawn() { let exe = "codex"; Cmd::new(exe); }
        """))

    def test_variable_alias_of_unrelated_programs_does_not_flag(self):
        for body in ('let bin = "ffmpeg"; Command::new(bin);',
                     'let bin = "git"; Command::new(&bin).arg("status");',
                     'let bin = env::var("FFMPEG_BIN").unwrap_or_default(); Command::new(bin);',
                     'let flags = ["DF_CODEX_MODE"]; let bin = "git"; Command::new(bin);'):
            self.assertEqual(rust(f"fn spawn() {{ {body} }}"), [], body)

    def test_powershell_variable_flags_only_with_http_cmdlet(self):
        http = rust("""
            fn spawn() {
                let shell = "powershell";
                Command::new(shell).arg("Invoke-RestMethod https://example.com");
            }
        """)
        self.assertIn(("process", "Command::new"), http)
        self.assertEqual(rust("""
            fn spawn() { let shell = "powershell"; Command::new(shell).arg("Get-ChildItem"); }
        """), [])

    def test_skipped_directories_are_not_scanned(self):
        result = tree({
            "crates/fixture/src/archive/old.rs": "fn f() { NativeTextProvider::new(x); }",
            "crates/fixture/src/target/gen.rs": "fn f() { NativeTextProvider::new(x); }",
            "crates/dungeonflux.old/src/lib.rs": "fn f() { NativeTextProvider::new(x); }",
        })
        self.assertEqual(result["total_hits"], 0)
        self.assertTrue(result["ok"])


def manifest(name, deps="", dev="", target=""):
    body = f'[package]\nname = "{name}"\nversion = "0.1.0"\n\n[dependencies]\n{deps}\n'
    if dev:
        body += f"\n[dev-dependencies]\n{dev}\n"
    if target:
        body += f'\n[target.\'cfg(not(target_arch = "wasm32"))\'.dependencies]\n{target}\n'
    return body


def cargo(crates):
    """Violations from the Cargo check over fixture manifests {crate name: manifest text}."""
    result = tree({f"crates/{name}/Cargo.toml": text for name, text in crates.items()})
    return result, [(v["class"], v["symbol"]) for v in result["violations"]]


REQWEST = 'reqwest = { version = "=0.13.5", default-features = false }'


class CargoReachabilityTests(unittest.TestCase):
    def test_df_ai_adding_reqwest_fails(self):
        result, found = cargo({"df-ai": manifest("df-ai", REQWEST)})
        self.assertEqual(found, [("cargo", "df-ai -> reqwest")])
        self.assertFalse(result["ok"])
        self.assertEqual(result["violations"][0]["path"], "crates/df-ai/Cargo.toml")

    def test_provider_api_may_not_depend_on_providers_or_process_crates(self):
        _, found = cargo({"df-provider-api": manifest(
            "df-provider-api", 'df-providers = { path = "../df-providers" }\nasync-process = "2"\nureq = "2"')})
        self.assertEqual(sorted(found), [("cargo", "df-provider-api -> async-process"),
                                         ("cargo", "df-provider-api -> df-providers"),
                                         ("cargo", "df-provider-api -> ureq")])

    def test_tokio_process_feature_and_renamed_dependency_fail(self):
        _, found = cargo({"df-ai": manifest(
            "df-ai", 'tokio = { version = "1", features = ["rt", "process"] }\nhttp_client = { package = "hyper", version = "1" }')})
        self.assertEqual(sorted(found), [("cargo", "df-ai -> hyper"), ("cargo", "df-ai -> tokio")])

    def test_target_specific_dependency_fails(self):
        _, found = cargo({"df-ai": manifest("df-ai", target=REQWEST)})
        self.assertEqual(found, [("cargo", "df-ai -> reqwest")])

    def test_reachability_through_a_workspace_crate_fails(self):
        _, found = cargo({
            "df-ai": manifest("df-ai", 'df-helper = { path = "../df-helper" }'),
            "df-helper": manifest("df-helper", 'df-google = { path = "../df-google" }'),
            "df-google": manifest("df-google", REQWEST),
        })
        self.assertEqual(found, [("cargo", "df-ai -> reqwest")])

    def test_dev_dependencies_and_plain_workspace_deps_are_allowed(self):
        result, found = cargo({
            "df-ai": manifest("df-ai", 'df-types = { path = "../df-types" }\nsha2 = "0.10"', dev=REQWEST),
            "df-types": manifest("df-types"),
            "df-provider-api": manifest("df-provider-api", dev='df-ai = { path = "../df-ai" }'),
        })
        self.assertEqual(found, [])
        self.assertTrue(result["ok"], result)

    def test_baseline_reqwest_owners_pass(self):
        result, found = cargo({name: manifest(name, REQWEST) for name in sorted(guard.REQWEST_BASELINE)})
        self.assertEqual(found, [])
        self.assertEqual([n for n in result["stale"] if "baseline" in n], [])

    def test_new_reqwest_crate_fails_even_when_named_owner_is_not_in_baseline(self):
        _, found = cargo({"df-launcher": manifest("df-launcher", REQWEST),
                          "df-session": manifest("df-session", REQWEST)})
        self.assertEqual(found, [("cargo", "df-launcher -> reqwest"), ("cargo", "df-session -> reqwest")])
        result, _ = cargo({"df-launcher": manifest("df-launcher", REQWEST)})
        self.assertIn("baseline", result["violations"][0]["why"])

    def test_dev_dependency_reqwest_in_new_crate_fails(self):
        _, found = cargo({"df-session": manifest("df-session", dev=REQWEST)})
        self.assertEqual(found, [("cargo", "df-session -> reqwest")])

    def test_baseline_crate_that_dropped_reqwest_is_a_stale_note(self):
        result, found = cargo({"df-hosted": manifest("df-hosted")})
        self.assertEqual(found, [])
        self.assertTrue(result["ok"])
        self.assertEqual(len([n for n in result["stale"] if "baseline" in n]), 1)

    def test_unparseable_manifest_fails(self):
        _, found = cargo({"df-ai": "[package\nname ="})
        self.assertEqual(found, [("cargo", "Cargo.toml")])


class AllowlistTests(unittest.TestCase):
    KEY = (FIXTURE_RS, "call", "NativeTextProvider")
    VIOLATING = "fn f() { NativeTextProvider::new(a); NativeTextProvider::new(b); }"

    def test_recorded_count_is_allowed_and_counted_by_phase(self):
        with mock.patch.dict(guard.ALLOWLIST, {self.KEY: {"count": 2, "phase": "T2", "reason": "fixture"}}, clear=True), \
                mock.patch.dict(guard.PHASE_CEILINGS, {"T2": 2}, clear=True):
            result = tree({FIXTURE_RS: self.VIOLATING})
        self.assertTrue(result["ok"], result)
        self.assertEqual(result["allowlisted_by_phase"], {"T2": 2})

    def test_added_hit_above_recorded_count_fails(self):
        with mock.patch.dict(guard.ALLOWLIST, {self.KEY: {"count": 1, "phase": "T2", "reason": "fixture"}}, clear=True), \
                mock.patch.dict(guard.PHASE_CEILINGS, {"T2": 1}, clear=True):
            result = tree({FIXTURE_RS: self.VIOLATING})
        self.assertFalse(result["ok"])
        self.assertEqual(len(result["violations"]), 2)
        self.assertIn("exceeds recorded count", result["violations"][0]["why"])

    def test_phase_ceiling_blocks_new_entry_beyond_recorded_total(self):
        allow = {
            (FIXTURE_RS, "call", "NativeTextProvider"): {"count": 1, "phase": "T2", "reason": "fixture"},
            (FIXTURE_RS, "call", "registered_text_provider"): {"count": 1, "phase": "T2", "reason": "fixture"},
        }
        src = "fn f() { NativeTextProvider::new(a); registered_text_provider(b); }"
        with mock.patch.dict(guard.ALLOWLIST, allow, clear=True), \
                mock.patch.dict(guard.PHASE_CEILINGS, {"T2": 1}, clear=True):
            result = tree({FIXTURE_RS: src})
        self.assertFalse(result["ok"])
        self.assertTrue(any("exceed ceiling" in e for e in result["errors"]))

    def test_unlisted_file_hit_fails(self):
        with mock.patch.dict(guard.ALLOWLIST, {}, clear=True), mock.patch.dict(guard.PHASE_CEILINGS, {}, clear=True):
            result = tree({FIXTURE_RS: self.VIOLATING})
        self.assertFalse(result["ok"])
        self.assertEqual(result["violations"][0]["why"], "no allowlist entry")

    def test_stale_entry_is_reported_without_failing(self):
        with mock.patch.dict(guard.ALLOWLIST, {self.KEY: {"count": 2, "phase": "T2", "reason": "fixture"}}, clear=True), \
                mock.patch.dict(guard.PHASE_CEILINGS, {"T2": 2}, clear=True):
            result = tree({FIXTURE_RS: "fn f() { NativeTextProvider::new(a); }"})
        self.assertTrue(result["ok"])
        self.assertEqual(len(result["stale"]), 1)


class CommandLineTests(unittest.TestCase):
    def test_violating_tree_exits_one(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / FIXTURE_RS
            path.parent.mkdir(parents=True)
            path.write_text("fn f() { NativeTextProvider::new(a); }", encoding="utf-8")
            code, out = run_cli(["--root", tmp])
        self.assertEqual(code, 1)
        self.assertIn("RESULT: FAIL", out)

    def test_clean_tree_exits_zero_and_json_is_parseable(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / FIXTURE_RS
            path.parent.mkdir(parents=True)
            path.write_text("fn f() -> u8 { 1 }", encoding="utf-8")
            # An empty allowlist, so no entry is left over in a tree that has nothing to match.
            with mock.patch.object(guard, "ALLOWLIST", {}):
                code, out = run_cli(["--root", tmp])
                code_json, out_json = run_cli(["--json", "--root", tmp])
        self.assertEqual(code, 0)
        self.assertIn("RESULT: PASS", out)
        self.assertEqual(code_json, 0)
        self.assertTrue(json.loads(out_json)["ok"])

    def test_missing_root_is_usage_error(self):
        with contextlib.redirect_stderr(io.StringIO()):
            code, _ = run_cli(["--root", str(REPO_ROOT / "no-such-directory")])
        self.assertEqual(code, 2)

    def test_repository_tree_has_zero_unaccounted_violations(self):
        result = guard.evaluate(REPO_ROOT)
        self.assertEqual(result["violations"], [])
        self.assertEqual(result["errors"], [])
        self.assertTrue(result["ok"])

    def test_the_repository_allowlist_has_no_leftover_entries(self):
        self.assertEqual(guard.evaluate(REPO_ROOT)["stale"], [])

    def test_the_allowlist_holds_only_permanent_phases_and_no_migration_placeholders(self):
        phases = {entry["phase"] for entry in guard.ALLOWLIST.values()}
        self.assertEqual(
            phases, {"backend", "provider-crate", "image-route-config", "fixture"}
        )
        self.assertEqual(set(guard.PHASE_CEILINGS), phases)

    def test_a_leftover_entry_fails_the_command_line_run(self):
        with tempfile.TemporaryDirectory() as root:
            code, _ = run_cli(["--root", root])
        self.assertEqual(code, 1)


if __name__ == "__main__":
    unittest.main()
