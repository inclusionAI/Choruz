# Pixel World Gameplay Design

Branch: `chore/hpc-install-paths`

This document captures the intended product direction for `Pixel World` based on current discussion. The goal is to make the experience feel closer to a small multiplayer office game than a second copy of the chat UI.

## Core Goal

`Pixel World` should visualize team activity, room relationships, and conversational energy.

It should **not** try to render the full text conversation history inside the world.

The main chat UI remains the source of truth for:

- full message text
- detailed conversation history
- input / reply workflow
- agent execution details

`Pixel World` should instead answer these questions at a glance:

- which rooms are active right now
- who is talking to whom
- where each group lives in the office
- which other room just became active
- where the user's attention may want to go next

## Product Principles

1. `Pixel World` is a game-like overview, not a transcript viewer.
2. Speech in the world should be lightweight and ephemeral.
3. Room-to-room awareness matters more than exact message text.
4. The map should feel stable enough to learn like a game level.
5. Activity should read through motion, bubbles, highlights, and ambient signals.

## Messaging Visualization

### What should happen when someone speaks

When a user or agent sends a message in a room:

- show a short-lived speech bubble above the speaker
- optionally render just `...` or a very short snippet
- animate the speaker briefly
- animate the room as active

The bubble should disappear quickly. The world should not accumulate full text logs.

### What should not happen

- no long transcript blocks inside the world
- no persistent message stack over characters
- no attempt to mirror the full chat feed in `Pixel World`

### Preferred visual treatment

For most messages:

- bubble with `...`
- small talking animation
- temporary room pulse

For mentions or direct triggers:

- slightly stronger bubble or icon
- brighter room pulse
- optional agent "thinking" or "responding" animation

## Cross-Room Activity

If another room gets a new message while the user is focused elsewhere, that room should advertise activity like a game world:

- a bubble above the room
- a pulsing window / door / roof accent
- a small badge or icon above the house
- optional minimap ping in the future

The intent is:

- the user can keep walking around
- the office still feels alive
- other active groups are visible without opening them

This should feel closer to "something is happening in that house" than "I need to read that exact message here."

## Map and Spatial Design

The main frontend value of `Pixel World` is spatial organization.

The office should make people feel:

- each group has a place
- neighboring rooms feel meaningfully related
- the office is explorable like a game map

This means layout quality matters a lot:

- stable room placement is more important than procedural novelty
- adjacent groups should feel intentionally arranged
- roads / hallways should be readable and aesthetically coherent
- the user should be able to build a mental map over time

## Proposed Interaction Model

### Room = group

Keep this mapping.

Each group conversation should still map to one room.

### Character = active presence

Characters should represent current activity, not full membership.

That means:

- a character can belong to many groups in the data model
- but the map mainly shows where they are currently active
- membership can be shown via secondary UI later, not by duplicating the same character in many rooms

### Activity hierarchy

The world should prioritize:

1. room activation
2. speaking animation
3. cross-room notification
4. optional short bubble text

Detailed content is lower priority.

## Camera Direction

Camera behavior should support readability, not fight it.

Preferred behavior:

- opening `Pixel World` should orient the user clearly
- if a room is currently selected, a brief initial room emphasis is acceptable
- camera should then behave predictably

Two acceptable future models:

1. Focus the active room first, then stay there until the user moves.
2. Focus the player immediately and only highlight the active room visually.

Current behavior that briefly pans and then snaps back can feel ambiguous.

## Recommended Feature Changes

### High priority

1. Replace persistent in-world message text with ephemeral speech bubbles.
2. Add room-level "new activity" indicators for off-screen or inactive rooms.
3. Keep map layout stable across ordinary data refreshes.
4. Make camera behavior deterministic and easier to understand.

### Medium priority

1. Differentiate normal speech vs mention-triggered speech.
2. Add stronger room pulse / glow states.
3. Add lightweight membership display on room hover or click.

### Lower priority

1. Minimap or room badges
2. richer ambient office life
3. stronger game-like traversal goals

## Implications for Current Code

Current implementation already has useful building blocks:

- room model from group conversations
- agent animation state
- event-driven message / mention hooks
- room highlighting
- Phaser scene control

But the current world still leans too close to "data visualization layer" and not enough toward "game-like office activity layer."

The next implementation pass should shift emphasis from:

- exact message representation

toward:

- room activity signaling
- ephemeral speaking effects
- stable, readable spatial gameplay

## Summary

Desired direction:

- chat page = detailed communication surface
- `Pixel World` = lightweight live office simulation

Success looks like this:

- user opens the world
- sees where activity is happening
- notices a distant room light up or bubble
- understands that two people or agents are talking
- clicks into the room or switches back to chat for full detail

The world should feel alive, readable, and playful, not like a cramped duplicate of the conversation pane.
