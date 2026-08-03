import { defineConfig } from "@playwright/test";

// The suite drives the real cockpit over the real store: `brain eyes`
// is started on a fixed port, and the JSON report is written where
// `brain testrun import e2e/test-results/eyes-report.json` picks it up —
// failures leave screenshots behind, which become declared assets.
export default defineConfig({
  testDir: ".",
  workers: 1,
  reporter: [
    ["list"],
    ["json", { outputFile: "test-results/eyes-report.json" }],
  ],
  use: {
    baseURL: "http://127.0.0.1:4877",
    channel: "chrome",
    viewport: { width: 1280, height: 900 },
    screenshot: "only-on-failure",
  },
  webServer: {
    command: "cd .. && target/debug/brain eyes --prefix twin/self --port 4877",
    url: "http://127.0.0.1:4877/api/snapshot",
    reuseExistingServer: true,
    timeout: 30_000,
  },
});
