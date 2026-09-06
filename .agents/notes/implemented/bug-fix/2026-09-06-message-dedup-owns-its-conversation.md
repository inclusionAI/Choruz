# Agent Note: Message deduplication tests own their conversation

Status: implemented

## Problem

Selecting the first existing group let the message-dedup browser test write into
another worker's attachment-draft group. That worker correctly expected its
unsent draft to leave the conversation empty, but found the dedup test's text.

## Decision

The dedup test creates its own person, Company and group, and deletes that
Company in cleanup. It waits for the persisted message's canonical DOM identity
before asserting one bubble; neither a sleep nor an optimistic echo proves sync
deduplication. Screenshots use the test's own output directory.

## Alternatives considered

Serializing the suite or selecting a different pre-existing group still makes
the test depend on another scenario's state. An absent fixture is not a reason
to skip the behaviour: the test owns the fixture it needs.

## Consequences

The scenario can run beside draft tests and in repeated parallel runs without
changing their conversations. The assertion checks both persisted cardinality
and visible canonical identity.

## Related

- [Test isolation](../../../skills/choruz-ci-test-reliability/SKILL.md)
