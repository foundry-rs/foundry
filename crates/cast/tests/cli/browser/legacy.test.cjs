// Exercises the CLI producer and the embedded browser adapter together.
const assert = require('node:assert/strict');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { createServer } = require('node:net');
const { resolve } = require('node:path');
const { setTimeout: delay } = require('node:timers/promises');
const { test } = require('node:test');
const { chromium } = require('playwright');

async function unusedPort(requested = 0) {
  const server = createServer();
  server.listen(requested, '127.0.0.1');
  await once(server, 'listening');
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return port;
}

async function waitForServer(url, child) {
  for (let attempt = 0; attempt < 100; attempt++) {
    assert.equal(child.exitCode, null, 'server exited before becoming ready');
    try {
      await fetch(url);
      return;
    } catch {
      await delay(100);
    }
  }
  assert.fail(`server did not start: ${url}`);
}

test('cast send --browser --legacy preserves type at the wallet provider', { timeout: 60_000 }, async (t) => {
  const rpcPort = await unusedPort();
  // The embedded app currently sends API requests to the default browser port.
  const browserPort = await unusedPort(9545);
  const rpcUrl = `http://127.0.0.1:${rpcPort}`;
  const browserUrl = `http://127.0.0.1:${browserPort}`;
  const root = resolve(__dirname, '../../../../..');
  const from = '0x1111111111111111111111111111111111111111';
  const to = '0x2222222222222222222222222222222222222222';
  const rejection = 'Mock rejects every send';
  const start = (binary, args) => {
    const child = spawn(binary, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    t.after(() => { if (child.exitCode === null) child.kill(); });
    return child;
  };
  const anvil = start(process.env.ANVIL_BIN || resolve(root, 'target/debug/anvil'), [
    '--accounts', '0', '--silent', '--chain-id', '1', '--port', String(rpcPort),
  ]);
  await waitForServer(rpcUrl, anvil);
  const cast = start(process.env.CAST_BIN || resolve(root, 'target/debug/cast'), [
    'send', to, '--from', from, '--chain', '1', '--value', '0', '--data', '0x',
    '--nonce', '0', '--legacy', '--gas-limit', '21000', '--gas-price', '1000000000',
    '--rpc-url', rpcUrl, '--async', '--browser', '--browser-disable-open',
    '--browser-port', String(browserPort),
  ]);
  let stdout = '';
  let stderr = '';
  cast.stdout.on('data', (data) => { stdout += data; });
  cast.stderr.on('data', (data) => { stderr += data; });
  t.after(() => t.diagnostic(stderr));
  const closed = once(cast, 'close');
  await waitForServer(browserUrl, cast);
  const browser = await chromium.launch();
  t.after(() => browser.close());
  const page = await browser.newPage();
  page.on('pageerror', (error) => t.diagnostic(error.message));
  await page.addInitScript(({ from, rejection }) => {
    window.capturedTransactions = [];
    const detail = {
      info: {
        uuid: '11111111-2222-4333-8444-555555555555',
        name: 'Rejecting mock wallet', rdns: 'test.foundry.mock', icon: '',
      },
      provider: {
        async request({ method, params }) {
          if (method === 'eth_requestAccounts' || method === 'eth_accounts') return [from];
          if (method === 'eth_chainId') return '0x1';
          if (method === 'eth_sendTransaction') {
            window.capturedTransactions.push(params[0]);
            throw Object.assign(new Error(rejection), { code: 4001 });
          }
          throw new Error(`Unexpected method: ${method}`);
        },
        on() {},
        removeListener() {},
      },
    };
    const announce = () => window.dispatchEvent(new CustomEvent('eip6963:announceProvider', { detail }));
    window.addEventListener('eip6963:requestProvider', announce);
    announce();
  }, { from, rejection });
  await page.goto(browserUrl);
  const wallets = page.getByRole('combobox');
  const connect = page.getByRole('button', { name: 'Connect Wallet', exact: true });
  await wallets.or(connect).first().waitFor();
  if (await wallets.isVisible()) {
    await wallets.selectOption('11111111-2222-4333-8444-555555555555');
  }
  await connect.click();
  await page.getByRole('button', { name: 'Confirm Connection', exact: true }).click();
  await page.getByRole('button', { name: 'Sign & Send', exact: true }).click();
  await page.waitForFunction(() => window.capturedTransactions.length === 1);
  assert.deepEqual(await page.evaluate(() => window.capturedTransactions), [{
    type: '0x0', from, to, data: '0x', gas: '0x5208', gasPrice: '0x3b9aca00',
    nonce: '0x0', value: '0x0',
  }]);
  const [code, signal] = await closed;
  assert.equal(signal, null);
  assert.equal(code, 1);
  assert.match(stderr, new RegExp(rejection));
  assert.equal(stdout, '');
});
