#!/usr/bin/env python3
"""Replace model generation while exercising the real evaluation transport."""
import json
import os
import sys

assert "--ephemeral" in sys.argv and "--ignore-user-config" in sys.argv
assert 'web_search="disabled"' in sys.argv and "shell_tool" in sys.argv
assert "--resume" not in sys.argv and "--model" in sys.argv
assert not os.listdir("."), "evaluation must use an empty scratch workspace"
data = json.loads(sys.stdin.read().strip().splitlines()[-1])
team_search = sys.argv[sys.argv.index("--model") + 1] == "team-evaluation-fixture"
assert "expected" not in data and "check" not in data
if "proposed_instruction" in data:
    assert "suite" not in data, "held-out answers must not drive content review"
    if data["seed_evidence"]["analysis"] == "Review unavailable":
        raise RuntimeError("final review fixture unavailable")
    answer = json.dumps({"accepted": data["seed_evidence"]["analysis"] != "Reject this proposal", "evidence": [data["seed_reference"]]})
elif "component" in data:
    assert data["component"] in ("instruction", "team")
    for parent in data["parents"]:
        for example in parent["examples"]:
            assert example["input"] in ["Increment 10", "Increment 11"], "holdout leaked to reflection"
    team = {"order": "parallel", "members": [{"name": "derive", "prompt": "Derive the requested number."}, {"name": "check", "prompt": "Check the requested format."}]}
    answer = json.dumps({"text": json.dumps(team) if data["component"] == "team" else "Return the number only."})
elif "role" in data:
    answer = data["role"]
else:
    number = int(data["task"].split()[1])
    answer = str(number + 1)
    if team_search:
        if not all(prompt in data["preflight"] for prompt in ("Derive the requested number.", "Check the requested format.")):
            answer = "The answer is " + answer
    elif data["guidance"] != "Return the number only.":
        answer = "The answer is " + answer
    elif number < 10:
        assert "choruz-team" in data["preflight"]
print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": answer}}))
