import { check } from 'k6';
import http from 'k6/http';
import ws from 'k6/ws';

const baseUrl = __ENV.CHORUZ_BASE_URL || 'http://127.0.0.1:3000';
const wsBaseUrl = __ENV.CHORUZ_WS_BASE_URL || 'ws://127.0.0.1:3000';
const timeoutMs = Number(__ENV.K6_TIMEOUT_MS || 1500);

export const options = {
  vus: Number(__ENV.K6_VUS || 100),
  iterations: Number(__ENV.K6_ITERATIONS || 100),
  thresholds: {
    checks: ['rate==1.0'],
    http_req_failed: ['rate==0'],
    http_req_duration: ['p(95)<1000'],
    ws_connecting: ['p(95)<1000'],
  },
};

export default function () {
  const login = http.post(
    `${baseUrl}/v1/auth/local/login`,
    JSON.stringify({
      username: __ENV.CHORUZ_OPERATOR_USER || 'operator',
      password: __ENV.CHORUZ_OPERATOR_PASSWORD || 'choruz-local',
    }),
    { headers: { 'Content-Type': 'application/json' } },
  );

  const loggedIn = check(login, {
    'local login succeeded': (response) => response.status === 200,
    'login returned principal id': (response) => Boolean(response.json('principal.id')),
  });
  if (!loggedIn) {
    return;
  }

  const token = login.json('session_token');
  const bootstrap = http.get(`${baseUrl}/v1/bootstrap?limit=100`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  const bootstrapped = check(bootstrap, {
    'bounded bootstrap succeeded': (response) => response.status === 200,
    'bootstrap page is capped': (response) => response.json('conversations.items').length <= 100,
    'bootstrap returned sync cursor': (response) => Number.isFinite(response.json('sync_cursor')),
  });
  if (!bootstrapped) return;

  const cursor = bootstrap.json('sync_cursor');
  const deviceId = `k6-${__VU}-${__ITER}`;
  const syncUrl = `${wsBaseUrl}/v1/ws/sync?device_id=${encodeURIComponent(
    deviceId,
  )}&cursor=${encodeURIComponent(cursor)}`;
  let ready = false;

  const response = ws.connect(syncUrl, { headers: { Authorization: `Bearer ${token}` } }, (socket) => {
    socket.on('message', (data) => {
      const frame = JSON.parse(data);
      if (frame.type === 'sync_ready') {
        ready = true;
        socket.close();
      }
    });
    socket.setTimeout(() => socket.close(), timeoutMs);
  });

  check(response, {
    'sync websocket upgraded': (res) => res && res.status === 101,
  });
  check(ready, { 'sync ready received': (value) => value === true });
}
