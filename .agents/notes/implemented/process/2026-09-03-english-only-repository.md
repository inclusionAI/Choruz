# Agent Note: English-only repository

Status: implemented

## Problem

Mixed-language documentation, comments, tests, and locale-dependent date formatting make the public project inconsistent and can show untranslated interface fragments based on the browser locale.

## Decision

All repository prose and user-visible interface text use English. Date and time labels select the `en-US` locale explicitly instead of inheriting the browser locale. Unicode coverage uses Latin extended characters and emoji.

## Alternatives considered

- **Keep browser-locale date formatting** — rejected because Choruz does not ship complete translations, so locale-sensitive dates would create a partially translated interface.
- **Keep a bilingual README** — rejected because maintaining parallel prose in one file duplicates facts and makes the primary contributor entry point harder to scan.
- **Add a repository-wide language gate to every pull request and push** — rejected because it would broaden CI for a prose convention that review can enforce.

## Consequences

Contributors and users get one consistent project language across documentation, code comments, test descriptions, and the interface. Users outside the United States see English date ordering rather than their browser's local format until Choruz supports complete localization.
