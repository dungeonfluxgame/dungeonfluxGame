#!/usr/bin/env python3
"""Check local href and src targets in HTML files under a docs directory."""

import collections
import os
import posixpath
import re
import sys
from pathlib import Path


ATTRIBUTE = re.compile(r'''(?:src|href)=["']([^"'?#]+)''')
EXTERNAL = re.compile(r"^[a-z]+:|^//", re.IGNORECASE)


def main() -> int:
    docs_dir = Path(sys.argv[1] if len(sys.argv) > 1 else "docs").resolve()
    if not docs_dir.is_dir():
        print(f"docs directory not found: {docs_dir}", file=sys.stderr)
        return 2

    broken = collections.Counter()
    affected_files = set()
    for html_path in docs_dir.rglob("*.html"):
        page = html_path.read_text(encoding="utf-8")
        relative_path = html_path.relative_to(docs_dir).as_posix()
        page_directory = posixpath.dirname(relative_path)
        base = "/" + page_directory + "/" if page_directory else "/"
        for match in ATTRIBUTE.finditer(page):
            target = match.group(1)
            if EXTERNAL.match(target):
                continue
            resolved = target if target.startswith("/") else posixpath.normpath(posixpath.join(base, target))
            file_path = docs_dir / resolved.lstrip("/")
            if not (file_path.is_file() or (file_path / "index.html").is_file()):
                broken[resolved] += 1
                affected_files.add(relative_path)

    for target, count in broken.most_common():
        print(f"{count} {target}")
    print(f"{len(affected_files)} files affected")
    if affected_files:
        print("\n".join(sorted(affected_files)))
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
