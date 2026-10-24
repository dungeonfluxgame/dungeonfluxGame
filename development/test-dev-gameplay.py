#!/usr/bin/env python3
"""Validate preview configuration without spawning games or providers."""
import importlib.util
import io
import os
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("dev_gameplay", Path(__file__).with_name("dev-gameplay.py"))
preview = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preview)

class PreviewTests(unittest.TestCase):
    def test_alternate_portraits_require_explicit_opt_in_and_both_credentials(self):
        options = SimpleNamespace(artifact_root=Path("artifacts").resolve(), database_port=55527, port=63329, screen="creation", creator_portraits=True, creator_alternate_bfl=True)
        with patch.dict(os.environ, {"OPENAI_API_KEY":"synthetic-openai", "BFL_API_KEY":"synthetic-bfl", "DF_JEV_API_KEY":"synthetic-jev", "DF_JEV_BASE_URL":"https://jev.example.invalid"}, clear=True):
            env = preview.preview_environment(options, Path("artifacts/tmp/preview").resolve(), Path("artifacts/build/preview").resolve())
            self.assertEqual(env["DF_GAMEPLAY_CREATOR_BFL_KEY"], "synthetic-bfl")
            self.assertNotIn("BFL_API_KEY",env)
            self.assertNotIn("OPENAI_API_KEY",env)
            self.assertNotIn("DF_JEV_API_KEY",env)
            self.assertNotIn("DF_JEV_BASE_URL",env)
            self.assertEqual(env["DF_GAMEPLAY_CREATOR_JEV_API_KEY"],"synthetic-jev")
            self.assertEqual(env["DF_GAMEPLAY_CREATOR_JEV_BASE_URL"],"https://jev.example.invalid")
            options.creator_alternate_bfl=False
            self.assertNotIn("DF_GAMEPLAY_CREATOR_BFL_KEY",preview.preview_environment(options,Path("artifacts/tmp/preview").resolve(),Path("artifacts/build/preview").resolve()))
            options.creator_alternate_bfl=True
            options.creator_portraits=False
            with self.assertRaises(RuntimeError): preview.preview_environment(options,Path("artifacts/tmp/preview").resolve(),Path("artifacts/build/preview").resolve())
        options.creator_portraits=True
        with patch.dict(os.environ, {"OPENAI_API_KEY":"synthetic-openai", "DF_JEV_API_KEY":"synthetic-jev"}, clear=True):
            with self.assertRaises(RuntimeError): preview.preview_environment(options,Path("artifacts/tmp/preview").resolve(),Path("artifacts/build/preview").resolve())
        with patch.dict(os.environ, {"OPENAI_API_KEY":"synthetic-openai", "BFL_API_KEY":"synthetic-bfl"}, clear=True):
            with self.assertRaisesRegex(RuntimeError,"DF_JEV_API_KEY"): preview.preview_environment(options,Path("artifacts/tmp/preview").resolve(),Path("artifacts/build/preview").resolve())
    def test_preview_environment_removes_inherited_credentials_and_game_choices(self):
        options = SimpleNamespace(artifact_root=Path("artifacts").resolve(), database_port=55527, port=63329, screen="creation")
        inherited = {"OPENAI_API_KEY": "private-secret", "FAL_KEY": "private-secret", "DF_JEV_API_KEY": "private-secret", "DF_JEV_BASE_URL": "https://invalid.example", "DF_GAMEPLAY_TEST_FIGHT": "4", "DF_GAMEPLAY_PUBLIC_URL": "https://invalid.example", "DF_GAMEPLAY_ADVENTURE": "campaign", "PATH": os.environ["PATH"]}
        with patch.dict(os.environ, inherited, clear=True):
            env = preview.preview_environment(options, Path("artifacts/tmp/preview").resolve(), Path("artifacts/build/preview").resolve())
        self.assertNotIn("private-secret", repr(env))
        self.assertNotIn("DF_GAMEPLAY_PUBLIC_URL", env)
        self.assertNotIn("DF_GAMEPLAY_TEST_FIGHT", env)
        self.assertNotIn("DF_JEV_BASE_URL", env)
        self.assertEqual(env["DF_GAMEPLAY_LOAD_DOTENV"], "off")
        self.assertEqual(env["DF_GAMEPLAY_LISTEN"], "127.0.0.1:63329")
        for name in ["STORYTELLER", "VOICE", "IMAGERY", "PLATES", "STT", "MUSIC", "HOME_VIDEO", "SCENE_VIDEO"]:
            self.assertEqual(env["DF_GAMEPLAY_" + name], "off")

    def test_list_and_config_do_not_spawn_processes(self):
        for flag in ["--list-screens", "--print-config"]:
            with patch("sys.argv", ["dev-gameplay.py", flag]), patch("sys.stdout", io.StringIO()), patch.object(preview.subprocess, "Popen", side_effect=AssertionError("must not spawn")):
                self.assertEqual(preview.main(), 0)

    def test_unsupported_screens_are_rejected_before_launch(self):
        with patch("sys.argv", ["dev-gameplay.py", "--screen", "account-login"]), patch("sys.stderr", io.StringIO()):
            with self.assertRaises(SystemExit) as raised:
                preview.arguments()
        self.assertEqual(raised.exception.code, 2)

if __name__ == "__main__":
    unittest.main()
