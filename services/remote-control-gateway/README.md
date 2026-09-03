# @choruz/remote-control-gateway

Cloudflare Worker (`wrangler.toml` name `choruz-remote-control-gateway`) that relays end-to-end-encrypted Remote Control traffic between a Choruz host and a paired browser. `src/index.ts` answers `GET /healthz`, `POST /v1/capabilities`, `GET /` and `/remote` (a redirect to the hosted dashboard's `/remote` page when `REMOTE_DASHBOARD_URL` is set, else a pointer to it), and the `/connect` WebSocket upgrade; three Durable Objects hold the state: `GatewayRoom` (one room per host, relaying frames and answering `gateway.ping` probes), `PairingRateLimiter` and `CapabilityStore`. Tickets are HMAC-signed with `GATEWAY_AUTH_SECRET`, the same value as the host's `CHORUZ_REMOTE_CONTROL_GATEWAY_SECRET`.

## Entry points

- `src/index.ts` — the fetch handler and the Durable Object classes
- `src/tickets.ts` — `verifyGatewayTicket`, `GatewayTicketPayload`
- `src/capability.ts` — `validCapability`
- `src/control.ts` — control-frame helpers (`gateway.ping` / `gateway.pong`, encrypted-frame detection, device revocation)
- `src/remote-entry.ts` — `remoteEntryResponse`, what `GET /` and `/remote` answer

## Tests

`pnpm --dir services/remote-control-gateway check` (`tsc --noEmit`) and `pnpm --dir services/remote-control-gateway test` (vitest); `dev` and `deploy` run wrangler.

## Related

- [docs/subsystems/host-and-remote.md](../../docs/subsystems/host-and-remote.md) — pairing, tickets and the host side of the relay
- [docs/architecture.md](../../docs/architecture.md)
