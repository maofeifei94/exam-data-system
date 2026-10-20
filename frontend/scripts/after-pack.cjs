// electron-builder afterPack 钩子：extraResources 复制进 Linux 包体时可能丢失可执行位，
// 这里补回 0755，否则主进程无法拉起后端可执行文件。

const fs = require('node:fs');
const path = require('node:path');

async function afterPack(context) {
  if (context.electronPlatformName !== 'linux') {
    return;
  }
  const backend = path.join(context.appOutDir, 'resources', 'backend', 'exam-backend');
  if (fs.existsSync(backend)) {
    fs.chmodSync(backend, 0o755);
  }
}

module.exports = afterPack;
module.exports.default = afterPack;
