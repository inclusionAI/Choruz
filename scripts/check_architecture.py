"""Check explicit source boundaries, not semantic equivalence of implementations.

Rules scan tracked production files (including new files when staged), never build
output. Diagnostics identify the contract and its owner. No baseline or suppression
list permits existing violations; intentional boundary changes update the rule and
its negative control together.
"""

import argparse
from dataclasses import dataclass
from fnmatch import fnmatchcase
from pathlib import Path
import re
import subprocess


@dataclass(frozen=True)
class Boundary:
    name: str
    owner: str
    scopes: tuple[str, ...]
    forbidden: str
    consumers: tuple[tuple[str, str], ...] = ()


GATEWAY = "services/choruz-api-gateway/src/"
APPLICATION = "crates/choruz-application/src/db_service/"
HEADLESS_CONSUMERS = (
    "services/choruz-pipeline/src/executor.rs",
    "services/choruz-connector/src/main.rs",
)

# Each pattern protects a known seam; it is not a general duplicate-code detector.
BOUNDARIES = (
    Boundary(
        "browser-api-transport", "apps/web/lib/api/transport.ts",
        ("apps/web/lib/api/*.ts", "apps/web/lib/groups/create-group-template-flow.ts"),
        r"(?<![\w.])(?:(?:globalThis|window)\s*\.\s*)?fetch\s*\(",
        tuple((path, r"\btransportFetch\s*\(") for path in (
            "apps/web/lib/api/choruz-api.ts",
            "apps/web/lib/groups/create-group-template-flow.ts",
        )),
    ),
    Boundary(
        "headless-turn", "crates/choruz-host-runtime/src/headless.rs",
        HEADLESS_CONSUMERS, r"\bCommand\s*::\s*new\s*\(",
        tuple((path, r"choruz_host_runtime\s*::\s*headless\s*::\s*run\s*\(")
              for path in HEADLESS_CONSUMERS),
    ),
    Boundary(
        "native-learning-evidence", GATEWAY + "learning_sources.rs",
        (GATEWAY + "experience_worker.rs", GATEWAY + "behavior_worker.rs"),
        r"\.\s*(?:retained_native_sources|learning_source_bindings)\s*\(",
        tuple((GATEWAY + file, r"learning_sources\s*::\s*native_sources\s*\(")
              for file in ("experience_worker.rs", "behavior_worker.rs")),
    ),
    Boundary(
        "learning-feedback", APPLICATION + "learning_feedback.rs",
        (APPLICATION + "behavior.rs",), r"\.\s*list_learning_feedback\s*\(",
        tuple((APPLICATION + file, r"\.\s*learning_feedback_records\s*\(")
              for file in ("experience.rs", "behavior.rs")),
    ),
    Boundary(
        "execution-identity", "crates/choruz-host-runtime/src/execution_identity.rs",
        (GATEWAY + "experience_diagnostics.rs", GATEWAY + "decision_worker.rs",
         GATEWAY + "evaluation_worker.rs"),
        r"\bstruct\s+ExecutionIdentity\b",
        tuple((GATEWAY + file, r"ExecutionIdentity\s*::\s*from\s*\(") for file in
              ("experience_diagnostics.rs", "decision_worker.rs", "evaluation_worker.rs")),
    ),
    Boundary(
        "database-webhook-outbox", APPLICATION + "events.rs",
        (GATEWAY + "webhook.rs",), r"\bChatApp\b|\.\s*session\b",
        ((GATEWAY + "webhook.rs", r"\bdb\s*\.\s*collect_pending_webhook_deliveries\s*\("),),
    ),
    Boundary(
        "driver-metadata", "apps/web/lib/drivers/driver-registry.ts",
        ("apps/web/lib/drivers/driver-availability.ts",),
        r"\b(?:const|let)\s+DRIVER_(?:DEFINITIONS|LABELS)\b",
        (("apps/web/lib/drivers/driver-availability.ts", r"\bDRIVER_DEFINITIONS\s*\.\s*map\s*\("),),
    ),
    Boundary(
        "cached-message-merge", "apps/web/lib/messages/messages.ts",
        ("apps/web/components/chat/chat-app.tsx",),
        r"\bidbMsgs\s*\.\s*length\s*>\s*existing\s*\.\s*length",
        (("apps/web/components/chat/chat-app.tsx", r"\bmergeFetchedMessages\s*\("),),
    ),
    Boundary(
        "snapshot-request-epoch", "apps/web/lib/messages/conversation-flags.ts",
        ("apps/web/components/chat/chat-app.tsx",),
        r"\bconst\s+applyBootstrap\s*=\s*useCallback\s*\((?:(?!=>)[\s\S])*=>\s*\{\s*"
        r"const\s+snapshotStartedAt\s*=\s*Date\s*\.\s*now\s*\(",
        (("apps/web/hooks/use-conversation-flags.ts", r"\.\s*applyOverrides\s*\("),),
    ),
    Boundary(
        "mention-lexer", "crates/choruz-common/src/mentions.rs",
        (APPLICATION + "messages.rs", "crates/choruz-router/src/policy.rs"),
        r"\bfn\s+(?:contains_mention(?:_with_candidates)?|mention_matches_at|is_mention_char)\s*\(",
        tuple((path, r"choruz_common\s*::\s*mentions\s*::") for path in (
            APPLICATION + "messages.rs", "crates/choruz-router/src/policy.rs",
        )),
    ),
    Boundary(
        "workspace-access", APPLICATION + "companies.rs",
        (GATEWAY + "handlers_cron.rs", GATEWAY + "handlers_runtime.rs"),
        r"\boperator\s*\.\s*workspace_id\s*==\s*agent\s*\.\s*workspace_id|"
        r"\bids\s*\.\s*insert\s*\(\s*p\s*\.\s*workspace_id\s*\.\s*clone\s*\(",
        tuple((GATEWAY + file, r"\.\s*accessible_workspace_ids\s*\(") for file in (
            "handlers_cron.rs", "handlers_runtime.rs",
        )),
    ),
    Boundary(
        "authentication-executable", "crates/choruz-agent-runtime/src/executable.rs",
        ("crates/choruz-harness-login/src/lib.rs",),
        r"\bfn\s+login_binary\b[^{]*\{(?:(?!\n\})[\s\S])*?"
        r"\bstd\s*::\s*env\s*::\s*var\s*\(",
        (("crates/choruz-harness-login/src/lib.rs",
          r"choruz_agent_runtime\s*::\s*executable\s*::\s*terminal_binary\s*\("),),
    ),
    Boundary(
        "host-service-readiness", "crates/choruz-common/src/lib.rs",
        ("apps/choruz-cli/src/main.rs", "crates/choruz-supervisor/src/supervisor.rs"),
        r"\basync\s+fn\s+api_ready\b[^{]*\{(?:(?!\n\})[\s\S])*?"
        r"\.\s*is_ok_and\s*\(\s*\|response\|\s*response\s*\.\s*status\s*\(\s*\)\s*==\s*StatusCode\s*::\s*OK",
        tuple((path, r"\.\s*validate_ready\s*\(") for path in (
            "apps/choruz-cli/src/main.rs", "crates/choruz-supervisor/src/supervisor.rs",
        )),
    ),
    Boundary(
        "bridge-channel-link", "services/choruz-bridge/src/mapping-store.ts",
        ("services/choruz-bridge/src/adapters/*.ts",),
        r"\.\s*createMapping\s*\(",
        tuple(("services/choruz-bridge/src/adapters/" + file, r"\.\s*getOrCreate\s*\(")
              for file in ("slack.ts", "telegram.ts")),
    ),
)

# Auth verification is server-only and cannot use the browser relay transport.
SERVER_AUTH = "apps/web/lib/api/api-auth.ts"
RETIRED = (
    "services/choruz-api-gateway/src/db_projection.rs",
    "crates/choruz-executor/src/adapter.rs",
    "crates/choruz-executor/src/codex_adapter.rs",
    "crates/choruz-executor/src/sandbox.rs",
)


def production_text(text: str, path: str) -> str:
    """Ignore literal text and Rust test modules; preserve executable interpolation.

    This lightweight lexical check deliberately does not resolve aliases, macros
    or data flow. Behavioral tests and the independent ownership audit cover those.
    """
    tokens = r"'(?:\\.|[^'\\\n])'|" + r'//[^\n]*|/\*[\s\S]*?\*/|\br(\#*)"[\s\S]*?"\1|"(?:\\[\s\S]|[^"\\])*"'
    if path.endswith((".ts", ".tsx")):
        tokens += r"|'(?:\\[\s\S]|[^'\\])*'"
    token = re.compile(tokens)
    masked = list(text)

    def hide(start: int, end: int):
        masked[start:end] = ["\n" if char == "\n" else " " for char in text[start:end]]

    def template(start: int) -> int:
        index = start + 1
        literal = start
        while index < len(text):
            if text[index] == "\\":
                index += 2
            elif text[index] == "`":
                hide(literal, index + 1)
                return index + 1
            elif text.startswith("${", index):
                hide(literal, index + 2)
                index = code(index + 2, expression=True)
                literal = index
            else:
                index += 1
        hide(literal, len(text))
        return len(text)

    def code(index: int, expression: bool = False) -> int:
        depth = 0
        while index < len(text):
            match = token.match(text, index)
            if match:
                hide(index, match.end())
                index = match.end()
                continue
            if path.endswith((".ts", ".tsx")) and text[index] == "`":
                index = template(index)
                continue
            if expression:
                if text[index] == "{":
                    depth += 1
                elif text[index] == "}":
                    if depth == 0:
                        hide(index, index + 1)
                        return index + 1
                    depth -= 1
            index += 1
        return index

    code(0)
    source = "".join(masked)
    if path.endswith(".rs"):
        for match in re.finditer(r"#\[cfg\(test\)\]\s*(?:pub\s+)?mod\s+\w+\s*\{", source):
            depth = 1
            index = match.end()
            while depth and index < len(source):
                depth += (source[index] == "{") - (source[index] == "}")
                index += 1
            hide(match.start(), index)
    return "".join(masked)


def check(root: Path, files: list[str]) -> list[str]:
    errors = []
    paths = set(files)
    sources = {}

    def source(path: str) -> str:
        if path not in sources:
            sources[path] = production_text((root / path).read_text(encoding="utf-8"), path)
        return sources[path]

    for path in RETIRED:
        if path in paths:
            errors.append(f"{path}: retired implementation; extend the shared owner instead")
    for rule in BOUNDARIES:
        if rule.owner not in paths:
            errors.append(f"{rule.name}: missing owner {rule.owner}; update the boundary on an intentional move")
        for path in sorted(paths):
            if path in (rule.owner, SERVER_AUTH) or ".test." in path:
                continue
            if not any(fnmatchcase(path, scope) for scope in rule.scopes):
                continue
            for match in re.finditer(rule.forbidden, source(path)):
                line = source(path).count("\n", 0, match.start()) + 1
                errors.append(f"{path}:{line}: {rule.name} bypass; use {rule.owner}")
        for path, pattern in rule.consumers:
            if path not in paths or not re.search(pattern, source(path)):
                errors.append(f"{path}: {rule.name} shared-owner wiring missing; use {rule.owner}")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    result = subprocess.run(["git", "ls-files", "-z"], cwd=args.root, check=True, capture_output=True)
    files = [path for path in result.stdout.decode().split("\0") if path and (args.root / path).is_file()]
    errors = check(args.root, files)
    for error in errors:
        print(error)
    if errors:
        print(f"Architecture boundaries: {len(errors)} violation(s)")
        return 1
    print(f"Architecture boundaries: {len(BOUNDARIES)} contracts checked; no violations")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
