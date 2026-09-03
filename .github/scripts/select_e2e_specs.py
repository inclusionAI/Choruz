#!/usr/bin/env python3

"""Pick the Playwright specs a change has to run.

Reads the changed file paths (one per line on stdin, or as arguments) and
prints, as GitHub Actions outputs, the spec files to run and how many shards
they deserve. A change outside apps/web, or to a web file no rule below
knows to be self-contained, means the whole P0 smoke set. A change confined
to a feature runs that feature's specs (which may include specs outside the
P0 set) plus the app smoke test.
"""

from __future__ import annotations

import argparse
import fnmatch
import math
import re
import sys
from pathlib import Path

WEB = "apps/web/"
SMOKE = "tests/e2e/app-smoke.spec.ts"

# The always-on subset a pull request runs when its change is not contained.
P0_SPECS = (
    "tests/e2e/auth.spec.ts",
    "tests/e2e/company.spec.ts",
    "tests/e2e/agent.spec.ts",
    "tests/e2e/terminal.spec.ts",
    "tests/e2e/api-routes.spec.ts",
    "tests/e2e/messaging.spec.ts",
    "tests/e2e/outbox.spec.ts",
    "tests/e2e/conversation.spec.ts",
    "tests/e2e/websocket.spec.ts",
    "tests/e2e/attachment.spec.ts",
    "tests/e2e/machines.spec.ts",
)

# Web files whose effect stays inside one feature, and the specs that cover
# the feature. Paths are relative to apps/web. Anything under apps/web that
# no rule matches (chat-app, sidebar, chat-input, shared lib, styles, config,
# fixtures) is treated as touching everything.
RULES: tuple[tuple[tuple[str, ...], tuple[str, ...]], ...] = (
    (
        ("components/workspace/git-graph*", "lib/workspace/git-graph-repo-path.ts", "app/api/git-graph/**"),
        ("tests/e2e/git-graph.spec.ts",),
    ),
    (
        (
            "components/workspace/file-tree.tsx",
            "components/workspace/file-editor.tsx",
            "components/workspace/path-picker.tsx",
            "components/workspace/folder-picker-modal.tsx",
            "app/api/filesystem/**",
            "lib/workspace/workspace-path-guard.ts",
        ),
        ("tests/e2e/file-explorer.spec.ts", "tests/e2e/file-editor.spec.ts", "tests/e2e/editor-tabs.spec.ts"),
    ),
    (
        ("components/pixel-world/**",),
        (
            "tests/e2e/pixel-world.spec.ts",
            "tests/e2e/pixel-world-e2e.spec.ts",
            "tests/e2e/pixel-world-roster.spec.ts",
            "tests/e2e/pixel-world-state-mapping.spec.ts",
        ),
    ),
    (
        ("components/chat/detail-panel.tsx", "components/chat/member-row.tsx"),
        ("tests/e2e/detail-panel.spec.ts", "tests/e2e/conversation.spec.ts"),
    ),
    (
        ("components/channel-tasks/**", "components/chat/channel-conversation-tabs.tsx", "lib/channel-task*"),
        ("tests/e2e/channel-tasks.spec.ts",),
    ),
    (
        ("components/chat/thread-panel.tsx", "lib/messages/threads.ts", "lib/messages/thread-unreads.ts"),
        ("tests/e2e/threads.spec.ts", "tests/e2e/messaging.spec.ts"),
    ),
    (
        (
            "components/runtime/server-manager.tsx",
            "components/runtime/runtime-host-manager.tsx",
            "components/runtime/runtime-status-panel.tsx",
            "components/runtime/remote-control-manager.tsx",
            "components/remote/**",
            "app/remote/**",
            "lib/remote/**",
            "lib/runtime-window.ts",
        ),
        ("tests/e2e/server.spec.ts", "tests/e2e/machines.spec.ts", "tests/e2e/remote-dashboard.spec.ts"),
    ),
    (
        ("components/ui/theme-provider.tsx",),
        ("tests/e2e/theme.spec.ts", "tests/e2e/chat-input-theme.spec.ts"),
    ),
    (
        ("app/docs/**",),
        ("tests/e2e/docs.spec.ts",),
    ),
    (
        (
            "components/agents/create-agent-modal.tsx",
            "components/groups/create-group-modal.tsx",
            "components/groups/create-company-modal.tsx",
            "components/agents/driver-model-picker.tsx",
            "components/agents/harness-account*",
            "components/agents/agent-*",
            "components/agents/import-workspace-sessions-modal.tsx",
            "lib/create-*-template-flow.ts",
            "lib/team-template*",
            "lib/driver-*",
            "lib/harness-account*",
            "lib/agent-*",
            "lib/group-provisioning*",
            "lib/ai-manager-*",
            "app/api/harness-accounts/**",
            "app/api/drivers/**",
            "app/api/agents/**",
            "app/api/companies/**",
            "app/api/group-provisioning-jobs/**",
            "app/api/skills/**",
            "app/api/agent-skills/**",
            "app/api/agent-config/**",
        ),
        (
            "tests/e2e/modals.spec.ts",
            "tests/e2e/agent.spec.ts",
            "tests/e2e/company.spec.ts",
            "tests/e2e/workspace-session-import.spec.ts",
        ),
    ),
    (
        ("components/runtime/terminal-view.tsx", "lib/terminal/terminal-write-buffer.ts", "lib/terminal/ansi.ts"),
        ("tests/e2e/terminal.spec.ts",),
    ),
    (
        (
            "components/chat/message-bubble.tsx",
            "components/chat/message-list.tsx",
            "lib/messages/messages.ts",
            "lib/messages/message-db.ts",
            "lib/format-chat-time.ts",
            "lib/messages/quotes.ts",
        ),
        (
            "tests/e2e/messaging.spec.ts",
            "tests/e2e/message-list.spec.ts",
            "tests/e2e/indexeddb.spec.ts",
            "tests/e2e/websocket.spec.ts",
            "tests/e2e/attachment.spec.ts",
            "tests/e2e/message-dedup.spec.ts",
            "tests/e2e/quotes.spec.ts",
        ),
    ),
    (
        ("components/chat/chat-input.tsx", "app/api/attachments/**", "lib/audio-utils.ts", "lib/format-bytes.ts"),
        ("tests/e2e/messaging.spec.ts", "tests/e2e/attachment.spec.ts", "tests/e2e/outbox.spec.ts"),
    ),
    (
        ("components/chat/chat-header.tsx",),
        ("tests/e2e/chat-header.spec.ts", "tests/e2e/conversation.spec.ts", "tests/e2e/detail-panel.spec.ts"),
    ),
    # Unit tests, stories and docs do not change what the browser sees.
    (("**/*.test.ts", "**/*.test.tsx", "**/*.md"), ()),
)

# Roughly how many tests one shard should carry; shards are capped at three.
TESTS_PER_SHARD = 20
MAX_SHARDS = 3

# Files that no check has to run for: prose only.
DOCS_PATTERNS = (
    "docs/**",
    "**/*.md",
    "**/*.mdx",
    "LICENSE*",
    ".github/CODEOWNERS",
    ".editorconfig",
    ".gitignore",
    ".nvmrc",
)

# Web unit tests (vitest): `vitest related` runs the tests that import the
# changed files. A change to the harness itself means the whole suite.
VITEST_ALL_PATTERNS = (
    "apps/web/vitest.config.*",
    "apps/web/package.json",
    "apps/web/tsconfig.json",
    "apps/web/next.config.*",
    "package.json",
    "pnpm-lock.yaml",
    "pnpm-workspace.yaml",
    ".github/workflows/**",
    ".github/actions/**",
    ".github/scripts/**",
)
VITEST_SOURCE_DIRS = ("lib/", "components/", "app/", "plugins/")
VITEST_SOURCE_SUFFIXES = (".ts", ".tsx", ".js", ".jsx", ".mjs")


def vitest_selection(changed: list[str]) -> tuple[str, list[str]]:
    """Return ("all" | "related" | "none", files relative to apps/web)."""
    paths = [p.strip() for p in changed if p.strip()]
    if any(_matches(p, pattern) for p in paths for pattern in VITEST_ALL_PATTERNS):
        return "all", []
    related: list[str] = []
    for path in paths:
        for pattern, test_file in VITEST_EXTRA_RELATED:
            if _matches(path, pattern) and test_file not in related:
                related.append(test_file)
        if not path.startswith(WEB):
            continue
        rel = path[len(WEB):]
        if rel.startswith(VITEST_SOURCE_DIRS) and rel.endswith(VITEST_SOURCE_SUFFIXES) and rel not in related:
            related.append(rel)
    return ("related", related) if related else ("none", [])


# Prose that a gate still has to read: an Agent Note is Markdown, but the
# notes verifier must run on it, so it never counts as documentation-only.
# The agent instruction fragments are Markdown too, but the pipeline embeds
# them and both the pipeline fixtures and the web template tests pin them.
GATED_PROSE_PATTERNS = (".agents/**", "agent-templates/**")

# Files outside apps/web whose change a web unit test pins.
VITEST_EXTRA_RELATED = (("agent-templates/**", "lib/agents/agent-templates.test.ts"),)


def docs_only(changed: list[str]) -> bool:
    """True when every changed file is documentation (and there is at least one)."""
    paths = [p.strip() for p in changed if p.strip()]
    if not paths:
        return False
    return all(
        any(_matches(p, pattern) for pattern in DOCS_PATTERNS)
        and not any(_matches(p, pattern) for pattern in GATED_PROSE_PATTERNS)
        for p in paths
    )


def _matches(path: str, pattern: str) -> bool:
    """fnmatch with `**` allowed to span directories."""
    if pattern.endswith("/**"):
        return path.startswith(pattern[:-2]) or path == pattern[:-3]
    if pattern.startswith("**/"):
        return fnmatch.fnmatchcase(path, pattern[3:]) or fnmatch.fnmatchcase(path, pattern)
    return fnmatch.fnmatchcase(path, pattern)


def specs_for_change(changed: list[str]) -> tuple[bool, list[str]]:
    """Return (everything, specs). `everything` means the change is not
    contained by the rules; `specs` is the ordered, de-duplicated list of
    specs the contained part of the change maps to. When `everything` is
    true the caller adds the P0 set to those specs: a change that touches a
    feature and something unmapped still runs the feature's own specs."""
    everything = False
    selected: list[str] = []

    def add(spec: str) -> None:
        if spec not in selected:
            selected.append(spec)

    for raw in changed:
        path = raw.strip()
        if not path:
            continue
        if not path.startswith(WEB):
            everything = True
            continue
        rel = path[len(WEB):]
        if rel.startswith("tests/"):
            if rel.endswith(".spec.ts"):
                add(rel)
                continue
            everything = True  # fixtures and helpers feed every spec
            continue
        for patterns, specs in RULES:
            if any(_matches(rel, pattern) for pattern in patterns):
                for spec in specs:
                    add(spec)
                break
        else:
            everything = True
    if selected:
        add(SMOKE)
    return everything, selected


def count_tests(root: Path, specs: list[str]) -> int:
    total = 0
    for spec in specs:
        try:
            text = (root / WEB / spec).read_text(encoding="utf-8")
        except OSError:
            continue
        total += len(re.findall(r"^\s*test\(", text, re.MULTILINE))
    return total


def shard_count(tests: int) -> int:
    return max(1, min(MAX_SHARDS, math.ceil(tests / TESTS_PER_SHARD)))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("files", nargs="*", help="changed paths (default: stdin, one per line)")
    parser.add_argument("--all", action="store_true", help="select the P0 set regardless of the change")
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()

    changed = args.files if args.files else [line for line in sys.stdin.read().splitlines()]
    prose = not args.all and docs_only(changed)
    if args.all:
        everything, specs = True, []
    elif prose:
        everything, specs = False, []
    else:
        everything, specs = specs_for_change(changed)
    if everything:
        specs = list(P0_SPECS) + [spec for spec in specs if spec not in P0_SPECS]
        shards = MAX_SHARDS
    elif specs:
        shards = shard_count(count_tests(args.root, specs))
    else:
        shards = 0

    print(f"specs={' '.join(specs)}")
    print(f"shard_count={shards}")
    # The matrix must never be empty even when the job is skipped.
    print(f"shards={list(range(1, max(shards, 1) + 1))}")
    print(f"everything={'true' if everything else 'false'}")
    print(f"docs_only={'true' if prose else 'false'}")
    vitest, vitest_files = ("all", []) if args.all else vitest_selection(changed)
    print(f"vitest={vitest}")
    print(f"vitest_files={' '.join(vitest_files)}")


if __name__ == "__main__":
    main()
