# @choruz/web

The Next.js 16 / React 19 client: the dashboard and chat surface, the in-app docs site under `/docs`, and the Next.js API routes that front provisioning and filesystem work. It reaches the gateway through the `/api/v1/*` rewrite in `next.config.ts` and stays current over the `/v1/ws/sync` socket.

## Layout

- `app/` — App Router pages, the `api/` routes and the `styles/` CSS manifest
- `components/` — one folder per feature surface (`chat`, `channel-tasks`, `agents`, `groups`, `workspace`, `runtime`, `ui`, `pixel-world`)
- `hooks/` — the `use-*.ts` React hooks
- `lib/` — domain modules (`api`, `messages`, `agents`, `drivers`, `groups`, `channel-tasks`, `terminal`, `remote`, `workspace`)
- `plugins/` — the client plugin registry and one `client.tsx` per plugin
- `tests/` — Playwright e2e specs and fixtures; unit tests sit beside the module they pin
- `public/` — static assets

The provisioning route composes agent instructions from the repository's top-level [`agent-templates/`](../../agent-templates/), which the pipeline embeds as well.

The [Layout section of the web-client page](../../docs/subsystems/web-client.md#layout) says which folder a new module belongs in.

## Tests

`pnpm web:check` (typecheck), `pnpm web:test` (vitest), `pnpm web:e2e tests/e2e/<feature>.spec.ts` (Playwright against a full local stack).

## Related

- [docs/subsystems/web-client.md](../../docs/subsystems/web-client.md) — modules, data, invariants and the test harness
- [docs/architecture.md](../../docs/architecture.md)
