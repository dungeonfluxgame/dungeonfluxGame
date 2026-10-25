#!/usr/bin/env python3
"""Check the demo nginx request logs do not include private URL or header data."""

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[3]
CONFIG = ROOT / "deploy" / "demo" / "nginx" / "dungeonflux.conf"
FORBIDDEN_LOG_VARIABLE = re.compile(
    r"\$(?:request|request_uri|args|query_string|http_referer|http_cookie|http_authorization)\b"
)
REQUIRED_LOG_VARIABLES = {
    "$request_method",
    "$uri",
    "$status",
    "$body_bytes_sent",
    "$request_time",
    "$upstream_status",
    "$request_id",
}


class NginxLogConfigTests(unittest.TestCase):
    def test_access_logs_name_a_custom_format(self) -> None:
        config = CONFIG.read_text(encoding="utf-8")
        access_logs = re.findall(r"^\s*access_log\s+([^;]+);", config, re.MULTILINE)

        self.assertTrue(access_logs, "expected at least one access_log directive")
        for directive in access_logs:
            fields = directive.split()
            if fields[0] == "off":
                continue
            self.assertGreaterEqual(
                len(fields), 2, f"access_log must name a format: {directive}"
            )
            self.assertNotEqual(
                fields[1], "combined", f"access_log must not use combined: {directive}"
            )
            self.assertEqual(
                fields[1], "dungeonflux_safe", f"unexpected access_log format: {directive}"
            )

    def test_log_format_uses_only_query_free_request_fields(self) -> None:
        config = CONFIG.read_text(encoding="utf-8")
        formats = re.findall(
            r"^\s*log_format\s+(\w+)\s+(.+?);", config, re.MULTILINE | re.DOTALL
        )

        self.assertTrue(formats, "expected a custom log_format")
        safe_formats = []
        for name, body in formats:
            if FORBIDDEN_LOG_VARIABLE.search(body):
                self.fail(f"log_format {name} contains a private request variable")
            if name == "dungeonflux_safe":
                safe_formats.append(body)

        self.assertEqual(len(safe_formats), 1, "expected one dungeonflux_safe log_format")
        variables = set(re.findall(r"\$[a-zA-Z_]+", safe_formats[0]))
        self.assertTrue(
            REQUIRED_LOG_VARIABLES <= variables,
            f"dungeonflux_safe is missing required fields: {REQUIRED_LOG_VARIABLES - variables}",
        )


if __name__ == "__main__":
    unittest.main()
