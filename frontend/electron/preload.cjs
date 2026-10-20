const { contextBridge } = require('electron');

// Expose the backend base URL to the renderer so the typed API client can use it
// when the app is loaded from file:// inside Electron.
contextBridge.exposeInMainWorld('examapi', {
  baseUrl: 'http://127.0.0.1:8710',
});
