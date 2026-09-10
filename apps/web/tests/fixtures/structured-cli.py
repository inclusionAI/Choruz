#!/usr/bin/env python3
"""Deterministic external CLI boundary; HTTP, authorization and host transport stay real."""
import json
import os
import pathlib
import sys
import uuid

codex = "app-server" in sys.argv
if "--version" in sys.argv:
    print("structured-fixture 1.0")
    sys.exit(0)

if "--ephemeral" in sys.argv or "--no-session-persistence" in sys.argv:
    if "exec" in sys.argv:
        assert "--ephemeral" in sys.argv and "--ignore-user-config" in sys.argv
    assert not pathlib.Path(".fixture-native.json").exists()
    request = json.loads(sys.stdin.read().strip().splitlines()[-1])
    if "task" in request:
        assert "expected" not in request and "check" not in request
        response = "CHECKED" if "Verify required checks" in request["guidance"] else "UNCHECKED"
        if all(name in request["preflight"] for name in ('"member":"derive"', '"member":"check"')):
            response = "TEAM_CHECKED"
    else:
        assert request["role"] == "Verify workspace changes before reporting completion." or request["role"].startswith("Plan task-specific observable checks")
        if request["request"] == "wait":
            import time
            time.sleep(60)
        response = request["role"] + " Inspect the changed files before reporting completion."
    if "exec" in sys.argv:
        print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": response}}))
    else:
        print(json.dumps({"type": "result", "is_error": False, "result": response}))
    sys.exit(0)

if "--dangerously-skip-permissions" in sys.argv and not sys.stdin.isatty():
    pathlib.Path("headless-review-input.json").write_text(json.dumps({"prompt": sys.argv[-1]}))
    print(json.dumps({"type": "result", "is_error": False, "result": "Execution fixture completed."}))
    sys.exit(0)

if sys.stdin.isatty():
    print("Raw terminal connected", flush=True)
    for line in sys.stdin:
        if line.strip() == "exit":
            break
        print(line.rstrip(), flush=True)
    sys.exit(0)

if not codex:
    assert "--permission-prompt-tool" in sys.argv
    assert sys.argv[sys.argv.index("--permission-prompt-tool") + 1] == "stdio"

workspace = pathlib.Path.cwd()
store = workspace / ".fixture-native.json"
saved = json.loads(store.read_text()) if store.exists() else {"id": str(uuid.uuid4()), "items": [], "claude": []}
for flag in ["--session-id", "--resume"]:
    if flag in sys.argv:
        saved["id"] = sys.argv[sys.argv.index(flag) + 1]
pending = None


def emit(value):
    print(json.dumps(value), flush=True)


def persist():
    store.write_text(json.dumps(saved))
    if not codex:
        root = pathlib.Path(os.environ.get("CLAUDE_CONFIG_DIR", str(pathlib.Path.home() / ".claude"))) / "projects" / "structured-fixture"
        root.mkdir(parents=True, exist_ok=True)
        (root / (saved["id"] + ".jsonl")).write_text("".join(json.dumps(row) + "\n" for row in saved["claude"]))


def claude(role, blocks):
    identifier = str(uuid.uuid4())
    row = {"type": role, "uuid": identifier, "parentUuid": saved["claude"][-1]["uuid"] if saved["claude"] else None, "cwd": str(workspace), "session_id": saved["id"], "message": {"id": identifier, "role": role, "content": blocks}}
    saved["claude"].append(row)
    emit(row)


def item(value):
    saved["items"].append(value)
    emit({"method": "item/completed", "params": {"item": value}})


def finish(text):
    if codex:
        item({"id": str(uuid.uuid4()), "type": "agentMessage", "text": text})
        emit({"method": "turn/completed", "params": {"turn": {"id": "turn", "status": "completed"}}})
    else:
        claude("assistant", [{"type": "text", "text": text}])
        emit({"type": "result", "is_error": False, "session_id": saved["id"]})
    persist()


for line in sys.stdin:
    event = json.loads(line)
    method = event.get("method")
    if method == "initialize":
        emit({"id": event["id"], "result": {}})
    elif method in ["thread/start", "thread/resume"]:
        if method == "thread/resume":
            saved["id"] = event["params"]["threadId"]
        emit({"id": event["id"], "result": {"thread": {"id": saved["id"], "turns": []}, "initialTurnsPage": {"data": [{"items": saved["items"]}], "nextCursor": None}}})
    elif event.get("type") == "control_request" and event["request"].get("subtype") == "initialize":
        emit({"type": "control_response", "response": {"subtype": "success", "request_id": "initialize", "response": {}}})
    elif method == "turn/interrupt" or event.get("request", {}).get("subtype") == "interrupt":
        pending = None
        if codex:
            finish("Stopped by user")
        else:
            emit({"type": "result", "is_error": True, "errors": ["[ede_diagnostic] result_type=user last_content_type=n/a stop_reason=tool_use"]})
    elif method == "turn/start" or event.get("type") == "user":
        text = event["params"]["input"][0]["text"] if codex else event["message"]["content"][0]["text"]
        if codex:
            emit({"method": "turn/started", "params": {"turn": {"id": "turn"}}})
            item({"id": event["id"], "type": "userMessage", "content": [{"type": "text", "text": text}]})
        else:
            claude("user", [{"type": "text", "text": text}])
        if text == "wait":
            continue
        pending = str(uuid.uuid4())
        if codex:
            emit({"method": "item/started", "params": {"item": {"id": pending, "type": "commandExecution", "command": "pwd", "status": "inProgress"}}})
            emit({"id": pending, "method": "item/commandExecution/requestApproval", "params": {"command": "pwd", "threadId": saved["id"], "turnId": "turn"}})
        else:
            claude("assistant", [{"type": "tool_use", "id": pending, "name": "Bash", "input": {"command": "pwd"}}])
            emit({"type": "control_request", "request_id": pending, "request": {"subtype": "can_use_tool", "tool_name": "Bash", "input": {"command": "pwd"}}})
    elif pending and (event.get("id") == pending or event.get("response", {}).get("request_id") == pending):
        allowed = event.get("result", {}).get("decision") == "accept" if codex else event["response"]["response"]["behavior"] == "allow"
        if allowed:
            (workspace / "approved-on-device").write_text(str(workspace))
        if codex:
            item({"id": pending, "type": "commandExecution", "command": "pwd", "aggregatedOutput": str(workspace) if allowed else "Denied", "status": "completed"})
        else:
            claude("user", [{"type": "tool_result", "tool_use_id": pending, "content": str(workspace) if allowed else "Denied", "is_error": not allowed}])
        pending = None
        finish("Verified workspace on selected device" if allowed else "Permission declined")
