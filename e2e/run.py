"""Run every declared case; a missing feature is a nonzero, gap-labelled result, never xfail."""

import argparse
import importlib
import json
import os
import sys
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

from e2e.catalog import FLOWS
from e2e.harness import CheckFailure, Session


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--origin", default="http://127.0.0.1:49271")
    parser.add_argument("--site-open", default="http://127.0.0.1:49272")
    parser.add_argument("--site-closed", default="http://127.0.0.1:49273")
    parser.add_argument("--outbox", type=Path,
                        default=Path("artifacts/tmp/agents/FLOWMAP/outbox"))
    parser.add_argument("--output", type=Path,
                        default=Path("artifacts/tmp/agents/FLOWMAP/results.json"))
    parser.add_argument("--flow", action="append", help="Flow id or name; repeat to select")
    parser.add_argument("--dimension", action="append")
    parser.add_argument("--channel", default="msedge")
    parser.add_argument("--list", action="store_true")
    args = parser.parse_args()
    flows = [flow for flow in FLOWS if not args.flow or flow.id in args.flow
             or flow.name in args.flow]
    if not flows:
        parser.error("No matching flows")
    cases = []
    for flow in flows:
        module = importlib.import_module(f"e2e.{flow.area}.test_{flow.name}")
        cases.extend(case for case in module.CASES
                     if not args.dimension or case.dimension in args.dimension)
    if args.list:
        for case in cases:
            print(f"{case.flow}/{case.dimension}/{case.variant}")
        return 0
    args.output.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path("artifacts/tmp/lanes/FLOWMAP/tmp").resolve()
    temporary.mkdir(parents=True, exist_ok=True)
    os.environ["TEMP"] = os.environ["TMP"] = str(temporary)
    evidence = []
    with Session(args) as session:
        for case in cases:
            session.begin(case)
            try:
                session.execute(case)
                result, detail, gap = "PASS", session.observation, None
            except CheckFailure as error:
                result, detail, gap = error.result, str(error), error.gap
            except Exception as error:
                result, detail, gap = "FAIL", f"{type(error).__name__}: {error}", "FLOW-HARNESS"
            screenshot = session.capture(case, result)
            row = {"flow": case.flow, "dimension": case.dimension, "variant": case.variant,
                   "result": result, "detail": detail, "gap": gap, "screenshot": screenshot,
                   "screenshot_error": session.capture_error,
                   "observed_at": datetime.now(timezone.utc).isoformat()}
            evidence.append(row)
            args.output.write_text(json.dumps({"results": evidence}, indent=2) + "\n",
                                   encoding="utf-8", newline="\n")
            print(f"{case.flow}/{case.dimension}/{case.variant}: {result} {gap or ''}", flush=True)
    counts = dict(Counter(row["result"] for row in evidence))
    print(json.dumps(counts, sort_keys=True))
    return int(any(row["result"] != "PASS" for row in evidence))


if __name__ == "__main__":
    sys.exit(main())
