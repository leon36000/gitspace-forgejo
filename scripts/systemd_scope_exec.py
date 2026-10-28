#!/usr/bin/env python3
"""Exec one bounded command after its transient systemd scope is attested."""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import signal
import sys

MAX_PAYLOAD_BYTES = 256 * 1024
ENVIRONMENT_KEYS = {"PATH", "HOME", "LANG", "LC_ALL", "TERM", "NO_COLOR", "TMPDIR"}


def main() -> int:
    if len(sys.argv) != 2 or re.fullmatch(r"[a-f0-9]{32}", sys.argv[1]) is None:
        return 125
    signal.pthread_sigmask(signal.SIG_UNBLOCK, {signal.SIGTERM})
    os.write(1, b"PROCESS_SCOPE_READY:" + sys.argv[1].encode("ascii") + b"\n")
    payload = sys.stdin.buffer.readline(MAX_PAYLOAD_BYTES + 1)
    if not payload or len(payload) > MAX_PAYLOAD_BYTES or not payload.endswith(b"\n"):
        print("refusing missing or oversized scoped-command envelope", file=sys.stderr)
        return 125
    try:
        request = json.loads(payload)
    except (UnicodeDecodeError, json.JSONDecodeError):
        print("refusing invalid scoped-command envelope", file=sys.stderr)
        return 125
    if not isinstance(request, dict) or set(request) != {"command", "environment"}:
        print("refusing malformed scoped-command envelope", file=sys.stderr)
        return 125
    command = request.get("command")
    environment = request.get("environment")
    if (
        not isinstance(command, list)
        or not command
        or any(not isinstance(part, str) or "\0" in part for part in command)
        or not isinstance(environment, dict)
        or set(environment) - ENVIRONMENT_KEYS
        or any(
            not isinstance(key, str)
            or not isinstance(value, str)
            or "\0" in value
            for key, value in environment.items()
        )
    ):
        print("refusing invalid scoped-command fields", file=sys.stderr)
        return 125
    try:
        null_fd = os.open(os.devnull, os.O_RDONLY)
        try:
            os.dup2(null_fd, 0)
        finally:
            if null_fd != 0:
                os.close(null_fd)
        os.execvpe(command[0], command, environment)
    except OSError:
        print("scoped command could not be executed", file=sys.stderr)
        return 125
    return 125


if __name__ == "__main__":
    raise SystemExit(main())
