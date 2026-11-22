// 生成图标：icon.ico（BMP 编码，兼容所有 rc.exe）+ icon.png（托盘/窗口用）。
// 环境里 npm registry 不可达，所以用纯 Node 标准库手工构造，不依赖任何图像库。
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');

const outDir = path.join(__dirname, '..', 'crates', 'app', 'icons');
fs.mkdirSync(outDir, { recursive: true });

// ---- 画一个圆角方块 + 放大镜 ----
function pixel(size, x, y) {
  const cx = size / 2, cy = size / 2;
  const r = size * 0.30;              // 圆角半径
  const pad = size * 0.06;
  const inBox = (v, lo, hi) => v >= lo && v <= hi;
  const boxLo = pad, boxHi = size - 1 - pad;
  // 圆角矩形覆盖判断
  let dx = 0, dy = 0;
  if (x < boxLo + r) dx = boxLo + r - x; else if (x > boxHi - r) dx = x - (boxHi - r);
  if (y < boxLo + r) dy = boxLo + r - y; else if (y > boxHi - r) dy = y - (boxHi - r);
  const inside = Math.hypot(dx, dy) <= r || (dx === 0 && dy === 0);
  if (!inside || !inBox(x, boxLo, boxHi) || !inBox(y, boxLo, boxHi)) {
    return [0, 0, 0, 0];
  }
  const bg = [46, 92, 231];           // 主色
  const fg = [255, 255, 255];
  // 放大镜：圆心偏左上，半径 0.24，环宽 0.07；手柄朝右下
  const lx = size * 0.44, ly = size * 0.44, lr = size * 0.20;
  const d = Math.hypot(x - lx, y - ly);
  if (Math.abs(d - lr) < size * 0.055) return [...fg, 255];
  if (d < lr - size * 0.05) return [...bg, 255];
  // 手柄
  const t = (x - lx) * 1 + (y - ly) * 1;      // 沿 45 度方向投影
  const perp = Math.abs((x - lx) - (y - ly)) / Math.SQRT2;
  if (t > lr * 0.7 && t < lr * 1.9 && perp < size * 0.055) return [...fg, 255];
  return [...bg, 255];
}

function rgba(size) {
  const buf = Buffer.alloc(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      // 2x 超采样抗锯齿
      let acc = [0, 0, 0, 0];
      for (let sy = 0; sy < 2; sy++) {
        for (let sx = 0; sx < 2; sx++) {
          const p = pixel(size * 2, x * 2 + sx, y * 2 + sy);
          for (let k = 0; k < 4; k++) acc[k] += p[k];
        }
      }
      const i = (y * size + x) * 4;
      for (let k = 0; k < 4; k++) buf[i + k] = acc[k] / 4;
    }
  }
  return buf;
}

// ---- PNG ----
function crc32(buf) {
  let c, table = [];
  for (let n = 0; n < 256; n++) {
    c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  let crc = 0xffffffff;
  for (const b of buf) crc = table[(crc ^ b) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const len = Buffer.alloc(4); len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4); crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}
function png(size) {
  const raw = rgba(size);
  const lines = [];
  for (let y = 0; y < size; y++) {
    lines.push(Buffer.from([0]));                       // filter: none
    lines.push(raw.subarray(y * size * 4, (y + 1) * size * 4));
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0); ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; ihdr[9] = 6;                              // 8bit RGBA
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', zlib.deflateSync(Buffer.concat(lines), { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

// ---- ICO（内含 32/48 两个 BMP 条目，兼容性最好）----
function icoEntry(size) {
  const src = rgba(size);
  const andMask = Buffer.alloc(size * Math.ceil(size / 32) * 4); // 每行 32bit 对齐，全 0 = 用 alpha
  const bih = Buffer.alloc(40);
  bih.writeUInt32LE(40, 0);
  bih.writeInt32LE(size, 4);
  bih.writeInt32LE(size * 2, 8);   // BMP in ICO: height = 图像 + mask
  bih.writeUInt16LE(1, 12);
  bih.writeUInt16LE(32, 14);
  const px = Buffer.alloc(size * size * 4);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const si = (y * size + x) * 4;
      const di = ((size - 1 - y) * size + x) * 4;  // BMP 自底向上
      px[di] = src[si + 2];      // B
      px[di + 1] = src[si + 1];  // G
      px[di + 2] = src[si];      // R
      px[di + 3] = src[si + 3];  // A
    }
  }
  return Buffer.concat([bih, px, andMask]);
}
function ico(sizes) {
  const entries = sizes.map((s) => ({ size: s, data: icoEntry(s) }));
  const header = Buffer.alloc(6);
  header.writeUInt16LE(0, 0); header.writeUInt16LE(1, 2); header.writeUInt16LE(entries.length, 4);
  let offset = 6 + entries.length * 16;
  const dir = Buffer.alloc(entries.length * 16);
  const bodies = [];
  entries.forEach((e, i) => {
    const p = i * 16;
    dir[p] = e.size >= 256 ? 0 : e.size;
    dir[p + 1] = e.size >= 256 ? 0 : e.size;
    dir[p + 2] = 0; dir[p + 3] = 0;
    dir.writeUInt16LE(1, p + 4);
    dir.writeUInt16LE(32, p + 6);
    dir.writeUInt32LE(e.data.length, p + 8);
    dir.writeUInt32LE(offset, p + 12);
    offset += e.data.length;
    bodies.push(e.data);
  });
  return Buffer.concat([header, dir, ...bodies]);
}

fs.writeFileSync(path.join(outDir, 'icon.png'), png(64));
fs.writeFileSync(path.join(outDir, '32x32.png'), png(32));
fs.writeFileSync(path.join(outDir, '128x128.png'), png(128));
fs.writeFileSync(path.join(outDir, 'icon.ico'), ico([16, 32, 48]));

const written = ['icon.png', '32x32.png', '128x128.png', 'icon.ico'];

// ---- macOS：程序化画出各尺寸 → iconutil 打成 efto.icns ----
// tauri-bundler 的 ico→icns 转换会把小尺寸图块打坏（48px 实测变彩条，
// 系统权限列表里就是花的），所以直接给 bundler 现成的 icns，不给它转换的机会。
if (process.platform === 'darwin') {
  const { execFileSync } = require('child_process');
  const os = require('os');
  // iconutil 要求目录名以 .iconset 结尾
  const tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'efto-icons-'));
  const iconset = path.join(tmpRoot, 'efto.iconset');
  fs.mkdirSync(iconset);
  const sizes = [
    ['icon_16x16.png', 16], ['icon_16x16@2x.png', 32],
    ['icon_32x32.png', 32], ['icon_32x32@2x.png', 64],
    ['icon_128x128.png', 128], ['icon_128x128@2x.png', 256],
    ['icon_256x256.png', 256], ['icon_256x256@2x.png', 512],
    ['icon_512x512.png', 512], ['icon_512x512@2x.png', 1024],
  ];
  for (const [name, size] of sizes) {
    fs.writeFileSync(path.join(iconset, name), png(size));
  }
  execFileSync('iconutil', ['-c', 'icns', iconset, '-o', path.join(outDir, 'efto.icns')]);
  fs.rmSync(tmpRoot, { recursive: true, force: true });
  written.push('efto.icns');
}

console.log('icons written to', outDir);
for (const f of written) {
  const st = fs.statSync(path.join(outDir, f));
  console.log(' ', f, st.size, 'bytes');
}
