import { defineConfig } from "@playwright/test";

const appOrigin = process.env.PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:33000";

export default defineConfig({
  expect: {
    timeout: 5_000,
  },
  fullyParallel: false,
  outputDir: "test-results/playwright",
  projects: [
    {
      name: "chromium",
      use: {
        browserName: "chromium",
        viewport: { height: 900, width: 1280 },
      },
    },
  ],
  reporter: [["line"]],
  testDir: "./tests/e2e",
  timeout: 30_000,
  use: {
    baseURL: appOrigin,
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
    video: "retain-on-failure",
  },
  workers: 1,
});
