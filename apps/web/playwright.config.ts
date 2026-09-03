import { defineConfig, devices } from "@playwright/test";

const extendedProjects = process.env.CHORUZ_E2E_EXTENDED === "1"
  ? [{
      name: "chromium-reduced-motion",
      use: { ...devices["Desktop Chrome"], reducedMotion: "reduce" as const },
    }]
  : [];

const ci = Boolean(process.env.CI);

export default defineConfig({
  testDir: "./tests",
  timeout: 30000,
  retries: ci ? 1 : 0,
  // CI runners have 4 cores and the app under test is already running, so
  // more than one worker pays off; CHORUZ_E2E_WORKERS overrides.
  workers: ci ? Number(process.env.CHORUZ_E2E_WORKERS ?? 2) : undefined,
  // Without this a whole file is the unit of parallelism, so one long file
  // (messaging, user-journeys) runs alone at the end of a shard while the
  // other workers sit idle. Tests seed their own data with uniqueName and
  // the journeys are declared describe.serial, which keeps them together.
  fullyParallel: ci,
  forbidOnly: ci,
  reporter: process.env.CI ? [["github"], ["html", { open: "never" }]] : "list",
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    ...extendedProjects,
  ],
  use: {
    baseURL: process.env.CHORUZ_WEB_BASE_URL
      ?? `http://127.0.0.1:${process.env.CHORUZ_WEB_PORT ?? "3100"}`,
    channel: process.env.CHORUZ_PLAYWRIGHT_CHANNEL || undefined,
    // Recording a trace and a video for every test costs time on each one;
    // on CI (retries: 1) only the retry of a failed test records them.
    trace: ci ? "on-first-retry" : "retain-on-failure",
    screenshot: "only-on-failure",
    video: ci ? "on-first-retry" : "retain-on-failure",
  },
});
