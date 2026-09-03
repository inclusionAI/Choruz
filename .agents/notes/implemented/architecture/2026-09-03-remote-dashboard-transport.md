# Agent Note: The remote dashboard is the same dashboard over a relay transport

Status: implemented

## Problem

Remote Control used to end in a second product. "Connect" in the dashboard's Remote Control modal navigated to the Cloud Gateway's `/remote` page, a 50 KB HTML string inside `services/remote-control-gateway/src/index.ts` with its own styles, a message feed, an importer and an agent form. It covered a fraction of the dashboard, drifted from it, and every feature had to be built twice. The person's goal is the opposite: from any browser, open the home company in the normal dashboard, with its terminals, groups, machines and boards, and create groups that mix agents on the paired machines.

The relay is not a general tunnel and must not become one. It relays JSON frames of at most 1 MB, one socket per role per room, end-to-end encrypted so the Cloud Gateway only sees ciphertext, with a 30 s request timeout on both ends and no streaming. Proxying the Next.js server, its assets and two long-lived sockets through it would fight each of those limits and hand the Cloud Gateway plaintext pages.

## Decision

One dashboard, two transports. The web app talks to Choruz through exactly two seams, `transportFetch` for same-origin `/api/*` calls and `transportSocket` for `/v1/ws/sync` and `/v1/ws/terminals/*` (`apps/web/lib/api/transport.ts`); `ChoruzTransport.socket` returns `DashboardSocket`, the part of `WebSocket` the dashboard touches, so a browser socket and a relayed one are interchangeable at the seam. The local transport is the default. A relay transport implements the same interface over the existing end-to-end channel, so the remote dashboard is the same React tree with a different transport installed.

- **Transport seam.** Every client-side call in `apps/web` goes through `transportFetch` / `transportSocket`; server code keeps plain `fetch`, and `transportFetch` falls back to `fetch` on the server so shared helpers keep working in route handlers.
- **Host bridge as a request executor.** `remote_control_executor.rs` handles the generic frame kinds the bridge dispatches to it. `http.request` / `http.body` / `http.response` carry an allow-listed same-origin call (`/api/v1/*` to the gateway with a bearer token, other `/api/*` to the home Next server with the session cookie, `/api/v1/remote-control/*` refused so device management stays on the host) with bodies chunked at 384 KiB under the frame cap and authenticated with a session token the bridge issues per call for the principal. `stream.open` / `stream.opened` / `stream.data` / `stream.close` multiplex the two gateway sockets over the one transport socket, text and binary alike as base64 chunks, so the sync feed and terminals cross the relay unprojected. Requests run in their own tasks and answer through one outbound channel; a `device.hello` resets the executor because the browser that owned the streams is gone. The bridge itself only offers the transport room, marks `device.hello` and feeds the executor; the projected message feed, `sync.ack`, `message.send`, the dashboard snapshot and the sessions and agent operations it used to proxy are gone with the page that needed them.
- **Relay transport in the web app.** `apps/web/lib/remote/` holds `relay-pairing.ts` (the device side of the committed ECDH handshake, tested against a scripted host, plus per-gateway credential storage), `relay-session.ts` (rendezvous socket, transport offer, `device.hello`, envelopes encrypted and decrypted in order so chunked frames stay ordered, reconnect and revocation) and `relay-transport.ts` (`createRelayTransport(link)`, a `ChoruzTransport` whose `RelaySocket` implements `DashboardSocket` over `stream.*` frames). The transport fails every in-flight request and closes every socket whenever the session leaves `connected`, matching the host executor's reset.
- **Remote entry.** `apps/web/app/remote` is a client-rendered page that pairs (or reconnects with stored credentials), installs the relay transport, fetches the bootstrap through it and renders `ChatApp` with the same props `app/dashboard` computes (`lib/api/dashboard-snapshot.ts` is the shared mapping). The Remote Control modal's **Connect** opens the dashboard's own `/remote?gateway=…&device_name=…#credential=…`; the secret-bearing credential fragment is consumed and removed from browser history before pairing. The Cloud Gateway serves no page: `GET /` and `/remote` redirect to a hosted dashboard's `/remote` when `REMOTE_DASHBOARD_URL` is set and otherwise say where that page lives. `DASHBOARD_HTML`, the Worker's inline page, and the web route `app/api/agents/remote-provision` that only it used are deleted.

## Alternatives considered

- **Reverse-proxy the home Next.js server through the Cloud Gateway** (Cloudflare Tunnel style): rejected. The Cloud Gateway would see every page and API response in plaintext, which reverses the stated invariant that it only relays ciphertext.
- **A VPN (Tailscale, WireGuard) and no relay**: rejected as the product answer. It works for the person who runs Choruz, not for a phone that only has a browser and a pairing credential; it stays a documented option.
- **Keep the inline page and grow it**: rejected. It is the second product this note removes.
- **A service worker that tunnels the whole origin**: rejected. The first navigation cannot go through a worker that is not installed yet, and the app's sockets cannot be intercepted by one at all; a transport seam in the app is smaller and testable.
- **Serve the dashboard from the Cloud Gateway Worker itself** (Next.js on Workers inside `remote-control-gateway`): deferred. It couples a 500-line relay Worker to the web app's build, and a browser with a Choruz install already has a `/remote` page; the gateway's `REMOTE_DASHBOARD_URL` redirect leaves room for a hosted dashboard deployed on its own.

## Consequences

- The remote dashboard cannot drift from the local one: there is one React tree, and a feature exists remotely the moment it exists locally. Group creation, boards, terminals and the member machine badges all work from a paired browser without code of their own.
- The relay keeps its shape (JSON, ciphertext, 1 MB frames); chunking and multiplexing live in the executor and the relay transport. The executor's tests run its frames against an in-process axum server standing in for the gateway and the Next server; the relay transport's tests drive the same frame shapes from TypeScript with scripted sockets and real WebCrypto.
- Terminal output crosses the relay only inside encrypted `stream.data` frames; the Cloud Gateway never sees it. The data-policy section of `docs/operations/remote-control.md` states this.
- A client call that bypasses the seam (a bare `fetch` in a component, a `new WebSocket` in a hook) works locally and silently fails remotely; `apps/web/lib/api/transport.test.ts` pins the seam and the `/remote` e2e spec exercises the entry page, so the remaining guard is review.
- A browser with no Choruz install needs a hosted dashboard; until one is deployed, the Cloud Gateway's `/remote` explains where to go instead of serving a page. This is the trade the deferred alternative above accepts.

## Testing

- `services/choruz-api-gateway/src/remote_control_executor.rs`: allow-list, chunked bodies both ways, multiplexed text and binary streams, reset, unreachable targets.
- `apps/web/lib/remote/relay-{transport,session,pairing}.test.ts`, `apps/web/lib/api/transport.test.ts`, `apps/web/lib/remote/remote-control.test.ts`.
- `apps/web/tests/e2e/remote-dashboard.spec.ts`: the `/remote` entry form, link prefill and the error an unreachable gateway produces.
- `services/remote-control-gateway/src/remote-entry.test.ts`: the gateway's redirect and fallback text.
