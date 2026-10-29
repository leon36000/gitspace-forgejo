#!/usr/bin/env python3
"""Check simple local Markdown links in the five canonical project documents."""

from __future__ import annotations

import json
from pathlib import Path
import re
from urllib.parse import unquote, urlsplit


ROOT = Path(__file__).resolve().parents[2]
DOCUMENTS = (
    "docs/checkpoint.md",
    "docs/architecture.md",
    "docs/implementation-plan.md",
    "docs/refusal-evidence-matrix.md",
    "docs/source-inventory.md",
)
LINK = re.compile(r"\[[^\]]*\]\(([^)]+)\)")


def audit() -> dict[str, object]:
    missing: list[dict[str, str]] = []
    checked_links = 0
    for relative_document in DOCUMENTS:
        document = ROOT / relative_document
        for raw_target in LINK.findall(document.read_text(encoding="utf-8")):
            target = raw_target.strip()
            parsed = urlsplit(target)
            if parsed.scheme or not parsed.path:
                continue
            checked_links += 1
            path = Path(unquote(parsed.path))
            resolved = path if path.is_absolute() else document.parent / path
            if not resolved.exists():
                missing.append({"document": relative_document, "target": target})
    return {
        "documents": list(DOCUMENTS),
        "checked_links": checked_links,
        "missing_count": len(missing),
        "missing": missing,
        "passed": not missing,
    }


if __name__ == "__main__":
    print(json.dumps(audit(), ensure_ascii=False, sort_keys=True, indent=2))
