const { contextBridge, ipcRenderer } = require('electron');
contextBridge.exposeInMainWorld('choruzDesktop', { chooseFolder: () => ipcRenderer.invoke('choruz:choose-folder') });
window.addEventListener('DOMContentLoaded', () => document.documentElement.classList.add('choruz-desktop'));
