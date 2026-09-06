import { expect, test } from "@playwright/test";

test("pipeline serves health and metrics without the retired fanout endpoint", async ({ request }) => {
  const port = process.env.CHORUZ_PIPELINE_METRICS_PORT;
  expect(port, "the host stack must supply its owned pipeline port").toBeTruthy();
  const base = `http://127.0.0.1:${port}`;
  const ready = await request.get(`${base}/readyz`);
  expect(ready.status()).toBe(200);
  expect(await ready.json()).toMatchObject({ service: "choruz-pipeline", status: "ready", database: true });
  const health = await request.get(`${base}/healthz`);
  expect(health.status()).toBe(200);
  expect(await health.json()).toMatchObject({ service: "choruz-pipeline", status: "ok" });
  const metrics = await request.get(`${base}/metrics`);
  expect(metrics.status()).toBe(200);
  expect(metrics.headers()["content-type"]).toContain("text/plain");
  expect((await request.get(`${base}/ws/fanout`)).status()).toBe(404);
});
