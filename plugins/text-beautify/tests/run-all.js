'use strict';

// 跑全部回归测试。任一子脚本退出码非 0 就整体失败。
// 注意：子进程通过 stdio: 'inherit' 直接继承控制台，不再捕获输出 ——
// 在受限环境里捕获 piped stdio 会被拒绝（EPERM），而这里也不需要解析子进程输出。

const { spawnSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const dir = __dirname;
const files = fs
  .readdirSync(dir)
  .filter((f) => /-test\.js$/.test(f))
  .sort();

if (!files.length) {
  console.error('没有找到 *-test.js');
  process.exit(1);
}

let failed = 0;
for (const f of files) {
  console.log('\n=== ' + f + ' ===');
  const r = spawnSync(process.execPath, [path.join(dir, f)], { stdio: 'inherit' });
  if (r.status !== 0) {
    failed++;
    console.error('!!! ' + f + ' 失败（退出码 ' + r.status + '）');
  }
}

console.log('\n' + (files.length - failed) + '/' + files.length + ' 个测试文件通过');
process.exit(failed ? 1 : 0);
