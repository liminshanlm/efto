// 颜色提取器：显示冻结的整屏图，光标处放大取色，单击把 HEX 写进剪贴板。
// 坐标口径与 screenshot.js 一致：图是物理像素，页面是 CSS 像素，
// scale = view_w / window.innerWidth，图内坐标 = 页面坐标 * scale + 偏移。
(function () {
  'use strict';

  var I = window.__TAURI_INTERNALS__;
  var bg = document.getElementById('bg');
  var zoom = document.getElementById('zoom');
  var zctx = zoom.getContext('2d');
  var box = document.getElementById('magnifier');
  var hexEl = document.getElementById('hex');
  var rgbEl = document.getElementById('rgb');
  var hline = document.getElementById('hline');
  var vline = document.getElementById('vline');
  var dot = document.getElementById('dot');
  var hint = document.getElementById('hint');
  var errorEl = document.getElementById('error');

  var shotW = 0, shotH = 0;
  var offX = 0, offY = 0, viewW = 0, viewH = 0;
  var scale = 1;
  var sample = document.createElement('canvas');
  var sctx = sample.getContext('2d', { willReadFrequently: true });
  var ready = false;
  var lastHex = '';

  function invoke(cmd, payload) {
    if (!I || typeof I.invoke !== 'function') {
      return Promise.reject(new Error('Tauri IPC 未就绪'));
    }
    return I.invoke(cmd, payload || {});
  }

  function showError(msg) {
    errorEl.textContent = String(msg);
    errorEl.style.display = 'block';
    hint.style.display = 'none';
  }

  function layoutBg() {
    if (!scale) return;
    bg.style.left = (-offX / scale) + 'px';
    bg.style.top = (-offY / scale) + 'px';
    bg.style.width = (shotW / scale) + 'px';
    bg.style.height = (shotH / scale) + 'px';
  }

  function resize() {
    scale = viewW && window.innerWidth ? viewW / window.innerWidth : 1;
    layoutBg();
  }

  function clamp(v, lo, hi) {
    return Math.max(lo, Math.min(hi, v));
  }

  function imagePoint(clientX, clientY) {
    var x = Math.round(clientX * scale + offX);
    var y = Math.round(clientY * scale + offY);
    return {
      x: clamp(x, 0, Math.max(0, shotW - 1)),
      y: clamp(y, 0, Math.max(0, shotH - 1))
    };
  }

  function toHex(r, g, b) {
    return '#' + [r, g, b].map(function (v) {
      return ('0' + v.toString(16)).slice(-2);
    }).join('').toUpperCase();
  }

  function drawZoom(x, y) {
    var n = Math.min(15, shotW, shotH);
    if (n < 1) return;
    var half = Math.floor(n / 2);
    var cell = 132 / n;
    var sx = clamp(x - half, 0, shotW - n);
    var sy = clamp(y - half, 0, shotH - n);
    var data = sctx.getImageData(sx, sy, n, n).data;
    zctx.imageSmoothingEnabled = false;
    for (var row = 0; row < n; row++) {
      for (var col = 0; col < n; col++) {
        var i = (row * n + col) * 4;
        zctx.fillStyle = 'rgb(' + data[i] + ',' + data[i + 1] + ',' + data[i + 2] + ')';
        zctx.fillRect(Math.floor(col * cell), Math.floor(row * cell), Math.ceil(cell), Math.ceil(cell));
      }
    }
    var cx = (x - sx) * cell;
    var cy = (y - sy) * cell;
    zctx.strokeStyle = 'rgba(255, 255, 255, .95)';
    zctx.lineWidth = 2;
    zctx.strokeRect(cx + 1, cy + 1, cell - 2, cell - 2);
    zctx.strokeStyle = 'rgba(0, 0, 0, .85)';
    zctx.lineWidth = 1;
    zctx.strokeRect(cx + .5, cy + .5, cell - 1, cell - 1);
  }

  function placeBox(cx, cy) {
    var w = box.offsetWidth || 148;
    var h = box.offsetHeight || 190;
    var gap = 22;
    var left = cx + gap;
    var top = cy + gap;
    if (left + w > window.innerWidth - 8) left = cx - w - gap;
    if (top + h > window.innerHeight - 8) top = cy - h - gap;
    box.style.left = clamp(left, 8, Math.max(8, window.innerWidth - w - 8)) + 'px';
    box.style.top = clamp(top, 8, Math.max(8, window.innerHeight - h - 8)) + 'px';
  }

  function update(clientX, clientY) {
    if (!ready) return;
    var p = imagePoint(clientX, clientY);
    var d = sctx.getImageData(p.x, p.y, 1, 1).data;
    var hex = toHex(d[0], d[1], d[2]);
    lastHex = hex;
    hexEl.textContent = hex;
    rgbEl.textContent = 'rgb(' + d[0] + ', ' + d[1] + ', ' + d[2] + ')';
    drawZoom(p.x, p.y);
    dot.style.display = 'block';
    dot.style.left = clientX + 'px';
    dot.style.top = clientY + 'px';
    hline.style.display = 'block';
    hline.style.top = clientY + 'px';
    vline.style.display = 'block';
    vline.style.left = clientX + 'px';
    box.style.display = 'block';
    placeBox(clientX, clientY);
  }

  function finish() {
    if (!lastHex) return;
    hint.textContent = '已复制 ' + lastHex;
    invoke('color_picker_finish', { color: lastHex }).catch(function (e) {
      showError('复制失败：' + e);
    });
  }

  function cancel() {
    invoke('color_picker_cancel').catch(function () {});
  }

  document.addEventListener('pointermove', function (e) {
    update(e.clientX, e.clientY);
  });

  document.addEventListener('pointerdown', function (e) {
    if (e.button !== 0) return;
    e.preventDefault();
    finish();
  });

  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') {
      e.preventDefault();
      cancel();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      finish();
    }
  });

  window.addEventListener('resize', resize);
  window.addEventListener('blur', function () {});

  invoke('screenshot_begin').then(function (info) {
    shotW = info.width;
    shotH = info.height;
    offX = info.off_x || 0;
    offY = info.off_y || 0;
    viewW = info.view_w || shotW;
    viewH = info.view_h || shotH;
    sample.width = shotW;
    sample.height = shotH;
    bg.onload = function () {
      sctx.drawImage(bg, 0, 0, shotW, shotH);
      ready = true;
      update(window.innerWidth / 2, window.innerHeight / 2);
    };
    bg.src = info.data_url;
    resize();
  }).catch(function (e) {
    showError('读取屏幕失败：' + e);
  });
})();
