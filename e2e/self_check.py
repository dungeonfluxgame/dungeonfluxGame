"""Safety and completeness invariants of the test runner, independent of product availability."""

import importlib
import unittest

from e2e.catalog import DIMENSIONS, FLOWS
from e2e.http_client import Client, require_loopback
from e2e.scenarios import applies, scenarios_for


class RunnerContract(unittest.TestCase):
    def test_external_origins_and_forbidden_port_are_refused_before_network(self):
        for origin in ("https://dungeonfluxdnd.com", "http://127.0.0.1:63309",
                       "http://user:pass@localhost:49271", "file:///etc/passwd",
                       "http://localhost:49271/private", "http://localhost:49271?token=x"):
            with self.subTest(origin=origin), self.assertRaises(ValueError):
                require_loopback(origin)

    def test_protocol_relative_and_fragment_paths_are_refused_before_network(self):
        client = Client("http://127.0.0.1:49271")
        for path in ("//provider.invalid/token", "https://provider.invalid", "/verify#token=x"):
            with self.subTest(path=path), self.assertRaises(ValueError):
                client.call("GET", path)

    def test_every_flow_has_one_module_and_all_applicable_dimensions(self):
        self.assertEqual(len(FLOWS), 55)
        keys = set()
        for flow in FLOWS:
            module = importlib.import_module(f"e2e.{flow.area}.test_{flow.name}")
            self.assertEqual(module.CASES, scenarios_for(flow))
            dimensions = {case.dimension for case in module.CASES}
            self.assertEqual(dimensions, {name for name in DIMENSIONS if applies(flow, name)})
            for case in module.CASES:
                self.assertIsInstance(case.flow, str)
                self.assertEqual(case.flow, flow.id)
                key = (case.flow, case.dimension, case.variant)
                self.assertNotIn(key, keys)
                keys.add(key)


if __name__ == "__main__":
    unittest.main()
