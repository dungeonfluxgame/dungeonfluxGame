"""Focused pure and local operating-system checks for the memory runner."""

import importlib.util
from pathlib import Path
import unittest
from unittest import mock
from types import SimpleNamespace
import tempfile

SPEC = importlib.util.spec_from_file_location('load_runner', Path(__file__).with_name('run_load.py'))
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)


class LoadRunnerTests(unittest.TestCase):
    def test_reserved_database_and_game_ports_are_refused_before_resources_start(self):
        for ports in [(63309, 63411, [1]), (55911, 63308, [2]), (55911, 65534, [5]),
                      (0, 63411, [1]), (55911, 1000, [1])]:
            with self.assertRaises(ValueError):
                RUNNER.validate_ports(*ports)
        RUNNER.validate_ports(55911, 63411, [1, 5])

    def test_cleanup_attempts_all_servers_and_databases_after_invalid_receipt_and_drop_failure(self):
        scratch = RUNNER.ROOT / 'artifacts/tmp/agents/LOADHARNESS'
        with tempfile.TemporaryDirectory(dir=scratch) as directory:
            output = Path(directory)
            stop = output / 'stop'
            stop.mkdir()
            (stop / 'target.receipt').write_text('../bad.stop')
            processes = [mock.Mock(pid=1, returncode=0), mock.Mock(pid=2, returncode=0)]
            settings = SimpleNamespace(psql='psql', database_port=55911)
            with mock.patch.object(RUNNER, 'database', side_effect=[RuntimeError('drop failed'), None]) as drop:
                with self.assertRaises(RuntimeError):
                    RUNNER.cleanup(settings, output, processes, [], [], ['one', 'two'], [stop])
            self.assertEqual(drop.call_count, 2)
            for process in processes:
                process.wait.assert_called_once()
            self.assertTrue((output / 'cleanup.json').is_file())

    def test_nearest_rank_p95_and_peak_cover_unsorted_samples(self):
        summary = RUNNER.summarize(list(range(100, 0, -1)))
        self.assertEqual(summary, dict(samples=100, peak_bytes=100, p95_bytes=95))

    def test_empty_measurement_is_an_error(self):
        with self.assertRaises(ValueError):
            RUNNER.summarize([])

    def test_runtime_environment_does_not_inherit_provider_keys(self):
        self.assertNotIn('OPENAI_API_KEY', RUNNER.runtime_environment())
        self.assertNotIn('DF_GAMEPLAY_STORYTELLER', RUNNER.runtime_environment())

    def test_whole_host_physical_memory_is_nonzero_without_server_processes(self):
        resident, used, total = RUNNER.memory([])
        self.assertEqual(resident, [])
        self.assertGreater(used, 0)
        self.assertGreater(total, used)


if __name__ == '__main__':
    unittest.main()
