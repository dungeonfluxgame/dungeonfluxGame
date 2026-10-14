import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from presentation_timeline import summarize, timing_events


class PresentationTimelineTests(unittest.TestCase):
    def test_cli_reads_a_utf16_timeline_saved_by_powershell(self):
        with tempfile.TemporaryDirectory() as folder:
            saved = Path(folder) / "timeline.json"
            saved.write_text(json.dumps({"events": [{"line":
                "PHONE paint producer=phone batch=1 id=0 scene=0 at_ms=300 reason=ready"
            }]}), encoding="utf-16")
            result = subprocess.run([
                sys.executable, "-B", str(Path(__file__).with_name("presentation_timeline.py")), str(saved)
            ], capture_output=True, text=True, timeout=10, check=False)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("paint=300", result.stdout)

    def test_reading_budget_and_actual_voice_cut_are_reported_separately(self):
        events = list(timing_events([
            "DISPLAY line-open producer=p batch=1 id=29 scene=7 at_ms=100 opened_ms=100 chars=89 read_ms=8285 reason=ready",
            "DISPLAY voice-start producer=p batch=2 id=29 scene=7 at_ms=200 voice_start_ms=200 reason=ready",
            "DISPLAY line producer=p batch=3 id=29 scene=7 at_ms=4000 opened_ms=100 closed_ms=4000 voice_start_ms=200 voice_end_ms=none chars=89 read_ms=8285 reason=skipped",
        ]))
        report = summarize(events)
        self.assertIn("29 | 89 | 8285 | 3900 | cut/unknown | skipped", report)
        self.assertNotIn("complete:", report)

    def test_late_voice_end_reports_a_cut_and_preserves_the_line_closure_reason(self):
        events = list(timing_events([
            "DISPLAY line producer=p batch=1 id=29 scene=7 at_ms=4000 opened_ms=100 closed_ms=4000 voice_start_ms=200 voice_end_ms=none chars=89 read_ms=8285 reason=skipped",
            "DISPLAY voice-end producer=p batch=2 id=29 scene=7 at_ms=4500 opened_ms=100 closed_ms=4000 voice_start_ms=200 voice_end_ms=4500 chars=89 read_ms=8285 reason=ended",
        ]))
        self.assertIn("29 | 89 | 8285 | 3900 | cut | skipped", summarize(events))

    def test_missing_voice_and_media_failure_are_not_invented_as_success(self):
        events = list(timing_events([
            "DISPLAY line producer=p batch=1 id=30 scene=7 at_ms=10000 opened_ms=100 closed_ms=10000 chars=89 read_ms=8285 voice_start_ms=none voice_end_ms=none reason=read",
            "DISPLAY plate-kept producer=p batch=2 id=5 scene=7 at_ms=10001 reason=failed",
            "PHONE paint producer=phone batch=1 id=0 scene=0 at_ms=300 reason=ready",
            "PHONE usable producer=phone batch=1 id=0 scene=0 at_ms=250 reason=ready",
            "DISPLAY gap producer=p batch=3 dropped_records=4",
        ]))
        report = summarize(events)
        self.assertIn("none/unobserved", report)
        self.assertIn("plate-kept | 5 | failed", report)
        self.assertIn("usable=250 | paint=300", report)
        self.assertIn("dropped_records=4", report)

    def test_separate_page_clocks_do_not_merge_line_ids(self):
        events = list(timing_events([
            "DISPLAY line producer=a batch=1 id=1 scene=7 at_ms=10 opened_ms=1 closed_ms=10 chars=1 read_ms=5 reason=read",
            "DISPLAY line producer=b batch=1 id=1 scene=7 at_ms=40 opened_ms=10 closed_ms=40 chars=1 read_ms=5 reason=read",
        ]))
        report = summarize(events)
        self.assertIn("a | 7 | 1 | 1 | 5 | 9", report)
        self.assertIn("b | 7 | 1 | 1 | 5 | 30", report)

    def test_plate_milestones_and_native_exchange_total_use_explicit_measurements(self):
        events = list(timing_events([
            "DISPLAY plate-requested producer=p batch=1 id=5 scene=7 at_ms=100 reason=ready",
            "DISPLAY plate-loaded producer=p batch=1 id=5 scene=7 at_ms=400 reason=ready",
            "DISPLAY plate-decoded producer=p batch=1 id=5 scene=7 at_ms=500 reason=ready",
            "DISPLAY plate-shown producer=p batch=1 id=5 scene=7 at_ms=600 crossfade_ms=1200 reason=ready",
            "STORY exchange id=abc total_ms=250 status=ready",
        ]))
        report = summarize(events)
        self.assertIn("p | 7 | 5 | 100 | 400 | 500 | 600 | 1200 | False", report)
        self.assertIn("STORY exchange | abc | 250 | ready", report)

    def test_synthetic_uploads_are_marked_even_when_label_arrives_after_events(self):
        events = list(timing_events([
            "DISPLAY line producer=syntheticpage batch=1 id=1 scene=7 at_ms=10 opened_ms=1 closed_ms=10 chars=1 read_ms=5 reason=read",
            "DISPLAY synthetic-probe producer=syntheticpage synthetic=true records=1 evidence=ingress-only",
        ]))
        self.assertIn("syntheticpag*synthetic", summarize(events))

    def test_animation_frame_opportunity_does_not_claim_browser_first_paint(self):
        events = list(timing_events([
            "PHONE paint-opportunity producer=p batch=1 id=0 scene=0 at_ms=500 reason=ready",
        ]))
        report = summarize(events)
        self.assertIn("paint=missing | paint-opportunity=500", report)

    def test_spoken_words_that_resemble_diagnostics_are_not_timing_evidence(self):
        events = list(timing_events([
            'LINE queued speaker="Brakka" text="DISPLAY line producer=p id=1 scene=7 held_ms=9"',
            'LINE queued speaker="Brakka" text="STORY exchange id=fake total_ms=1"',
        ]))
        self.assertEqual(events, [])


if __name__ == "__main__":
    unittest.main()
