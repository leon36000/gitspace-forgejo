#!/usr/bin/env python3
"""Append this evidence package and checkpoint to both SHA-256 indexes."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import sys

run = Path(__file__).resolve().parent
helper = run.parents[1] / "evidence/continuation-450-current-pr-base-id-consistency-001/append_manifests.py"
spec = importlib.util.spec_from_file_location("append_manifest_helper", helper)
if spec is None or spec.loader is None:
    raise SystemExit("cannot load the audited append-only manifest helper")
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)
module.RUN = run
module.PREFIX = run / "manifest-prefix-before.json"
module.LOCAL = run / "SHA256SUMS"
module.TOUCHED = ("docs/checkpoint.md",)
raise SystemExit(module.main())
