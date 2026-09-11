#!/usr/bin/env python3
"""Replace only model responses for the automatic trace-to-evaluation journey."""
import json
import sys

data = json.loads(sys.stdin.read().strip().splitlines()[-1])
if "cases" in data:
    decisions = []
    for case in data["cases"]:
        decision = {"episode_ref":case["episode_ref"], "accepted":True, "sensitive":False, "evidence":case["evidence"], "reason":"Original explicit answer supports this task; trial correctness does not determine admission.", "repair":None}
        if case["input"] == "Ambiguous increment 0":
            decision.update(accepted=False, reason="The operation is ambiguous.", repair={"input":"Increment 0", "check":{"type":"exact","expected":"1"}, "reason":"Restore the explicit source operation."})
        elif case["input"] == "Repair increment 3":
            decision.update(accepted=False, reason="Context missing.", repair={"input":"Still ambiguous 3", "check":{"type":"exact","expected":"4"}, "reason":"Proposed repair still needs review."})
        elif case["input"] in ["Read missing file", "Still ambiguous 3"]:
            decision.update(accepted=False, reason="Required context is unavailable.")
        elif case["input"] == "SYNTHETIC_CREDENTIAL_4":
            decision.update(accepted=False, sensitive=True, reason="Synthetic sensitive data", repair={"input":"Increment 4", "check":{"type":"exact","expected":"5"}, "reason":"Must not retry a sensitive task"})
        elif case["input"].startswith("Change "):
            assert "original_cases" in data["context"]
            decision.update(accepted=False, reason="The variant changes the requested operation.")
        assert case["episode_ref"] in data["trials"]
        decisions.append(decision)
    answer = {"decisions":decisions}
elif "trace" in data:
    cases = []
    for row in data["trace"]["records"]:
        text = json.dumps(row)
        if "BENCHMARK:" not in text:
            continue
        import re
        number = int(re.search(r"BENCHMARK: (\d+)", text)[1])
        cases.append({"episode_ref": row["ref"], "evidence": [row["ref"]],
                      "classification": {"task_type":"math","capability":"reasoning","structure":"single_step","outcome":"direct","related_refs":[]},
                      "input": {0:"Ambiguous increment 0",1:"Read missing file",3:"Repair increment 3",4:"SYNTHETIC_CREDENTIAL_4"}.get(number,f"Increment {number}"), "check": {"type": "exact", "expected": str(number+1)},
                      "reason": "Explicit user task and exact answer in source"})
    import hashlib
    invalid_assigned = False
    for case in cases:
        n = int(case["check"]["expected"])-1
        training = hashlib.sha256(case["episode_ref"].encode()).digest()[0] % 3 == 0
        invalid = training and n not in [1,3,4] and not invalid_assigned
        invalid_assigned |= invalid
        case["variant"] = f"Change {n} by two" if invalid else f"Increase {n} by one"
    answer = {"summary": "Independent increment objectives with explicit answers.", "instruction": None,
              "evidence": [case["episode_ref"] for case in cases], "previous_revision_outcome": "not_observed",
              "problems": [], "addressed_problems": [], "evaluation_cases": cases}
elif "component" in data:
    answer = {"text": "Return the number only."}
elif "seed_evidence" in data:
    answer = {"accepted": True, "evidence": [data["seed_reference"]]}
else:
    assert "check" not in data and "source" not in data and "expected" not in data
    if data["task"].startswith("Independently attempt each task"):
        tasks = json.loads(data["task"].splitlines()[-1])["task_trials"]
        assert all(set(t)=={"episode_ref","input"} for t in tasks), "blind trial must not receive references or answers"
        answer = {t["episode_ref"]:"wrong trial answer" if t["input"]=="Increment 2" else "Trial attempt for "+t["input"] for t in tasks}
    else:
        result = str(int(data["task"].split()[1])+1)
        answer = result if data["guidance"] else "The answer is " + result
print(json.dumps({"type":"item.completed","item":{"type":"agent_message","text":json.dumps(answer) if isinstance(answer,dict) else answer}}))
