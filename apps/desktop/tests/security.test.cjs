const { test } = require('node:test');
const assert = require('node:assert/strict');
const { allowedNavigation, allowedFolderSender } = require('../security.cjs');
const { origin, ports } = require('../runtime.cjs');

test('navigation stays on the owned application or the exact local login endpoint', () => {
  assert.equal(allowedNavigation(`${origin}/dashboard`), true);
  assert.equal(allowedNavigation(`http://127.0.0.1:${ports.api}/v1/auth/local/bootstrap?return_port=${ports.web}`), true);
  for (const url of ['https://example.com', 'file:///etc/passwd', `http://127.0.0.1:${ports.api}/v1/bootstrap`, `http://user@127.0.0.1:${ports.web}/dashboard`, `${origin}.evil.test/dashboard`]) {
    assert.equal(allowedNavigation(url), false, url);
  }
});

test('native folder access rejects other windows, embedded frames and remote documents', () => {
  const mainFrame = { url: `${origin}/dashboard` };
  const contents = { mainFrame };
  const event = { sender: contents, senderFrame: mainFrame };
  assert.equal(allowedFolderSender(event, contents), true);
  assert.equal(allowedFolderSender({ ...event, sender: {} }, contents), false);
  assert.equal(allowedFolderSender({ ...event, senderFrame: { ...mainFrame } }, contents), false);
  mainFrame.url = 'https://example.com/';
  assert.equal(allowedFolderSender(event, contents), false);
});
