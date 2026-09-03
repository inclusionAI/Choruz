# Remote Control

Remote Control extends the control plane of a running Choruz installation to another Web browser. It is separate from Remote SSH: Remote SSH changes where Choruz runs, while Remote Control changes where a person can observe and control it.

## Data policy

A paired browser runs the normal Choruz dashboard against the host: the same pages, terminals, boards and member lists the host's own browser shows. Every request and socket that dashboard opens crosses the Cloud Gateway as an end-to-end encrypted frame that the host executes locally, so the Cloud Gateway only handles opaque ciphertext and has no Agent input/output storage binding. Terminal output crosses the relay the same way, encrypted, never in plaintext.

The host bridge runs inside the API process, not inside an open Dashboard tab, so closing the local browser does not stop a paired browser from working. It keeps one connection to the Cloud Gateway per principal with a paired browser and executes that browser's calls with a session token it issues itself; the browser's own credentials never cross the relay.

A device is paired with one single-use credential, `v1.<128-bit id>.<128-bit secret>`, that expires after five minutes. The user pastes it once; there is no shorter fallback or second verification code. The Worker receives only the opaque identifier. The host API process and remote browser mix the secret into their committed P-256 ECDH exchange and prove possession over the transcript before the API redeems the credential. The local database stores only a keyed credential hash; the shared E2E session key is AES-GCM wrapped before storage. Devices can be revoked independently. Rotating the pairing/session secret intentionally invalidates existing paired devices.

## The Remote page

The remote dashboard is the `/remote` page of any Choruz web app (`apps/web/app/remote`). It needs no session cookie: it takes the Cloud Gateway URL, pairing credential and device name from the link or form, removes the secret-bearing URL fragment from browser history, runs the device side of the pairing handshake, keeps the resulting credentials in `localStorage` per gateway origin, and then installs the relay transport and renders the dashboard. Reopening the page reconnects without pairing again; **Disconnect** forgets the pairing, and a device revoked on the host is told so and returned to the form.

To control another Choruz computer from an existing Choruz Dashboard, open **Actions → Remote Control**, paste the credential printed by the other computer, and choose **Connect**: the dashboard opens its own `/remote` page with the credential in the fragment. A browser with no Choruz installed uses a hosted dashboard's `/remote` page; the Cloud Gateway redirects `GET /` and `/remote` there when its `REMOTE_DASHBOARD_URL` variable is set, and otherwise answers with the address to open.

## Network transport

Remote Control uses the Cloud Gateway so paired browsers can reconnect from any network. A future direct-LAN transport should only be exposed after its discovery, authentication, fallback, and browser compatibility paths have end-to-end coverage.

## Import Sessions and Create Agent from a paired browser

The paired browser's dashboard is the host's dashboard, so **Import Sessions** and **Create Agent** behave exactly as they do locally: the scan, the import and the provisioning run on the host through the same `/api/*` routes, relayed inside encrypted frames. The Cloud Gateway never sees a workspace path, session metadata or selected IDs in plaintext. Scanning is read-only and never launches a Harness; imported sessions resume only when messaged; provisioning secrets never reach the browser.

## Dashboard relay

Every same-origin call the dashboard makes crosses the Cloud Gateway as an encrypted frame that the host bridge executes locally (`services/choruz-api-gateway/src/remote_control_executor.rs`): `http.request` / `http.body` / `http.response` carry one `/api/*` request and its chunked body, and `stream.open` / `stream.data` / `stream.close` multiplex the dashboard's sync socket (`/v1/ws/sync`) and terminal sockets (`/v1/ws/terminals/*`) over the one transport socket. Bodies and socket messages travel in chunks of at most 384 KiB so every encrypted frame stays under the Gateway's 1 MB cap; a request body or socket message is capped at 16 MiB, at most 32 sockets and 64 chunked requests are in flight per paired browser, and a request times out after 30 s. The browser side is `apps/web/lib/remote/relay-transport.ts`.

The bridge authenticates each relayed call with a session token it issues for the paired principal, so the browser's own credentials never cross the relay and never reach the host. `/api/v1/*` goes to the local API gateway with a bearer token, any other `/api/*` path goes to the local Next.js server with the session cookie, and everything else is answered 404 without a local request. `/api/v1/remote-control/*` is answered 403: pairing, revoking and inspecting devices stay on the host's own browser.

## Hosted Gateway (default)

The default Gateway is operated by Choruz. A host needs no Cloudflare account, Worker deployment, or shared Gateway secret: run `choruz start`, wait for the displayed credential, then paste it on a Choruz `/remote` page. The command starts `choruz-server` in the background and returns only after the API process has joined the credential's Gateway room, so closing the shell or Dashboard pairing modal does not remove the host from that room.

The gateway is a Cloudflare Worker plus hibernating Durable Objects. A stable principal room is used only to rendezvous with already-paired browsers. Every host bridge lifecycle gets a random, ephemeral transport room; the host connects before advertising that room, allowing Cloudflare to place it near the host's current network rather than permanently pinning an account to its original geography. A network change normally drops the socket and creates a fresh room. While connected, three consecutive gateway RTT samples above 350 ms also rotate the room, rate-limited to once every five minutes. It does not run coding agents or store a development environment.

Hosted mode uses expiring, high-entropy room capabilities. The room capability
is not the end-to-end encryption key; message and control payloads remain
encrypted between the paired host and browser. No global Worker secret is
distributed to installations.

### Existing pairings

A pending pairing is accepted only when its host, browser and Worker use the current credential protocol. Already paired browsers keep their stored session credentials.

## Self-hosted Gateway (advanced)

To use your own Worker, configure matching secrets on the host and Worker:

```sh
export CHORUZ_REMOTE_CONTROL_GATEWAY_URL=https://choruz-remote-control-gateway.<account>.workers.dev
export CHORUZ_REMOTE_CONTROL_GATEWAY_SECRET="$(openssl rand -hex 32)"
printf '%s' "$CHORUZ_REMOTE_CONTROL_GATEWAY_SECRET" | pnpm --dir services/remote-control-gateway exec wrangler secret put GATEWAY_AUTH_SECRET
pnpm --dir services/remote-control-gateway deploy
```

`CHORUZ_REMOTE_CONTROL_GATEWAY_SECRET` and the Worker secret `GATEWAY_AUTH_SECRET` must contain the identical raw value and at least 32 UTF-8 bytes.

The persistent bridge calls the local API and Web server for every relayed
dashboard request and for authenticated management operations. Standard
installations use ports 3000 and 3100. Custom topologies can set
`CHORUZ_INTERNAL_API_URL` and `CHORUZ_INTERNAL_WEB_URL`; both must resolve to
trusted local endpoints and must never point through the public Cloud Gateway.

The gateway accepts expiring signed tickets for self-hosted mode and expiring
opaque room capabilities for hosted mode; it rate-limits pairing attempts per
client address before room lookup, caps frames at 1 MB, separates host/device
roles, requires encrypted envelopes in transport rooms, and uses Durable Object
WebSocket hibernation. It forwards opaque frames in memory and does not retain
Remote Control traffic.

## Pairing diagnostics

The API logs a pairing when it is issued and redeemed. Because the API process owns the host side of the one-time handshake, it records when the host socket opens and whether the attempt completes, expires, disconnects or fails. The Gateway logs the complementary credential submission, acceptance or rejection, WebSocket connection, pairing protocol message and close. Every event carries the opaque `pairing_id` generated by the host.

Logs record only the opaque pairing identifier and its format. They never record the credential secret, gateway tickets, ECDH material, session keys, proofs, device name or encrypted payload. Inspect hosted Worker events through the Cloudflare Worker logs. Inspect host lifecycle events through the local API Gateway process logs; Worker logs are useful only for cases that fail before the local host receives a pairing message.

## Approval boundary

Remote Control transports `approval_required` and `human_input_needed`, but it must not claim that a CLI permission was approved until the relevant harness exposes an approval broker and returns a successful command-result envelope. Harnesses currently launched with auto-approval remain observability/control sessions, not genuine per-tool remote approval sessions.
