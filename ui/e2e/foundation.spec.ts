import { expect, test, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import WebSocket from 'ws';

test.describe.configure({ mode: 'serial' });

async function closeAgentPopup(page: Page) {
  const dialog = page.getByRole('dialog', { name: 'Connect an agent' });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Close' }).click();
}


test('shows the selected logo in the header and browser tab', async ({ page, request }) => {
  await page.goto('/');
  const logo = page.locator('.brand .mark');
  await expect(logo).toBeVisible();
  await expect(logo).toHaveAttribute('viewBox', '0 0 1024 1024');
  await expect(logo.locator('polygon')).toHaveCount(2);
  await expect(logo.locator('polygon').nth(1)).toHaveAttribute('fill', '#5b806b');
  await expect(page.locator('link[rel="icon"]')).toHaveAttribute('href', '/studioline-logo.svg');
  const response = await request.get('/studioline-logo.svg');
  expect(response.ok()).toBeTruthy();
  expect(response.headers()['content-type']).toContain('image/svg+xml');
  const favicon = await response.text();
  for (const polygon of await logo.locator('polygon').all()) {
    expect(favicon).toContain(`points="${await polygon.getAttribute('points')}"`);
    expect(favicon).toContain(`fill="${await polygon.getAttribute('fill')}"`);
  }
});

test('creates a project, returns home, and reopens it after a refresh', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Projects', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'New project' }).click();
  await expect(page.getByRole('dialog', { name: 'Create a project' })).toBeVisible();
  const projectName = '<img src=x onerror=alert(1)>';
  await page.getByLabel('Project name').fill(projectName);
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await closeAgentPopup(page);
  await expect(page.getByRole('heading', { name: projectName })).toBeVisible();
  await expect(page.locator('img[src="x"]')).toHaveCount(0);
  await page.getByRole('button', { name: 'All projects' }).click();
  await page.reload();
  await expect(page.getByRole('button', { name: new RegExp('img src=x') })).toBeVisible();
  await page.getByRole('button', { name: new RegExp('img src=x') }).click();
  await closeAgentPopup(page);
  await expect(page.getByRole('heading', { name: projectName })).toBeVisible();
  expect(errors).toEqual([]);
});

test('rejects duplicate project names without losing existing projects', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('button', { name: 'New project' }).click();
  await page.getByLabel('Project name').fill('Test');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await closeAgentPopup(page);
  await expect(page.getByRole('heading', { name: 'Test', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'All projects' }).click();
  await page.getByRole('button', { name: 'New project' }).click();
  await page.getByLabel('Project name').fill(' test ');
  await page.getByRole('button', { name: 'Create project', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('A project with this name already exists.');
  await page.getByRole('button', { name: 'Cancel' }).click();
  await expect(page.getByRole('button', { name: /Test Local project Open/i })).toHaveCount(1);
});

test('shows connection status and inspects a simulated Studio session', async ({ page }) => {
  if (!process.env.STUDIOLINE_DATA_DIR) throw new Error('Set STUDIOLINE_DATA_DIR to the isolated server test directory.');
  await page.goto('/');
  await page.getByRole('button', { name: /Test Local project Open/i }).click();
  await closeAgentPopup(page);
  await expect(page.getByRole('heading', { name: 'Test', exact: true })).toBeVisible();
  const key = readFileSync(join(process.env.STUDIOLINE_DATA_DIR, 'studio-plugin.key'), 'utf8');
  const plugin = new WebSocket('ws://127.0.0.1:38741/api/studio/socket');
  try {
    await new Promise<void>((resolve, reject) => { plugin.once('open', () => resolve()); plugin.once('error', reject); });
    const connected = new Promise<void>((resolve, reject) => {
      plugin.on('message', bytes => {
        const message = JSON.parse(bytes.toString()) as { type: string; id?: string; command?: string; message?: string };
        if (message.type === 'connected') resolve();
        if (message.type === 'error') reject(new Error(message.message));
        if (message.type === 'command' && message.command === 'inspect_place') {
          plugin.send(JSON.stringify({ type: 'result', id: message.id, ok: true, data: { name: 'Simulated place', place_id: 42, workspace_children: [{ name: 'Baseplate', class_name: 'Part' }], total_children: 1 } }));
        }
      });
      plugin.once('error', reject);
    });
    plugin.send(JSON.stringify({ type: 'hello', token: key, place_name: 'Simulated place', place_id: 42 }));
    await connected;
    await expect(page.getByText('Connected to Simulated place.')).toBeVisible();
    await page.getByRole('button', { name: 'Inspect game' }).click();
    await expect(page.getByRole('heading', { name: 'Game inspection' })).toBeVisible();
    await expect(page.getByText('Baseplate', { exact: true })).toBeVisible();
    plugin.send(JSON.stringify({ type: 'place_name', name: 'Published name' }));
    await expect(page.getByText('Connected to Published name.')).toBeVisible();
    await expect(page.getByText('Published name', { exact: true })).toBeVisible();
    plugin.close();
    await expect(page.getByText('Not connected', { exact: true })).toBeVisible();
  } finally { plugin.terminate(); }
});

test('hides plugin installation when the current plugin is already installed', async ({ page }) => {
  await page.route('**/api/studio/install', async route => {
    if (route.request().method() === 'GET') await route.fulfill({ json: { state: 'installed' } });
    else await route.continue();
  });
  await page.goto('/');
  await page.getByRole('button', { name: /Test Local project Open/i }).click();
  await closeAgentPopup(page);
  await expect(page.getByRole('button', { name: 'Install Studio plugin' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Update Studio plugin' })).toHaveCount(0);
});

test('masks the tunnel address and individual prompt until revealed', async ({ page }) => {
  const token = 'a'.repeat(64);
  await page.route('**/api/agents/status', route => route.fulfill({ json: { state: 'ready', url: 'https://example.trycloudflare.com', error: null } }));
  await page.route('**/api/agents/issue', route => route.fulfill({ json: { id: 'test-credential', token } }));
  await page.route('**/api/agents?*', route => route.fulfill({ json: [{ id: 'test-credential', name: null, role: null }] }));
  await page.route('**/api/agents/test-credential', route => route.fulfill({ json: { revoked: true } }));
  await page.goto('/');
  await page.getByRole('button', { name: /Test Local project Open/i }).click();
  const dialog = page.getByRole('dialog', { name: 'Connect an agent' });
  await expect(dialog.getByText('Status: Ready')).toBeVisible();
  await expect(dialog.getByLabel('Tunnel address')).toHaveAttribute('type', 'password');
  await dialog.getByRole('button', { name: 'Create agent prompt' }).click();
  await expect(dialog.getByText('Prompt hidden. Reveal or copy it when you are ready.')).toBeVisible();
  await expect(dialog).not.toContainText(token);
  await dialog.getByRole('button', { name: 'Reveal prompt' }).click();
  await expect(dialog).toContainText(token);
  await dialog.getByRole('button', { name: 'Hide prompt' }).click();
  await expect(dialog).not.toContainText(token);
  await dialog.getByRole('button', { name: 'Revoke' }).click();
  await expect(dialog.getByText('Individual connection prompt')).toHaveCount(0);
  await dialog.getByRole('button', { name: 'Close' }).click();
});

test('exports a SQLite backup and deletes a project only after confirmation', async ({ page }) => {
  await page.goto('/');
  const downloaded = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Export StudioLine data' }).click();
  const file = await downloaded;
  expect(file.suggestedFilename()).toBe('StudioLine-export.db');
  const path = await file.path();
  expect(readFileSync(path).subarray(0, 16).toString()).toBe('SQLite format 3\x00');
  await page.getByRole('button', { name: /Test Local project Open/i }).click();
  await closeAgentPopup(page);
  await page.getByRole('button', { name: 'Delete project' }).click();
  const dialog = page.getByRole('dialog', { name: 'Delete Test?' });
  await expect(dialog).toContainText('Your Roblox place is not deleted');
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await expect(page.getByRole('heading', { name: 'Test', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Delete project' }).click();
  await page.getByRole('dialog', { name: 'Delete Test?' }).getByRole('button', { name: 'Delete project' }).click();
  await expect(page.getByRole('heading', { name: 'Projects', exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: /Test Local project Open/i })).toHaveCount(0);
});

test('can cancel shutdown and then shut down from the browser', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto('/');
  await page.getByRole('button', { name: new RegExp('img src=x') }).click();
  await closeAgentPopup(page);
  const key = readFileSync(join(process.env.STUDIOLINE_DATA_DIR!, 'studio-plugin.key'), 'utf8');
  const plugin = new WebSocket('ws://127.0.0.1:38741/api/studio/socket');
  await new Promise<void>((resolve, reject) => { plugin.once('open', () => resolve()); plugin.once('error', reject); });
  const shutdownMessage = new Promise<void>((resolve, reject) => {
    plugin.on('message', bytes => {
      const message = JSON.parse(bytes.toString()) as { type: string };
      if (message.type === 'shutdown') { plugin.send(JSON.stringify({ type: 'shutdown_ack' })); resolve(); }
      if (message.type === 'error') reject(new Error('Plugin rejected'));
    });
    plugin.once('error', reject);
  });
  plugin.send(JSON.stringify({ type: 'hello', token: key, place_name: 'Shutdown test', place_id: 1 }));
  await expect(page.getByText('Connected to Shutdown test.')).toBeVisible();
  await page.getByRole('button', { name: 'Shut down' }).click();
  await expect(page.getByRole('dialog', { name: 'Shut down StudioLine?' })).toBeVisible();
  await page.getByRole('button', { name: 'Cancel' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  // Once the service is stopped, no logo resource can be fetched from it.
  await page.route('**/studioline-logo.svg', route => route.abort());
  await page.getByRole('button', { name: 'Shut down' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Shut down' }).click();
  await expect(page.getByRole('heading', { name: 'StudioLine is closed' })).toBeVisible();
  const shutdownLogo = page.locator('.shutdown-page svg.mark');
  await expect(shutdownLogo).toBeVisible();
  await expect(shutdownLogo.locator('polygon')).toHaveCount(2);
  await expect(shutdownLogo.locator('polygon').nth(1)).toHaveAttribute('fill', '#5b806b');
  await expect(page.locator('.shutdown-page img')).toHaveCount(0);
  await shutdownMessage;
  plugin.terminate();
  await expect.poll(() => page.evaluate(() => fetch('/api/health').then(() => true).catch(() => false))).toBe(false);
  await expect(shutdownLogo).toBeVisible();
  await expect(shutdownLogo.locator('polygon')).toHaveCount(2);
  expect(errors).toEqual([]);
});
