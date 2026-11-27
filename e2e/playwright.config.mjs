// Browser tests. Before running: build the server (cargo build --release
// -p heistarr-server) and the web UI (./scripts/build-web.sh).
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  // The tests share one server and build on each other's state.
  fullyParallel: false,
  workers: 1,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: "http://127.0.0.1:6499",
    trace: "off",
    screenshot: "only-on-failure",
  },
  projects: [
    { name: "desktop", use: { ...devices["Desktop Chrome"], viewport: { width: 1360, height: 900 } } },
  ],
  webServer: {
    command: "./start.sh",
    url: "http://127.0.0.1:6499/healthz",
    timeout: 60_000,
    reuseExistingServer: false,
    stdout: "pipe",
  },
});
