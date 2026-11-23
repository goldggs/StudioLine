import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  fullyParallel: false,
  workers: 1,
  reporter: 'list',
  outputDir: process.env.STUDIOLINE_TEST_OUTPUT || '/tmp/studioline-playwright-output',
  use: { baseURL: 'http://127.0.0.1:38741', browserName: 'chromium', headless: true },
});
