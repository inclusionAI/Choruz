const { origin, ports } = require('./runtime.cjs');

function allowedNavigation(url) {
  try {
    const parsed = new URL(url);
    if (parsed.username || parsed.password) return false;
    return parsed.origin === origin || (parsed.origin === `http://127.0.0.1:${ports.api}` && parsed.pathname === '/v1/auth/local/bootstrap');
  } catch { return false; }
}

function allowedFolderSender(event, contents) {
  return event.sender === contents && event.senderFrame === contents.mainFrame
    && new URL(event.senderFrame.url).origin === origin;
}

module.exports = { allowedNavigation, allowedFolderSender };
