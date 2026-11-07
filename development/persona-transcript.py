"""Turns a market-thread log into a markdown transcript: every line, then the answering
character's stance, fired triggers, feelings and timing. A batch log holds several scenes, each
after a `=== <beats>` line; `section=<beats>` picks one. Usage:
py -3 development/persona-transcript.py <log> <out.md> <title> <header-text>
    [section=<beats>] id=Name ..."""
import re
import sys

log_path, out_path, title, header = sys.argv[1:5]
options = dict(arg.split("=", 1) for arg in sys.argv[5:])
section = options.pop("section", None)
names = options
out = [f"# {title}", "", header, ""]
inside = section is None
for raw in open(log_path, encoding="utf-8"):
    line = raw.rstrip("\n")
    if line.startswith("=== "):
        inside = section is None or line[4:].strip() == section
        continue
    if not inside:
        continue
    spoken = re.match(r"^\[(\d+)\] ([a-z_]+): (.*)$", line)
    if spoken:
        number, who, text = spoken.groups()
        out += ["", f"**[{number}] {names.get(who, who)}:** {text}"]
        continue
    detail = line.strip()
    if detail:
        out.append(f"- {detail}")
open(out_path, "w", encoding="utf-8", newline="\n").write("\n".join(out) + "\n")
