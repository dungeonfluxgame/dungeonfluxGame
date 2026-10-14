#!/usr/bin/env python3
"""Summarize DISPLAY/PHONE timing audit lines or `dev timeline` JSON, without game data writes."""

import argparse
import json
import re
from collections import defaultdict, deque
from pathlib import Path


def timing_events(lines):
    for line in lines:
        match = re.match(r"(DISPLAY|PHONE) ([a-z-]+) (.*)", line.lstrip())
        if match:
            fields = dict(re.findall(r"([a-z_]+)=([^\s]+)", match[3]))
            fields.update(audience=match[1], kind=match[2])
            yield fields
        else:
            match = re.match(r"(STORY exchange|REPLY model|APPROACH model|IMAGERY ready|IMAGERY failed|VOICE clip) (.*)", line.lstrip())
            if match:
                fields = dict(re.findall(r"([a-z_]+)=([^\s]+)", match[2]))
                fields.update(audience="SERVER", kind=match[1])
                yield fields


def number(event, key):
    value = event.get(key, "none")
    return None if value == "none" else int(value)


def summarize(events):
    lines = {}
    scenes = defaultdict(list)
    phones = defaultdict(dict)
    gaps = []
    media = defaultdict(dict)
    native = []
    synthetic = set()
    for event in events:
        producer = event.get("producer", "unknown")
        scene = event.get("scene", "unknown")
        if event["kind"] == "synthetic-probe":
            synthetic.add(producer)
            continue
        if event["audience"] == "SERVER":
            native.append(event)
            continue
        if event["kind"].startswith("plate-"):
            media[producer, scene, event.get("id", "unknown")][event["kind"]] = event
        if event["kind"] in ("line", "line-open", "voice-start", "voice-end", "voice-failed"):
            key = producer, scene, event.get("id", "unknown")
            # Optional fields on a later lifecycle record must not erase an observed media end.
            record = lines.setdefault(key, {})
            # A late media end updates the measured voice time, not why the DOM line closed.
            record.update({key: value for key, value in event.items()
                           if value != "none" and (key != "reason" or event["kind"] in ("line", "line-open"))})
        elif event["kind"] in ("plate-shown", "plate-kept", "clip-shown", "figure-shown", "figure-hidden"):
            scenes[producer, scene].append(event)
        elif event["kind"] in ("paint", "paint-opportunity", "usable"):
            phones[producer][event["kind"]] = event.get("at_ms", "unknown")
        elif event["kind"] == "gap":
            gaps.append(event)

    def producer_label(producer):
        return producer[:12] + ("*synthetic" if producer in synthetic else "")

    output = ["Line timings (one page clock per producer; times in milliseconds)",
              "producer | scene | line | chars | read | held | voice | close"]
    for (producer, scene, line_id), event in lines.items():
        opened, closed = number(event, "opened_ms"), number(event, "closed_ms")
        start, end = number(event, "voice_start_ms"), number(event, "voice_end_ms")
        held = "open/unknown" if opened is None or closed is None else str(closed - opened)
        if start is None:
            voice = "none/unobserved"
        elif end is None:
            voice = "cut/unknown" if closed is not None else "playing/unknown"
        else:
            voice = "cut" if closed is not None and end > closed else f"complete:{end - start}"
        output.append(f"{producer_label(producer)} | {scene} | {line_id} | {event.get('chars', '?')} | "
                      f"{event.get('read_ms', '?')} | {held} | {voice} | {event.get('reason', '?')}")
    output += ["", "Scene media changes", "producer | scene | at | event | asset | reason"]
    for (producer, scene), changes in scenes.items():
        for event in changes:
            output.append(f"{producer_label(producer)} | {scene} | {event.get('at_ms', '?')} | "
                          f"{event['kind']} | {event.get('id', '?')} | {event.get('reason', '?')}")
    output += ["", "Plate lifecycle (missing milestones remain unknown)",
               "producer | scene | asset | requested | loaded | decoded | shown | crossfade | kept"]
    for (producer, scene, asset), stages in media.items():
        times = [stages.get(f"plate-{stage}", {}).get("at_ms", "missing")
                 for stage in ("requested", "loaded", "decoded", "shown")]
        fade = stages.get("plate-shown", {}).get("crossfade_ms", "missing")
        output.append(f"{producer_label(producer)} | {scene} | {asset} | " + " | ".join(times)
                      + f" | {fade} | {'plate-kept' in stages}")
    output += ["", "First usable controls / browser first paint (page-relative; unsupported remains missing)"]
    for producer, events in phones.items():
        output.append(f"{producer_label(producer)} | usable={events.get('usable', 'missing')} | paint={events.get('paint', 'missing')} | paint-opportunity={events.get('paint-opportunity', 'missing')}")
    for event in gaps:
        output.append(f"GAP producer={event.get('producer')} dropped_records={event.get('dropped_records')}")
    output += ["", "Native operation / exchange totals", "event | input/line/id | elapsed | status"]
    for event in native:
        operation = event.get("id", event.get("input", event.get("line", "missing")))
        output.append(f"{event['kind']} | {operation} | {event.get('total_ms', event.get('ms', 'missing'))} | {event.get('status', '?')}")
    if not any(event["kind"] == "STORY exchange" for event in native):
        output.append("No correlated native exchange totals were emitted in this window.")
    output += ["", "Do not subtract clocks from different pages or add overlapping provider times.",
               "Missing end/open/paint events remain unknown. Synthetic uploads are not playback evidence."]
    return "\n".join(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path, help="server log or saved dev timeline JSON")
    parser.add_argument("--last", type=int, default=1000, help="last 1..10000 presentation events")
    args = parser.parse_args()
    if not 1 <= args.last <= 10000:
        parser.error("--last must be 1..10000")
    # Windows PowerShell saves redirected native output as BOM-marked UTF-16.
    with args.log.open("rb") as source:
        header = source.read(2)
    encoding = "utf-16" if header in (b"\xff\xfe", b"\xfe\xff") else "utf-8-sig"
    with args.log.open(encoding=encoding) as source:
        first = source.read(1)
        source.seek(0)
        if first == "{":
            timeline = json.load(source)
            source_lines = (event["line"] for event in timeline["events"])
            if timeline.get("dropped"):
                print(f"TIMELINE GAP: {timeline['dropped']} earlier audit events unavailable")
        else:
            source_lines = source
        events = deque(maxlen=args.last)
        matched = 0
        for event in timing_events(source_lines):
            matched += 1
            events.append(event)
    print(f"Showing {len(events)} of {matched} matching events; {matched - len(events)} earlier events are outside this window.")
    print(summarize(events))


if __name__ == "__main__":
    main()
