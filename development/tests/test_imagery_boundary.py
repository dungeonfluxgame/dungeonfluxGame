#!/usr/bin/env python3
"""Failure-focused unittest for development/check-imagery-boundary.py.

Run from the workspace root (no package install; shell disabled for this
contributor, so root executes): `python development/tests/test_imagery_boundary.py`
or `python -m unittest discover -s development/tests -p "test_*.py" -v`.
Root runs the suite; contributors must not claim results they cannot execute.
"""
from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

CHECKER = Path(__file__).resolve().parents[1] / "check-imagery-boundary.py"


def load_checker():
    spec = importlib.util.spec_from_file_location("check_imagery_boundary", CHECKER)
    if spec is None or spec.loader is None:
        raise AssertionError("cannot load check-imagery-boundary.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules["check_imagery_boundary"] = module
    spec.loader.exec_module(module)
    return module


CB = load_checker()

BYPASS = "let p = df_providers::NativeImageEditProvider::new(cfg, key, deadline)?;"

STARTUP = "crates/df-tools/src/gameplay/creator_portrait_startup.rs"

# Realistic transitional content for the allowlisted startup file: all six
# budgeted symbols with their callsite fingerprints inside one owning
# function. Synthetic minimal lines without an owning function or without
# these fingerprints would (correctly) trip stale-allowance detection.
STARTUP_FULL = (
    "fn start() {\n"
    "match selection {\n"
    "ImageProvider::OpenAi => ImageRoute::OpenAi(\n"
    "df_providers::NativeImageEditProvider::new(\n"
    "df_providers::ImageEditConfig::live(), key, deadline).unwrap()),\n"
    "ImageProvider::Flux => ImageRoute::Flux(\n"
    "df_providers::NativeFalImageProvider::new(\n"
    "df_providers::FalImageConfig::live(), fal, deadline).unwrap()),\n"
    "}\n"
    "let alt_settings = settings_for(true)?;\n"
    "let route = ImageRoute::Bfl(\n"
    "df_providers::NativeBflImageProvider::new(\n"
    "df_providers::BflImageConfig::live(), &key, deadline).unwrap());\n"
    "}\n"
)


def write_tree(files: dict[str, str]) -> Path:
    root = Path(tempfile.mkdtemp(prefix="imagery-boundary-"))
    (root / "crates" / "df-imagery" / "src").mkdir(parents=True)
    (root / "crates" / "df-imagery" / "Cargo.toml").write_text(
        '[package]\nname = "df-imagery"\n[dependencies]\n', encoding="utf-8"
    )
    (root / "crates" / "df-imagery" / "src" / "lib.rs").write_text(
        "mod backends;\n", encoding="utf-8"
    )
    for rel, text in files.items():
        target = root / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")
    return root


def kinds(findings) -> set[str]:
    return {f.kind for f in findings}


FIXTURE_LEGACY_ROWS = (
    CB.LegacyRow(
        "crates/df-tools/src/gameplay/imagery.rs",
        "NativeFalImageProvider::new",
        1,
        "FalImageConfig::live",
        "phase 3 consumer migration, then phase 5 deletion",
    ),
    CB.LegacyRow(
        "crates/df-tools/src/gameplay/imagery.rs",
        "NativeImageEditProvider::new",
        1,
        "ImageEditConfig::live",
        "phase 3 consumer migration, then phase 5 deletion",
    ),
    CB.LegacyRow(
        "crates/df-tools/src/gameplay/creator_portrait_startup.rs",
        "NativeImageEditProvider::new",
        1,
        "ImageEditConfig::live",
        "phase 3 creator chain migration, then phase 5 deletion",
    ),
    CB.LegacyRow(
        "crates/df-tools/src/gameplay/creator_portrait_startup.rs",
        "NativeFalImageProvider::new",
        1,
        "FalImageConfig::live",
        "phase 3 creator chain migration, then phase 5 deletion",
    ),
    CB.LegacyRow(
        "crates/df-tools/src/gameplay/creator_portrait_startup.rs",
        "NativeBflImageProvider::new",
        1,
        "BflImageConfig::live",
        "phase 3 creator chain migration, then phase 5 deletion",
    ),
    CB.LegacyRow(
        "crates/df-tools/src/gameplay/creator_portrait_startup.rs",
        "ImageRoute::OpenAi",
        1,
        "ImageProvider::OpenAi",
        "phase 3 creator chain migration, then phase 5 deletion",
    ),
    CB.LegacyRow(
        "crates/df-tools/src/gameplay/creator_portrait_startup.rs",
        "ImageRoute::Flux",
        1,
        "ImageProvider::Flux",
        "phase 3 creator chain migration, then phase 5 deletion",
    ),
    CB.LegacyRow(
        "crates/df-tools/src/gameplay/creator_portrait_startup.rs",
        "ImageRoute::Bfl",
        1,
        "alt_settings",
        "phase 3 creator chain migration, then phase 5 deletion",
    ),
    CB.LegacyRow(
        "crates/df-tools/src/gameplay/character_clips.rs",
        "NativeImageEditProvider::new",
        1,
        "IMAGE_EDIT_TIMEOUT",
        "phase 3 frame migration with the prior-attempt fence, then phase 5 deletion",
    ),
    CB.LegacyRow(
        "crates/df-tools/src/gameplay/dev_control/direct.rs",
        "NativeImageEditProvider::new",
        1,
        "loopback_fixture",
        "phase 3 dev-fixture migration to the central fake-backend seam",
    ),
    CB.LegacyRow(
        "crates/df-tools/src/gameplay/imagery/figure_masks.rs",
        "NativeImageEditProvider::new",
        1,
        "for_test",
        "phase 3 test-helper migration to the central fake-backend seam",
    ),
    CB.LegacyRow(
        "crates/df-tools/src/gameplay/imagery_live_tests.rs",
        "NativeFalImageProvider::new",
        1,
        "FluxPerson",
        "opt-in live acceptance (#[ignore]); migrate to service-level live acceptance",
    ),
)

class LegacyFixtureCase(unittest.TestCase):
    def setUp(self):
        self.saved_budget = CB.LEGACY_BUDGET
        CB.LEGACY_BUDGET = {(row.path, row.symbol): row for row in FIXTURE_LEGACY_ROWS}

    def tearDown(self):
        CB.LEGACY_BUDGET = self.saved_budget

class StripTests(LegacyFixtureCase):
    def test_line_comment_is_not_a_violation(self):
        self.assertNotIn("NativeImageEditProvider", CB.strip_rust("// NativeImageEditProvider::new\nlet x = 1;\n"))

    def test_block_comment_is_not_a_violation(self):
        src = "/* NativeFalImageProvider::new */\nlet x = 1;\n"
        self.assertNotIn("NativeFalImageProvider", CB.strip_rust(src))

    def test_nested_block_comment_is_not_a_violation(self):
        src = "/* outer /* NativeBflImageProvider::new */ still comment */\nlet x = 1;\n"
        self.assertNotIn("NativeBflImageProvider", CB.strip_rust(src))

    def test_string_literal_is_not_a_violation(self):
        src = 'let s = "NativeImageEditProvider::new";\n'
        self.assertNotIn("NativeImageEditProvider", CB.strip_rust(src))

    def test_raw_string_is_not_a_violation(self):
        src = 'let s = r#"NativeFalImageProvider::new"#;\n'
        self.assertNotIn("NativeFalImageProvider", CB.strip_rust(src))

    def test_lifetime_is_not_a_char_literal_crash(self):
        src = "fn f<'a>(x: &'a str) {}\n"
        self.assertIn("&'a", CB.strip_rust(src))

    def test_line_numbers_survive_stripping(self):
        src = '// ignored\n"str"\n' + BYPASS + "\n"
        stripped = CB.strip_rust(src).splitlines()
        self.assertIn("NativeImageEditProvider", stripped[2])

    def test_escaped_newline_keeps_line_numbers(self):
        src = 'let s = "a\\\n b";\n' + BYPASS + "\n"
        stripped = CB.strip_rust(src).splitlines()
        self.assertEqual(len(stripped), 3)
        self.assertIn("NativeImageEditProvider", stripped[2])


class ScanTests(LegacyFixtureCase):
    def test_qualified_call_in_new_file_fails(self):
        root = write_tree({"crates/df-tools/src/gameplay/new_feature.rs": BYPASS + "\n"})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))

    def test_submit_image_dispatch_fails(self):
        root = write_tree({"crates/df-tools/src/gameplay/sneaky.rs": "let j = client.submit_image(&req).await;\n"})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.symbol == "submit_image dispatch" for f in findings))

    def test_no_directory_blanket_sibling_of_allowlist_still_fails(self):
        root = write_tree({
            STARTUP: STARTUP_FULL,
            "crates/df-tools/src/gameplay/other_consumer.rs": BYPASS + "\n",
        })
        findings = CB.run(root, strict=False)
        by_path = {f.path for f in findings if f.kind == "bypass"}
        self.assertIn("crates/df-tools/src/gameplay/other_consumer.rs", by_path)
        self.assertNotIn(STARTUP, by_path)

    def test_legacy_allowlist_passes_default_but_fails_strict(self):
        root = write_tree({STARTUP: STARTUP_FULL})
        findings = CB.run(root, strict=False)
        self.assertEqual(kinds(findings), {"legacy"})
        self.assertEqual(len([f for f in findings if f.kind == "legacy"]), 6)
        strict_findings = CB.run(root, strict=True)
        self.assertTrue(any(f.kind == "legacy" for f in strict_findings))
        self.assertEqual(CB.main(["--root", str(root)]), 0)
        self.assertEqual(CB.main(["--root", str(root), "--strict"]), 1)

    def test_test_loopback_exemption_is_symbol_narrow(self):
        root = write_tree({
            "crates/df-tools/src/gameplay/imagery_tests.rs":
                'let c = ImageEditConfig::loopback_fixture(addr).unwrap();\n',
            "crates/df-tools/src/gameplay/sneaky_tests.rs": BYPASS + "\n",
        })
        findings = CB.run(root, strict=False)
        self.assertFalse(any("imagery_tests" in f.path for f in findings))
        self.assertTrue(any("sneaky_tests" in f.path and f.kind == "bypass" for f in findings))

    def test_alias_evasion_limit_is_disclosed(self):
        # Documented limitation: a bare `MyEdit::new` with no `use ... as`
        # binding in the file is not resolved.
        root = write_tree({"crates/df-tools/src/gameplay/aliased.rs": "let p = MyEdit::new(cfg)?;\n"})
        findings = CB.run(root, strict=False)
        self.assertEqual(findings, [])

    def test_dependency_on_df_tools_fails(self):
        root = write_tree({})
        (root / "crates" / "df-imagery" / "Cargo.toml").write_text(
            '[package]\nname = "df-imagery"\n[dependencies]\ndf-tools = { path = "../df-tools" }\n',
            encoding="utf-8",
        )
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "dependency" for f in findings))

    def test_public_backends_export_fails(self):
        root = write_tree({})
        (root / "crates" / "df-imagery" / "src" / "lib.rs").write_text(
            "pub mod backends;\n", encoding="utf-8"
        )
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "export" for f in findings))

    def test_missing_crates_dir_is_usage_error(self):
        with self.assertRaises(RuntimeError):
            CB.run(Path(tempfile.mkdtemp(prefix="imagery-no-crates-")), strict=False)


class MultilineScanTests(LegacyFixtureCase):
    def test_multiline_qualified_constructor_fails(self):
        src = (
            "let p = df_providers::NativeImageEditProvider\n"
            "    ::\n"
            "    new(cfg, key, deadline)?;\n"
        )
        root = write_tree({"crates/df-tools/src/gameplay/split.rs": src})
        findings = CB.run(root, strict=False)
        hit = [f for f in findings if f.kind == "bypass" and "NativeImageEditProvider" in f.symbol]
        self.assertEqual(len(hit), 1)
        self.assertEqual(hit[0].line, 1)

    def test_comment_separated_constructor_fails(self):
        src = "let p = NativeImageEditProvider /* pick */ :: /* make */ new(cfg, key, deadline)?;\n"
        root = write_tree({"crates/df-tools/src/gameplay/split.rs": src})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))

    def test_multiline_submit_image_dispatch_fails(self):
        src = "let j = client\n    .submit_image\n    (&req).await;\n"
        root = write_tree({"crates/df-tools/src/gameplay/split.rs": src})
        findings = CB.run(root, strict=False)
        hit = [f for f in findings if f.symbol == "submit_image dispatch"]
        self.assertEqual(len(hit), 1)
        self.assertEqual(hit[0].line, 2)

    def test_escaped_newline_reports_the_real_line(self):
        src = 'let s = "a\\\n b";\n' + BYPASS + "\n"
        root = write_tree({"crates/df-tools/src/gameplay/split.rs": src})
        findings = CB.run(root, strict=False)
        hit = [f for f in findings if f.kind == "bypass" and "NativeImageEditProvider" in f.symbol]
        self.assertEqual(len(hit), 1)
        self.assertEqual(hit[0].line, 3)


class FixtureAssociationTests(LegacyFixtureCase):
    LIVE = "let p = df_providers::NativeImageEditProvider::new(live_cfg, key, t)?;\n"

    def test_same_line_unrelated_loopback_identifier_still_fails(self):
        src = "let loopback_fixture = false; " + self.LIVE
        root = write_tree({"crates/df-tools/src/gameplay/sneaky_tests.rs": src})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))

    def test_previous_line_unrelated_loopback_identifier_still_fails(self):
        src = "let loopback_fixture = false;\n" + self.LIVE
        root = write_tree({"crates/df-tools/src/gameplay/sneaky_tests.rs": src})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))

    def test_live_constructor_in_test_file_without_fixture_fails(self):
        root = write_tree({"crates/df-tools/src/gameplay/sneaky_tests.rs": self.LIVE})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))

    def test_multiline_fixture_backed_constructor_passes(self):
        src = (
            "let p = df_providers::NativeImageEditProvider::new(\n"
            "    ImageEditConfig::loopback_fixture(addr).unwrap(),\n"
            '    "test-credential",\n'
            "    deadline,\n"
            ").unwrap();\n"
        )
        root = write_tree({"crates/df-tools/src/gameplay/imagery_tests.rs": src})
        self.assertEqual(CB.run(root, strict=False), [])

    def test_variable_bound_fixture_config_passes(self):
        src = (
            "let config = ImageEditConfig::loopback_fixture(addr).unwrap();\n"
            'let route = ImageRoute::OpenAi(NativeImageEditProvider::new(config, "k", t).unwrap());\n'
        )
        root = write_tree({"crates/df-tools/src/gameplay/cost_tests.rs": src})
        self.assertEqual(CB.run(root, strict=False), [])

    def test_provider_self_tests_are_exempt(self):
        root = write_tree({
            "crates/df-providers/tests/image_edit.rs":
                "let p = NativeImageEditProvider::new(ImageEditConfig::live(), cred, t).unwrap();\n",
        })
        self.assertEqual(CB.run(root, strict=False), [])

    def test_provider_src_cfg_test_is_exempt(self):
        src = (
            "#[cfg(test)]\n"
            "mod tests {\n"
            "let p = NativeFalImageProvider::new(FalImageConfig::live(), k, t).unwrap();\n"
            "}\n"
        )
        root = write_tree({"crates/df-providers/src/fal_image.rs": src})
        self.assertEqual(CB.run(root, strict=False), [])

    def test_sound_video_factory_is_outside_image_boundary(self):
        src = (
            "pub fn registered_fal_media_provider(config: Config, credential: &str, limits: Limits) {\n"
            "    crate::NativeFalMediaProvider::new(config, credential, limits)\n"
            "}\n"
        )
        root = write_tree({"crates/df-providers/src/registry.rs": src})
        findings = CB.run(root, strict=False)
        self.assertEqual(findings, [])


class ImageRouteTests(LegacyFixtureCase):
    COST_LIKE = (
        "match route {\n"
        "    ImageRoute::OpenAi(_) => (Provider::OpenAi, model),\n"
        "    ImageRoute::Flux(_) if has_refs => (Provider::Fal, MULTI),\n"
        "    ImageRoute::Flux(_) => (Provider::Fal, ULTRA),\n"
        "    ImageRoute::Bfl(_) => (Provider::Bfl, BFL),\n"
        "}\n"
        "let on_flux = matches!(engine.provider, ImageRoute::Flux(_));\n"
    )

    def test_route_pattern_matching_is_not_execution(self):
        root = write_tree({"crates/df-tools/src/gameplay/cost.rs": self.COST_LIKE})
        self.assertEqual(CB.run(root, strict=False), [])

    def test_route_construction_fails(self):
        root = write_tree({"crates/df-tools/src/gameplay/sneaky.rs": "let r = ImageRoute::Flux(provider);\n"})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and f.symbol == "ImageRoute::Flux" for f in findings))

    def test_multiline_route_construction_fails(self):
        src = "let r = ImageRoute\n    ::\n    Bfl(\n        provider\n    );\n"
        root = write_tree({"crates/df-tools/src/gameplay/sneaky.rs": src})
        findings = CB.run(root, strict=False)
        hit = [f for f in findings if f.symbol == "ImageRoute::Bfl"]
        self.assertEqual(len(hit), 1)
        self.assertEqual(hit[0].line, 1)

    def test_route_construction_in_test_scope_passes(self):
        root = write_tree({"crates/df-tools/src/gameplay/route_tests.rs": "let r = ImageRoute::Flux(provider);\n"})
        self.assertEqual(CB.run(root, strict=False), [])


class SubmitImageTests(LegacyFixtureCase):
    def test_submit_image_definition_is_not_a_violation(self):
        src = "pub async fn submit_image(&self, req: &Req) -> Result<Job, FalMediaError> { job }\n"
        root = write_tree({"crates/df-providers/src/fal_media.rs": src})
        self.assertEqual(CB.run(root, strict=False), [])

    def test_submit_image_call_in_test_scope_passes(self):
        src = "let mut job = provider\n    .submit_image(&request, owner, future::pending())\n    .await?;\n"
        root = write_tree({"crates/df-providers/tests/fal_media.rs": src})
        self.assertEqual(CB.run(root, strict=False), [])


class PrivateBackendTests(LegacyFixtureCase):
    def test_private_backend_prefix_is_not_scanned(self):
        root = write_tree({
            "crates/df-imagery/src/backends/native.rs": BYPASS + "\n",
        })
        self.assertEqual(CB.run(root, strict=False), [])

    def test_df_imagery_outside_backends_is_still_checked(self):
        root = write_tree({
            "crates/df-imagery/src/service.rs": BYPASS + "\n",
        })
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))


class BudgetTests(LegacyFixtureCase):
    def test_added_call_in_allowlisted_file_fails(self):
        extra = (
            "let spare = df_providers::NativeImageEditProvider::new(\n"
            "    df_providers::ImageEditConfig::live(), other, deadline).unwrap();\n"
        )
        root = write_tree({STARTUP: STARTUP_FULL + extra})
        findings = CB.run(root, strict=False)
        hit = [f for f in findings if f.path == STARTUP and f.symbol == "NativeImageEditProvider::new"]
        self.assertEqual(len([f for f in hit if f.kind == "legacy"]), 1)
        self.assertEqual(len([f for f in hit if f.kind == "bypass"]), 1)

    def test_present_file_with_zero_occurrences_is_stale(self):
        root = write_tree({STARTUP: "let x = 1;\n"})
        findings = CB.run(root, strict=False)
        stale = [f for f in findings if f.kind == "stale" and f.path == STARTUP]
        self.assertEqual(len(stale), 6)
        self.assertFalse(any(f.path == STARTUP and f.kind == "legacy" for f in findings))

    def test_absent_files_are_not_stale(self):
        root = write_tree({"crates/df-tools/src/gameplay/new_feature.rs": BYPASS + "\n"})
        findings = CB.run(root, strict=False)
        self.assertFalse(any(f.kind == "stale" for f in findings))


class DependencyFormTests(LegacyFixtureCase):
    def _cargo_root(self, cargo_text: str) -> Path:
        root = write_tree({})
        (root / "crates" / "df-imagery" / "Cargo.toml").write_text(cargo_text, encoding="utf-8")
        return root

    def test_subtable_dependency_fails(self):
        root = self._cargo_root('[package]\nname = "df-imagery"\n[dependencies.df-tools]\nversion = "0.1"\n')
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "dependency" and "df-tools" in f.symbol for f in findings))

    def test_target_dependency_fails(self):
        root = self._cargo_root(
            '[package]\nname = "df-imagery"\n[target.\'cfg(windows)\'.dependencies]\ndf-session = "0.1"\n'
        )
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "dependency" and "df-session" in f.symbol for f in findings))

    def test_dev_dependency_fails(self):
        root = self._cargo_root('[package]\nname = "df-imagery"\n[dev-dependencies]\ndf-tools = "0.1"\n')
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "dependency" and "df-tools" in f.symbol for f in findings))

    def test_renamed_dependency_fails(self):
        root = self._cargo_root(
            '[package]\nname = "df-imagery"\n[dependencies]\nfoo = { package = "df-tools", version = "0.1" }\n'
        )
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "dependency" and "df-tools" in f.symbol for f in findings))


class ExportFormTests(LegacyFixtureCase):
    def _lib_root(self, lib_text: str) -> Path:
        root = write_tree({})
        (root / "crates" / "df-imagery" / "src" / "lib.rs").write_text(lib_text, encoding="utf-8")
        return root

    def test_qualified_reexport_fails(self):
        root = self._lib_root("pub use crate::backends::NativeImageEditProvider;\n")
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "export" for f in findings))

    def test_multiline_public_mod_fails(self):
        root = self._lib_root("pub\nmod backends;\n")
        findings = CB.run(root, strict=False)
        hit = [f for f in findings if f.kind == "export"]
        self.assertEqual(len(hit), 1)
        self.assertEqual(hit[0].line, 1)

    def test_restricted_mod_passes(self):
        root = self._lib_root("pub(crate) mod backends;\n")
        self.assertEqual(CB.run(root, strict=False), [])

    def test_private_mod_passes(self):
        root = self._lib_root("mod backends;\n")
        self.assertEqual(CB.run(root, strict=False), [])


class AliasTests(LegacyFixtureCase):
    def test_use_as_alias_constructor_fails(self):
        src = (
            "use df_providers::NativeImageEditProvider as MyEdit;\n"
            "let p = MyEdit::new(cfg, key, deadline)?;\n"
        )
        root = write_tree({"crates/df-tools/src/gameplay/sneaky.rs": src})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "alias" and "MyEdit" in f.symbol for f in findings))

    def test_grouped_second_alias_constructor_fails(self):
        src = (
            "use df_providers::{NativeImageEditProvider as Edit, NativeFalImageProvider as Flux};\n"
            "fn f(){let p=Flux::new(config,key,t);}\n"
        )
        root = write_tree({"crates/df-tools/src/gameplay/new_image.rs": src})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "alias" and "Flux" in f.symbol for f in findings))


class CfgScopeTests(LegacyFixtureCase):
    def test_production_after_closed_cfg_test_module_fails(self):
        src = "#[cfg(test)] mod tests {}\npub fn production(){let p=NativeImageEditProvider::new(live_config,key,t);}\n"
        root = write_tree({"crates/df-providers/src/brand_new.rs": src})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))

    def test_dispatch_after_closed_cfg_test_module_fails(self):
        src = "#[cfg(test)] mod tests {}\npub async fn production(){client.submit_image(&request).await;}\n"
        root = write_tree({"crates/df-tools/src/gameplay/new_image.rs": src})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.symbol == "submit_image dispatch" for f in findings))

    def test_constructor_inside_cfg_test_module_passes_with_loopback(self):
        src = (
            "#[cfg(test)]\n"
            "mod tests {\n"
            "fn f(){let c=ImageEditConfig::loopback_fixture(a).unwrap();\n"
            "let p=NativeImageEditProvider::new(c,k,t).unwrap();}\n"
            "}\n"
        )
        root = write_tree({"crates/df-tools/src/gameplay/cfg_scoped.rs": src})
        self.assertEqual(
            [f for f in CB.run(root, strict=False) if "NativeImageEditProvider" in f.symbol],
            [],
        )


class FixtureBindingTests(LegacyFixtureCase):
    def test_unrelated_fixture_call_with_different_binding_still_fails(self):
        src = (
            "let unused = ImageEditConfig::loopback_fixture(addr);\n"
            "let p = NativeImageEditProvider::new(live_config,key,t);\n"
        )
        root = write_tree({"crates/df-tools/src/gameplay/new_image_tests.rs": src})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))

    def test_reassigned_fixture_binding_still_fails(self):
        src = (
            "let config = ImageEditConfig::loopback_fixture(addr).unwrap();\n"
            "config = live_config;\n"
            "let p = NativeImageEditProvider::new(config,key,t);\n"
        )
        root = write_tree({"crates/df-tools/src/gameplay/reassign_tests.rs": src})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))


class RoutePayloadTests(LegacyFixtureCase):
    def test_underscore_prefixed_payload_is_construction_not_wildcard(self):
        root = write_tree(
            {"crates/df-tools/src/gameplay/new_image.rs": "fn f(_provider: Provider){let route=ImageRoute::Flux(_provider);}\n"}
        )
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and f.symbol == "ImageRoute::Flux" for f in findings))

    def test_named_match_pattern_is_inspection_not_construction(self):
        root = write_tree(
            {
                "crates/df-tools/src/gameplay/new_image.rs": "fn f(route: ImageRoute){match route { ImageRoute::Flux(provider) => inspect(provider), _=>{} }}\n"
            }
        )
        self.assertEqual(CB.run(root, strict=False), [])

    def test_let_destructuring_pattern_is_inspection(self):
        root = write_tree(
            {"crates/df-tools/src/gameplay/new_image.rs": "fn f(route: ImageRoute){let ImageRoute::Flux(inner) = route; inspect(inner);}\n"}
        )
        self.assertEqual(CB.run(root, strict=False), [])

    def test_matches_macro_with_named_binding_is_inspection(self):
        root = write_tree(
            {"crates/df-tools/src/gameplay/new_image.rs": "fn f(route: ImageRoute){let hit = matches!(route, ImageRoute::Flux(provider));}\n"}
        )
        self.assertEqual(CB.run(root, strict=False), [])


class IndirectExportTests(LegacyFixtureCase):
    def _lib_root(self, lib_text: str) -> Path:
        root = write_tree({})
        (root / "crates" / "df-imagery" / "src" / "lib.rs").write_text(lib_text, encoding="utf-8")
        return root

    def test_module_alias_reexport_fails(self):
        root = self._lib_root(
            "mod backends;\nuse crate::backends as implementation;\npub use implementation::Runner;\n"
        )
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "export" for f in findings))

    def test_grouped_item_alias_reexport_fails(self):
        root = self._lib_root(
            "mod backends;\nuse crate::backends::{Runner as Fast};\npub use Fast;\n"
        )
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "export" for f in findings))


class WorkspaceDependencyTests(LegacyFixtureCase):
    def test_workspace_inherited_renamed_dependency_fails(self):
        root = write_tree({})
        (root / "crates" / "df-imagery" / "Cargo.toml").write_text(
            '[package]\nname="df-imagery"\n[dependencies]\nhidden.workspace=true\n',
            encoding="utf-8",
        )
        (root / "Cargo.toml").write_text(
            '[workspace]\nmembers=["crates/df-imagery"]\n[workspace.dependencies]\nhidden={package="df-tools",path="crates/df-tools"}\n',
            encoding="utf-8",
        )
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "dependency" and "df-tools" in f.symbol for f in findings))

    def test_target_subtable_renamed_dependency_fails(self):
        root = write_tree({})
        (root / "crates" / "df-imagery" / "Cargo.toml").write_text(
            '[package]\nname="df-imagery"\n[target.\'cfg(windows)\'.dependencies.hidden]\npackage="df-session"\npath="../df-session"\n',
            encoding="utf-8",
        )
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "dependency" and "df-session" in f.symbol for f in findings))


class LegacyCallsiteTests(LegacyFixtureCase):
    def test_moved_callsite_cannot_donate_legacy_budget(self):
        root = write_tree(
            {
                "crates/df-tools/src/gameplay/imagery.rs": "fn configured(){let unused = FalImageConfig::live();}\nfn unrelated_new_bypass(){let p=NativeFalImageProvider::new(live_config,key,t);}\n"
            }
        )
        findings = CB.run(root, strict=False)
        hit = [f for f in findings if "NativeFalImageProvider" in f.symbol]
        self.assertTrue(any(f.kind == "bypass" for f in hit))
        self.assertFalse(any(f.kind == "legacy" for f in hit))
        self.assertTrue(any(f.kind == "stale" for f in findings))

    def test_image_factory_growth_reports_extra_occurrence(self):
        root = write_tree(
            {
                "crates/df-tools/src/gameplay/imagery.rs": "fn configured(){let old=NativeFalImageProvider::new(FalImageConfig::live(),k,t);}\nfn new_bypass(){let p=NativeFalImageProvider::new(c,k,t);}\n"
            }
        )
        findings = CB.run(root, strict=False)
        hit = [f for f in findings if "NativeFalImageProvider" in f.symbol]
        self.assertEqual(len([f for f in hit if f.kind == "legacy"]), 1)
        self.assertEqual(len([f for f in hit if f.kind == "bypass"]), 1)


class LiteralNoiseTests(LegacyFixtureCase):
    def test_byte_raw_string_and_block_comment_are_not_violations(self):
        root = write_tree(
            {
                "crates/df-tools/src/gameplay/new_image.rs": 'fn f(){let s=br##"NativeImageEditProvider::new(cfg) // fake"##; /* ImageRoute::Flux(x) */ }\n'
            }
        )
        self.assertEqual(CB.run(root, strict=False), [])


class MatchesValueTests(LegacyFixtureCase):
    def test_matches_macro_value_construction_is_flagged_while_pattern_passes(self):
        root = write_tree(
            {
                "crates/df-tools/src/gameplay/new_image.rs": "fn f(provider: Provider){let hit = matches!(ImageRoute::Flux(provider), ImageRoute::Flux(_));}\n"
            }
        )
        findings = CB.run(root, strict=False)
        hit = [f for f in findings if f.symbol == "ImageRoute::Flux" and f.kind == "bypass"]
        self.assertEqual(len(hit), 1)


class NextArmConstructionTests(LegacyFixtureCase):
    def test_both_arm_body_constructions_are_flagged_while_patterns_pass(self):
        root = write_tree(
            {
                "crates/df-tools/src/gameplay/new_image.rs": "fn f(){match x { ImageRoute::OpenAi(_) => ImageRoute::Flux(a), ImageRoute::Flux(_) => ImageRoute::OpenAi(b), } }\n"
            }
        )
        findings = CB.run(root, strict=False)
        flux = [f for f in findings if f.symbol == "ImageRoute::Flux" and f.kind == "bypass"]
        openai = [f for f in findings if f.symbol == "ImageRoute::OpenAi" and f.kind == "bypass"]
        self.assertEqual(len(flux), 1)
        self.assertEqual(len(openai), 1)


class CfgTestPrefixTests(LegacyFixtureCase):
    def test_cfg_test_mode_prefix_is_production_and_still_fails(self):
        root = write_tree(
            {
                "crates/df-tools/src/gameplay/new_image.rs": "#[cfg(test_mode)]\nfn f(){let p=NativeImageEditProvider::new(live_config,key,t);}\n"
            }
        )
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))


class FixtureArgumentTests(LegacyFixtureCase):
    def test_fixture_in_unrelated_argument_still_fails(self):
        root = write_tree(
            {
                "crates/df-tools/src/gameplay/sneaky_tests.rs": "fn f(){let p = NativeImageEditProvider::new(live_config, ImageEditConfig::loopback_fixture(addr), t);}\n"
            }
        )
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))

    def test_same_line_reassigned_fixture_binding_still_fails(self):
        root = write_tree(
            {
                "crates/df-tools/src/gameplay/reassign_tests.rs": "fn f(){let config = ImageEditConfig::loopback_fixture(addr).unwrap(); config = live_config; let p = NativeImageEditProvider::new(config, key, t);}\n"
            }
        )
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))


class ExactOffsetLegacyTests(LegacyFixtureCase):
    def test_same_line_sibling_function_cannot_donate_legacy_budget(self):
        root = write_tree(
            {
                "crates/df-tools/src/gameplay/imagery.rs": "fn configured(){let unused=FalImageConfig::live();} fn other(){let p=NativeFalImageProvider::new(c,k,t);}\n"
            }
        )
        findings = CB.run(root, strict=False)
        hit = [f for f in findings if "NativeFalImageProvider" in f.symbol]
        self.assertTrue(any(f.kind == "bypass" for f in hit))
        self.assertFalse(any(f.kind == "legacy" for f in hit))
        self.assertTrue(any(f.kind == "stale" for f in findings))

    def test_same_function_config_swap_is_bypass_and_stale(self):
        root = write_tree(
            {
                "crates/df-tools/src/gameplay/imagery.rs": "fn configured(){let unused = ImageEditConfig::live(); let p = NativeImageEditProvider::new(other_cfg, key, t);}\n"
            }
        )
        findings = CB.run(root, strict=False)
        hit = [f for f in findings if "NativeImageEditProvider" in f.symbol]
        self.assertTrue(any(f.kind == "bypass" for f in hit))
        self.assertFalse(any(f.kind == "legacy" for f in hit))
        self.assertTrue(any(f.kind == "stale" for f in findings))


class ChainedExportTests(LegacyFixtureCase):
    def test_chained_backend_module_alias_reexport_fails(self):
        root = write_tree({})
        (root / "crates" / "df-imagery" / "src" / "lib.rs").write_text(
            "mod backends;\nuse crate::backends as a;\nuse a as b;\npub use b::Runner;\n",
            encoding="utf-8",
        )
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "export" for f in findings))


class FreshInlineDecoyTests(LegacyFixtureCase):
    DECOY = "fn f(){let p=NativeImageEditProvider::new({ let ignored=ImageEditConfig::loopback_fixture(addr); live_config },key,t);}"

    def test_block_mentioning_loopback_but_returning_live_fails(self):
        root = write_tree({"crates/df-tools/src/gameplay/new_image_tests.rs": self.DECOY})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))
        strict_findings = CB.run(root, strict=True)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in strict_findings))

    def test_direct_loopback_inline_still_passes(self):
        src = "fn f(){let p=NativeImageEditProvider::new(ImageEditConfig::loopback_fixture(addr).unwrap(),key,t);}\n"
        root = write_tree({"crates/df-tools/src/gameplay/new_image_tests.rs": src})
        self.assertEqual(CB.run(root, strict=False), [])
        self.assertEqual(CB.run(root, strict=True), [])

    def test_simple_immutable_let_still_passes(self):
        src = "fn f(){let config=ImageEditConfig::loopback_fixture(addr).unwrap(); let p=NativeImageEditProvider::new(config,key,t);}\n"
        root = write_tree({"crates/df-tools/src/gameplay/new_image_tests.rs": src})
        self.assertEqual(CB.run(root, strict=False), [])
        self.assertEqual(CB.run(root, strict=True), [])


class FreshScopeShadowTests(LegacyFixtureCase):
    def test_for_shadow_still_fails(self):
        src = "fn f(){let config=ImageEditConfig::loopback_fixture(addr).unwrap(); for config in live_configs {let p=NativeImageEditProvider::new(config,key,t);}}\n"
        root = write_tree({"crates/df-tools/src/gameplay/new_image_tests.rs": src})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))

    def test_closure_shadow_still_fails(self):
        src = "fn f(){let config=ImageEditConfig::loopback_fixture(addr).unwrap(); let g=|config| NativeImageEditProvider::new(config,key,t);}\n"
        root = write_tree({"crates/df-tools/src/gameplay/new_image_tests.rs": src})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))

    def test_destructured_shadow_still_fails(self):
        src = "fn f(){let config=ImageEditConfig::loopback_fixture(addr).unwrap(); let (config,other)=live_pair; let p=NativeImageEditProvider::new(config,key,t);}\n"
        root = write_tree({"crates/df-tools/src/gameplay/new_image_tests.rs": src})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))

    def test_expired_inner_block_still_fails(self):
        src = "fn f(config:ImageEditConfig){ {let config=ImageEditConfig::loopback_fixture(addr).unwrap();} let p=NativeImageEditProvider::new(config,key,t);}\n"
        root = write_tree({"crates/df-tools/src/gameplay/new_image_tests.rs": src})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))


class FreshLegacyArgTests(LegacyFixtureCase):
    def test_legacy_decoy_in_credentials_is_bypass_and_stale(self):
        src = "fn changed(){let replacement=NativeImageEditProvider::new(other_cfg,{let unused=ImageEditConfig::live();key},t);let f=NativeFalImageProvider::new(FalImageConfig::live(),key,t);}\n"
        root = write_tree({"crates/df-tools/src/gameplay/imagery.rs": src})
        findings = CB.run(root, strict=False)
        edit = [f for f in findings if "NativeImageEditProvider" in f.symbol]
        self.assertTrue(any(f.kind == "bypass" for f in edit))
        self.assertFalse(any(f.kind == "legacy" for f in edit))
        fal = [f for f in findings if "NativeFalImageProvider" in f.symbol]
        self.assertTrue(any(f.kind == "legacy" for f in fal))
        self.assertTrue(any(f.kind == "stale" for f in findings))

    def test_timeout_in_third_argument_still_qualifies(self):
        src = "fn frames(){let p=NativeImageEditProvider::new(cfg,key,IMAGE_EDIT_TIMEOUT);}\n"
        root = write_tree({"crates/df-tools/src/gameplay/character_clips.rs": src})
        findings = CB.run(root, strict=False)
        hit = [f for f in findings if "NativeImageEditProvider" in f.symbol]
        self.assertTrue(any(f.kind == "legacy" for f in hit))
        self.assertTrue(any(f.kind == "legacy" for f in CB.run(root, strict=True)))


class ClientDependencyTests(LegacyFixtureCase):
    def _client_root(self, client_text: str, workspace_text: str | None = None) -> Path:
        root = write_tree({})
        target = root / "crates" / "df-client" / "Cargo.toml"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(client_text, encoding="utf-8")
        if workspace_text is not None:
            (root / "Cargo.toml").write_text(workspace_text, encoding="utf-8")
        return root

    def test_unconditional_client_imagery_dependency_fails(self):
        root = self._client_root('[package]\nname="df-client"\n[dependencies]\ndf-imagery={path="../df-imagery"}\n')
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "dependency" and "df-imagery" in f.symbol for f in findings))
        self.assertTrue(any(f.kind == "dependency" for f in CB.run(root, strict=True)))

    def test_renamed_client_imagery_dependency_fails(self):
        root = self._client_root('[package]\nname="df-client"\n[dependencies]\nimages={package="df-imagery",path="../df-imagery"}\n')
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "dependency" and "df-imagery" in f.symbol for f in findings))

    def test_workspace_inherited_client_imagery_dependency_fails(self):
        root = self._client_root(
            '[package]\nname="df-client"\n[dependencies]\nhidden.workspace=true\n',
            '[workspace]\nmembers=["crates/df-client"]\n[workspace.dependencies]\nhidden={package="df-imagery",path="crates/df-imagery"}\n',
        )
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "dependency" and "df-imagery" in f.symbol for f in findings))

    def test_wasm_target_client_imagery_dependency_fails(self):
        root = self._client_root('[package]\nname="df-client"\n[target.\'cfg(target_arch = "wasm32")\'.dependencies]\ndf-imagery={path="../df-imagery"}\n')
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "dependency" and "df-imagery" in f.symbol for f in findings))

    def test_native_only_client_imagery_dependency_passes(self):
        root = self._client_root('[package]\nname="df-client"\n[target.\'cfg(not(target_arch = "wasm32"))\'.dependencies]\ndf-imagery={path="../df-imagery"}\n')
        self.assertEqual(CB.run(root, strict=False), [])
        self.assertEqual(CB.run(root, strict=True), [])

    def test_absent_client_manifest_passes(self):
        self.assertEqual(CB.run(write_tree({}), strict=False), [])

    def test_native_composition_is_not_banned(self):
        root = write_tree({})
        target = root / "crates" / "df-tools" / "Cargo.toml"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text('[package]\nname="df-tools"\n[target.\'cfg(not(target_arch = "wasm32"))\'.dependencies]\ndf-imagery={path="../df-imagery"}\n', encoding="utf-8")
        self.assertEqual(CB.run(root, strict=False), [])
        self.assertEqual(CB.run(root, strict=True), [])


class WholeFixtureSuffixTests(LegacyFixtureCase):
    def test_inline_unwrap_or_live_fallback_fails(self):
        src = "fn f(){let p=NativeImageEditProvider::new(ImageEditConfig::loopback_fixture(addr).unwrap_or(ImageEditConfig::live()),key,t);}"
        root = write_tree({"crates/df-tools/src/gameplay/new_image_tests.rs": src})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))
        strict_findings = CB.run(root, strict=True)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in strict_findings))

    def test_bound_unwrap_or_live_fallback_fails(self):
        src = "fn f(){let config=ImageEditConfig::loopback_fixture(addr).unwrap_or(ImageEditConfig::live()); let p=NativeImageEditProvider::new(config,key,t);}"
        root = write_tree({"crates/df-tools/src/gameplay/new_image_tests.rs": src})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))
        strict_findings = CB.run(root, strict=True)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in strict_findings))

    def test_expect_suffix_still_passes(self):
        src = 'fn f(){let p=NativeImageEditProvider::new(ImageEditConfig::loopback_fixture(addr).expect("fixture"),key,t);}'
        root = write_tree({"crates/df-tools/src/gameplay/new_image_tests.rs": src})
        self.assertEqual(CB.run(root, strict=False), [])
        self.assertEqual(CB.run(root, strict=True), [])

    def test_bound_expect_suffix_still_passes(self):
        src = 'fn f(){let config=ImageEditConfig::loopback_fixture(addr).expect("fixture"); let p=NativeImageEditProvider::new(config,key,t);}'
        root = write_tree({"crates/df-tools/src/gameplay/new_image_tests.rs": src})
        self.assertEqual(CB.run(root, strict=False), [])
        self.assertEqual(CB.run(root, strict=True), [])

    def test_arbitrary_suffix_still_fails(self):
        src = "fn f(){let p=NativeImageEditProvider::new(ImageEditConfig::loopback_fixture(addr).map(|c| c),key,t);}"
        root = write_tree({"crates/df-tools/src/gameplay/new_image_tests.rs": src})
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "bypass" and "NativeImageEditProvider" in f.symbol for f in findings))


class LegacyBlockDecoyTests(LegacyFixtureCase):
    def test_legacy_config_block_decoy_is_bypass_and_stale(self):
        src = "fn changed(){let replacement=NativeImageEditProvider::new({let ignored=ImageEditConfig::live(); other_cfg},key,t);let f=NativeFalImageProvider::new(FalImageConfig::live(),key,t);}"
        root = write_tree({"crates/df-tools/src/gameplay/imagery.rs": src})
        findings = CB.run(root, strict=False)
        edit = [f for f in findings if "NativeImageEditProvider" in f.symbol]
        self.assertTrue(any(f.kind == "bypass" for f in edit))
        self.assertFalse(any(f.kind == "legacy" for f in edit))
        fal = [f for f in findings if "NativeFalImageProvider" in f.symbol]
        self.assertTrue(any(f.kind == "legacy" for f in fal))
        self.assertTrue(any(f.kind == "stale" for f in findings))
        strict_findings = CB.run(root, strict=True)
        strict_edit = [f for f in strict_findings if "NativeImageEditProvider" in f.symbol]
        self.assertTrue(any(f.kind == "bypass" for f in strict_edit))


class BrowserDependencyTests(LegacyFixtureCase):
    def _tools_root(self, tools_text: str) -> Path:
        root = write_tree({})
        target = root / "crates" / "df-tools" / "Cargo.toml"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(tools_text, encoding="utf-8")
        return root

    def _client_root(self, client_text: str) -> Path:
        root = write_tree({})
        target = root / "crates" / "df-client" / "Cargo.toml"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(client_text, encoding="utf-8")
        return root

    def test_df_tools_native_only_passes(self):
        root = self._tools_root('[package]\nname="df-tools"\n[target.\'cfg(not(target_arch = "wasm32"))\'.dependencies]\ndf-imagery={path="../df-imagery"}\n')
        self.assertEqual(CB.run(root, strict=False), [])
        self.assertEqual(CB.run(root, strict=True), [])

    def test_df_tools_unconditional_fails(self):
        root = self._tools_root('[package]\nname="df-tools"\n[dependencies]\ndf-imagery={path="../df-imagery"}\n')
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "dependency" and f.path == "crates/df-tools/Cargo.toml" for f in findings))
        self.assertTrue(any(f.kind == "dependency" for f in CB.run(root, strict=True)))

    def test_df_tools_wasm_only_fails(self):
        root = self._tools_root('[package]\nname="df-tools"\n[target.\'cfg(target_arch = "wasm32")\'.dependencies]\ndf-imagery={path="../df-imagery"}\n')
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "dependency" and f.path == "crates/df-tools/Cargo.toml" for f in findings))

    def test_df_client_double_negation_fails(self):
        root = self._client_root('[package]\nname="df-client"\n[target.\'cfg(not(not(target_arch = "wasm32")))\'.dependencies]\ndf-imagery={path="../df-imagery"}\n')
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "dependency" and "df-imagery" in f.symbol for f in findings))
        self.assertTrue(any(f.kind == "dependency" for f in CB.run(root, strict=True)))

    def test_df_client_native_only_still_passes(self):
        root = self._client_root('[package]\nname="df-client"\n[target.\'cfg(not(target_arch = "wasm32"))\'.dependencies]\ndf-imagery={path="../df-imagery"}\n')
        self.assertEqual(CB.run(root, strict=False), [])
        self.assertEqual(CB.run(root, strict=True), [])

    def test_df_client_unconditional_still_fails(self):
        root = self._client_root('[package]\nname="df-client"\n[dependencies]\ndf-imagery={path="../df-imagery"}\n')
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "dependency" and "df-imagery" in f.symbol for f in findings))

    def test_df_client_wasm_still_fails(self):
        root = self._client_root('[package]\nname="df-client"\n[target.\'cfg(target_arch = "wasm32")\'.dependencies]\ndf-imagery={path="../df-imagery"}\n')
        findings = CB.run(root, strict=False)
        self.assertTrue(any(f.kind == "dependency" and "df-imagery" in f.symbol for f in findings))



class AnalysisBoundaryTests(unittest.TestCase):
    def test_production_allowances_are_empty(self):
        self.assertEqual(CB.LEGACY_ROWS, ())
        self.assertEqual(CB.LEGACY_BUDGET, {})

    def test_analysis_constructors_cannot_bypass_shared_service(self):
        for name in ("NativeHeadLocator", "NativePlateJudge", "NativeFalSegmenter"):
            with self.subTest(name=name):
                root = write_tree({"crates/df-tools/src/gameplay/analysis.rs":
                    f"fn f() {{ let p = df_providers::{name}::new(config, key, timeout); }}"})
                findings = CB.run(root, strict=True)
                self.assertTrue(any(f.kind == "bypass" and name in f.symbol for f in findings))

    def test_analysis_executor_exports_cannot_be_reintroduced(self):
        for name in ("NativeHeadLocator", "NativePlateJudge", "NativeFalSegmenter"):
            with self.subTest(name=name):
                root = write_tree({"crates/df-providers/src/lib.rs":
                    f"pub use legacy::{name} as Renamed;"})
                self.assertTrue(any(f.kind == "export" and name in f.symbol
                    for f in CB.run(root, strict=True)))

    def test_retired_executor_definition_cannot_hide_in_an_unexported_provider_module(self):
        root = write_tree({"crates/df-providers/src/hidden.rs":
            "pub struct NativePlateJudge {}"})
        self.assertTrue(any(f.kind == "export" and "NativePlateJudge" in f.symbol
            for f in CB.run(root, strict=True)))

    def test_private_analysis_executor_construction_remains_private(self):
        root = write_tree({"crates/df-imagery/src/backends/analysis/fixture.rs":
            "fn f() { let p = NativePlateJudge::new(config, key, timeout); }"})
        self.assertEqual(CB.run(root, strict=True), [])

if __name__ == "__main__":
    unittest.main()
