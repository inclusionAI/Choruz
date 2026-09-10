#!/usr/bin/env python3
"""Replace generation and expose process overlap at the native CLI boundary."""
import json
import os
from pathlib import Path
import sys
import time

assert "--ephemeral" in sys.argv and "--ignore-user-config" in sys.argv
assert 'web_search="disabled"' in sys.argv and "shell_tool" in sys.argv
data = json.loads(sys.stdin.read().strip().splitlines()[-1])
request = json.loads(data["request"])
directory = Path(request["directory"])
name = data["member"]
(directory / name).write_text(str(os.getpid()))
if request["order"] == "parallel":
    assert data["prior_findings"] == []
    deadline = time.monotonic() + 5
    while not all((directory / name).exists() for name in ("derive", "check")):
        assert time.monotonic() < deadline, "parallel members did not overlap"
        time.sleep(0.01)
elif name == "check":
    assert data["prior_findings"] == ["derive: Derive a candidate."]
else:
    assert data["prior_findings"] == []
if request["mode"] == "fail" and name == "check":
    sys.exit(1)
if request["mode"] in ("cancel", "fail"):
    while True:
        time.sleep(1)
answer = f'{name}: {data["role"]}'
print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": answer}}))
