# Agent Note: Demo recordings preserve their evidence chain

Status: implemented

## Problem

A polished walkthrough can conceal a failed transport, mix unrelated runs or show a different build from the one claimed. Recording an authenticated browser also risks exposing credentials and private conversations. A product overview needs truthful evidence without imposing media production on every GUI change.

## Decision

The [recording skill](../../../skills/choruz-record-demo/SKILL.md) separates functional acceptance, capture, encoded-artifact verification and optional publication. Each scenario retains one causal run, with exact local and remote revisions. Edited chapters and state-based recordings are labelled rather than presented as continuous real-time execution. Generated media stays outside source branches.

The workflow adapts the evidence-chain principles in DSH's [record-browser-gif skill](https://github.com/deepseek-ai/deepseek-harness/blob/d347e703908d0406b7a7ef80e3a0e594d86b2215/.agents/skills/record-browser-gif/SKILL.md). It uses Choruz's available recording surface and startup workflow instead of copying DSH-specific state roots, build commands or GIF publication requirements.

## Alternatives considered

**Require a GIF on every GUI PR.** This expands a requested demo workflow into a repository-wide merge requirement and adds production cost unrelated to the requested change.

**Assemble successful states from unrelated attempts.** The resulting sequence can imply delivery or recovery that never occurred in one interaction. Explicit chapters preserve independent scenarios without fabricating causality.

**Verify only source screenshots.** Encoding can change crop, order, timing and legibility. Inspecting the output tests the artifact the viewer actually receives.

## Consequences

The skill supports both continuous video and explicitly edited walkthroughs without inventing unavailable capture APIs. Recording takes additional real-product runs, and access problems remain visible limitations rather than being replaced with mocks. Publication requires separate scope and verification; a local demo is not automatically uploaded.
