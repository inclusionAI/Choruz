const { app, BrowserWindow, dialog, Menu, shell, ipcMain } = require('electron');
const path = require('node:path');
const { Runtime } = require('./runtime.cjs');
const { allowedNavigation, allowedFolderSender } = require('./security.cjs');

app.setName('Choruz');
app.setPath('userData', path.join(app.getPath('appData'), 'Choruz Desktop'));

let window;
let runtime;
let quitting = false;
let failing = false;
let confirmingQuit = false;

async function failure(error) {
  if (failing || quitting) return;
  failing = true;
  await runtime?.stop();
  const result = await dialog.showMessageBox({ type: 'error', title: 'Choruz could not continue', message: error.message, buttons: ['Quit', 'Open logs'], defaultId: 1 });
  if (result.response === 1) await shell.openPath(app.getPath('userData'));
  quitting = true;
  app.quit();
}

if (!app.requestSingleInstanceLock()) app.quit();
else {
  app.on('second-instance', () => { window?.show(); window?.focus(); });
  app.on('activate', () => { window?.show(); });
  app.on('before-quit', (event) => {
    if (quitting) return;
    event.preventDefault();
    if (confirmingQuit) return;
    confirmingQuit = true;
    void (async () => {
      const result = await dialog.showMessageBox({ type: 'question', title: 'Quit Choruz?', message: 'Running tasks will stop. Closing the window instead keeps Choruz running.', buttons: ['Keep running', 'Quit'], defaultId: 0, cancelId: 0 });
      confirmingQuit = false;
      if (result.response !== 1) return;
      quitting = true;
      await runtime?.stop();
      app.quit();
    })();
  });
  app.whenReady().then(async () => {
    window = new BrowserWindow({ width: 1320, height: 880, minWidth: 800, minHeight: 600, title: 'Choruz', backgroundColor: '#faf9f6', titleBarStyle: 'hiddenInset', webPreferences: { preload: path.join(__dirname, 'preload.cjs'), nodeIntegration: false, contextIsolation: true, sandbox: true } });
    window.on('close', (event) => { if (!quitting) { event.preventDefault(); window.hide(); } });
    window.webContents.on('will-navigate', (event, url) => { if (!allowedNavigation(url)) event.preventDefault(); });
    window.webContents.on('will-redirect', (event, url) => { if (!allowedNavigation(url)) event.preventDefault(); });
    let folderDialog;
    ipcMain.handle('choruz:choose-folder', async (event) => {
      if (!allowedFolderSender(event, window.webContents)) throw new Error('Folder selection is only available in the local workspace.');
      folderDialog ??= dialog.showOpenDialog(window, { properties: ['openDirectory', 'createDirectory'] }).finally(() => { folderDialog = null; });
      const result = await folderDialog;
      return result.canceled ? null : result.filePaths[0];
    });
    window.webContents.setWindowOpenHandler(({ url }) => {
      if (/^https?:\/\//.test(url)) void shell.openExternal(url);
      return { action: 'deny' };
    });
    window.webContents.session.setPermissionRequestHandler((_contents, _permission, callback) => callback(false));
    window.webContents.session.setPermissionCheckHandler(() => false);
    Menu.setApplicationMenu(Menu.buildFromTemplate([
      { label: 'Choruz', submenu: [{ role: 'about' }, { type: 'separator' }, { label: 'Show Choruz', click: () => window.show() }, { label: 'Open logs', click: () => shell.openPath(app.getPath('userData')) }, { type: 'separator' }, { role: 'hide' }, { role: 'quit' }] },
      { role: 'editMenu' }, { role: 'viewMenu' }, { role: 'windowMenu' },
    ]));
    await window.loadFile(path.join(__dirname, 'loading.html'));
    runtime = new Runtime({ bundle: app.isPackaged ? path.join(process.resourcesPath, 'runtime') : path.join(__dirname, 'bundle'), data: app.getPath('userData'), executable: process.execPath, onFailure: failure });
    try { await window.loadURL(await runtime.start()); } catch (error) { await failure(error); }
  });
}
