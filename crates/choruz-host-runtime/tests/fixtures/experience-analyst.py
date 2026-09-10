#!/usr/bin/env python3
"""Replace only model generation; source, dispatch and persistence remain real."""
import json
import os
import re
import sys

assert "--ephemeral" in sys.argv and "--ignore-user-config" in sys.argv
assert "--resume" not in sys.argv
assert not os.path.exists("source.jsonl")
source = sys.stdin.read()
if 'web_search="live"' in sys.argv:
    assert "source-workspace" not in source and '"ref"' not in source
    print(json.dumps({"type": "item.completed", "item": {"type": "web_search"}}))
    print(json.dumps({"type": "item.completed", "item": {
        "type": "agent_message", "text": "Completed search: require observable check evidence before completion.",
    }}))
    sys.exit(0)
refs = re.findall(r'"ref"\s*:\s*"([^"]+)"', source)
data = json.loads(source.strip().splitlines()[-1])
if "component" in data:
    team = {"order": "parallel", "members": [
        {"name": "derive", "prompt": "Plan task-specific observable checks for the task."},
        {"name": "check", "prompt": "Verify workspace changes before reporting completion."},
    ]}
    text = json.dumps(team) if data["component"] == "team" else data["parents"][0]["guidance"]["instruction"]
    print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": json.dumps({"text": text})}}))
    sys.exit(0)
if "seed_evidence" in data:
    print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": json.dumps({
        "accepted": True, "evidence": [data["seed_reference"]],
    })}}))
    sys.exit(0)
if "role" in data:
    print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": json.dumps({"checks": ["Run the required check before claiming completion."]})}}))
    sys.exit(0)
records = data["trace"]["records"]
failure = any("completion claim" in json.dumps(record).lower() for record in records)
failed_records = [record for record in records if "completion claim" in json.dumps(record).lower()]
active_revision = data.get("active_revision_id")
marker = f"[choruz-experience revision={active_revision}]" if active_revision else None
markers = [record for record in records if marker and marker in json.dumps(record)
           and (record["record"].get("type") == "user"
                or record["record"].get("payload", {}).get("role") == "user")]
prior = data.get("prior_summary", "")
objectives = json.loads(prior.removeprefix("Open objectives: ")) if prior.startswith("Open objectives: ") else {}
if objectives.get("applied_revision") != active_revision:
    objectives.pop("applied", None)
    objectives.pop("applied_revision", None)
applied_refs = {}
for record in records:
    if record in failed_records:
        applied_refs[record["ref"]] = objectives.get("applied")
    if "Objective opener:" in json.dumps(record):
        objectives["native"] = record["ref"]
    if "Shared objective opener:" in json.dumps(record):
        objectives["feedback"] = record["ref"]
    if record in markers:
        objectives["applied"] = record["ref"]
        objectives["applied_revision"] = active_revision
applied_ref = objectives.get("applied")
retain_marker = any("Retain marker reference" in json.dumps(record) for record in markers)
continued = bool(objectives)
refs = refs or data.get("prior_references", [])
assert refs, "production reader must supply native evidence"
instruction = "Verify required checks before reporting completion." if failure or "research" in data or "proposed_instruction" in data else None
report = {
    "summary": "Open objectives: " + json.dumps(objectives) if continued and not failure else "Agent claimed completion without the required check; user corrected it.",
    "instruction": instruction,
    "evidence": [applied_ref] if retain_marker else ([] if continued and not failure else [refs[0]]),
    "previous_revision_outcome": "not_observed",
    "problems": [{"key": "skipped-verification", "description": "Completion claimed without checking the required outcome.",
        "episode_ref": objectives["feedback" if "Shared completion claim" in json.dumps(record) else "native"] if continued and not applied_refs[record["ref"]] and not markers else record["ref"], "evidence": [record["ref"]],
        "applied_revision_ref": applied_refs[record["ref"]]} for record in failed_records],
    "addressed_problems": ["skipped-verification"] if instruction else [],
}
if any("Historical-only evidence" in json.dumps(record) for record in records):
    report["problems"][0]["evidence"] = data["known_problems"][0]["episodes"][0]["evidence"]
if "proposed_instruction" in data and any("Reject this proposal fixture" in json.dumps(record) for record in records):
    report["instruction"] = None
    report["addressed_problems"] = []
    report["summary"] = "The proposed guidance does not address this task. token=fixture-secret"
print(json.dumps({"type": "item.completed", "item": {
    "type": "agent_message", "text": json.dumps(report),
}}))
