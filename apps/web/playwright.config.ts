/**
 * Browser end-to-end tests (M10) against a running MindGraph stack: no API
 * mocks. Point them at the web app with MINDGRAPH_E2E_WEB_URL (default
 * http://127.0.0.1:3000) after loading the fraud demo:
 *
 *   docker compose up -d --build && make demo-fraud
 *   cd apps/web && npm run test:e2e
 *
 * `scripts/smoke-stack.sh --e2e` does all of this on an isolated stack.
 * The tests create their own graphs (named "E2E …") and only read the demo.
 */

import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./e2e",
  // One worker: the journeys share one stack and its background job queue.
  workers: 1,
  fullyParallel: false,
  timeout: 120_000,
  expect: { timeout: 20_000 },
  retries: process.env.CI ? 1 : 0,
  forbidOnly: Boolean(process.env.CI),
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: process.env.MINDGRAPH_E2E_WEB_URL ?? "http://127.0.0.1:3000",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
        viewport: { width: 1440, height: 900 },
        // The Graph Explorer draws with WebGL (Sigma.js). Headless runners
        // have no GPU, so Chromium renders WebGL in software (SwiftShader).
        launchOptions: { args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] },
      },
    },
  ],
});
