// 截图选区覆盖层。窗口已经被后端铺满整个虚拟桌面（所有显示器），
// 页面负责：显示整屏图、画遮罩与选区、Snipaste 风格的标注（矩形/椭圆/
// 箭头/画笔/荧光笔/马赛克/文字/橡皮 + 撤销重做）、把"选区画面 + 标注"
// 合成为 PNG data URL 交回后端。
(function () {
  'use strict';

  var I = window.__TAURI_INTERNALS__;
  var bg = document.getElementById('bg');
  var canvas = document.getElementById('mask');
  var ctx = canvas.getContext('2d');
  var bar = document.getElementById('bar');
  var hint = document.getElementById('hint');
  var errorEl = document.getElementById('error');
  var palette = document.getElementById('palette');
  var textInput = document.getElementById('text-input');
  var colorDot = document.getElementById('color-dot');
  var widthDot = document.getElementById('width-dot');
  var btnColor = document.getElementById('btn-color');
  var btnWidth = document.getElementById('btn-width');
  var btnUndo = document.getElementById('btn-undo');
  var btnRedo = document.getElementById('btn-redo');
  var btnLong = document.getElementById('btn-long');

  // 整屏图是物理像素；页面是 CSS 像素。scale = 物理 / CSS。
  var shotW = 0;
  var shotH = 0;
  var scale = 1;
  var dpr = window.devicePixelRatio || 1;

  // 选区用 CSS 像素保存（便于绘制），提交时再换算
  var sel = null;
  var dragging = false;
  var anchor = null;
  // 纯点击（没拖出选区）时恢复上一个选区，双击复制才能工作
  var selBeforeDrag = null;
  // 长截图状态：模式标志、已拼接行数、进度轮询定时器
  var longMode = false;
  var longRows = 0;
  var longTimer = null;

  // ---------- 标注状态 ----------
  var tool = null; // null | rect|ellipse|arrow|pen|marker|mosaic|text|eraser
  var color = '#ff4d4f';
  var WIDTHS = [2, 4, 8];
  var TEXT_SIZES = [14, 18, 26];
  var widthIdx = 0;
  var shapes = []; // 已提交的标注（CSS 像素坐标）
  var cur = null; // 拖拽中的标注
  var undoStack = [];
  var redoStack = [];
  var erasing = false;
  var eraserSnap = null;
  var textEditing = null; // { x, y, size }
  var mosaicCanvas = null;

  var COLORS = ['#ff4d4f', '#ffa940', '#ffe24a', '#73d13d', '#78a2ff', '#b37feb', '#ffffff', '#1c1c1c'];

  function invoke(cmd, payload) {
    if (!I || typeof I.invoke !== 'function') {
      return Promise.reject(new Error('Tauri IPC 未就绪'));
    }
    return I.invoke(cmd, payload || {});
  }

  function showError(msg) {
    errorEl.textContent = String(msg);
    errorEl.style.display = 'block';
  }

  function resize() {
    dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(window.innerWidth * dpr);
    canvas.height = Math.round(window.innerHeight * dpr);
    scale = shotW && window.innerWidth ? shotW / window.innerWidth : 1;
    draw();
  }

  function selLabel() {
    if (longMode) return '已拼接 ' + longRows + ' px';
    return Math.round(sel.w * scale) + ' × ' + Math.round(sel.h * scale);
  }

  function draw() {
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, window.innerWidth, window.innerHeight);
    // 未选区域压暗，选区保持原图亮度
    ctx.fillStyle = 'rgba(0, 0, 0, .45)';
    ctx.fillRect(0, 0, window.innerWidth, window.innerHeight);
    if (!sel || sel.w < 1 || sel.h < 1) return;

    // 长图模式只更新进度文字，选框保持用户拖出来的原样
    ctx.clearRect(sel.x, sel.y, sel.w, sel.h);
    drawAnnotations(ctx);

    ctx.strokeStyle = '#78a2ff';
    ctx.lineWidth = 1;
    ctx.strokeRect(sel.x + .5, sel.y + .5, sel.w - 1, sel.h - 1);

    var label = selLabel();
    ctx.font = '12px "Segoe UI", "Microsoft YaHei", sans-serif';
    var tw = ctx.measureText(label).width + 12;
    var lx = Math.min(sel.x, window.innerWidth - tw - 4);
    var ly = sel.y - 22 >= 4 ? sel.y - 22 : sel.y + 4;
    ctx.fillStyle = 'rgba(17, 19, 26, .92)';
    ctx.fillRect(lx, ly, tw, 18);
    ctx.fillStyle = '#eef1f7';
    ctx.fillText(label, lx + 6, ly + 13);
  }

  // ---------- 标注绘制 ----------

  // 把 shapes + cur 画到给定 ctx（CSS 像素坐标系，裁剪到选区）
  function drawAnnotations(g) {
    if (!sel) return;
    g.save();
    g.beginPath();
    g.rect(sel.x, sel.y, sel.w, sel.h);
    g.clip();
    for (var i = 0; i < shapes.length; i++) paintShape(g, shapes[i]);
    if (cur) paintShape(g, cur);
    g.restore();
  }

  function paintShape(g, s) {
    g.lineWidth = s.lw;
    g.strokeStyle = s.color;
    g.lineJoin = 'round';
    g.lineCap = 'round';
    if (s.t === 'rect') {
      g.strokeRect(s.x + s.lw / 2, s.y + s.lw / 2, Math.max(1, s.w - s.lw), Math.max(1, s.h - s.lw));
    } else if (s.t === 'ellipse') {
      g.beginPath();
      g.ellipse(
        s.x + s.w / 2, s.y + s.h / 2,
        Math.max(1, s.w / 2 - s.lw / 2), Math.max(1, s.h / 2 - s.lw / 2),
        0, 0, Math.PI * 2
      );
      g.stroke();
    } else if (s.t === 'arrow') {
      var dx = s.x2 - s.x1;
      var dy = s.y2 - s.y1;
      var len = Math.hypot(dx, dy) || 1;
      var head = Math.max(9, s.lw * 3.2);
      var bx = s.x2 - dx / len * head;
      var by = s.y2 - dy / len * head;
      var nx = -dy / len * head * .5;
      var ny = dx / len * head * .5;
      g.beginPath();
      g.moveTo(s.x1, s.y1);
      g.lineTo(s.x2, s.y2);
      g.moveTo(s.x2, s.y2); g.lineTo(bx + nx, by + ny);
      g.moveTo(s.x2, s.y2); g.lineTo(bx - nx, by - ny);
      g.stroke();
    } else if (s.t === 'pen' || s.t === 'marker') {
      if (s.t === 'marker') {
        g.globalAlpha = .38;
        g.lineWidth = s.lw * 5;
        g.lineCap = 'square';
      }
      g.beginPath();
      g.moveTo(s.pts[0].x, s.pts[0].y);
      for (var i = 1; i < s.pts.length; i++) g.lineTo(s.pts[i].x, s.pts[i].y);
      if (s.pts.length === 1) g.lineTo(s.pts[0].x + .01, s.pts[0].y);
      g.stroke();
      g.globalAlpha = 1;
    } else if (s.t === 'mosaic') {
      // 把选区里的原图缩小到 1/10 再放大回来，就是马赛克
      var sw = Math.max(1, s.w * scale);
      var sh = Math.max(1, s.h * scale);
      var tw = Math.max(1, Math.round(sw / 10));
      var th = Math.max(1, Math.round(sh / 10));
      if (!mosaicCanvas) mosaicCanvas = document.createElement('canvas');
      mosaicCanvas.width = tw;
      mosaicCanvas.height = th;
      var mg = mosaicCanvas.getContext('2d');
      mg.clearRect(0, 0, tw, th);
      mg.drawImage(bg, s.x * scale, s.y * scale, sw, sh, 0, 0, tw, th);
      g.imageSmoothingEnabled = false;
      g.drawImage(mosaicCanvas, 0, 0, tw, th, s.x, s.y, s.w, s.h);
      g.imageSmoothingEnabled = true;
    } else if (s.t === 'text') {
      g.fillStyle = s.color;
      g.font = s.size + 'px "Segoe UI", "Microsoft YaHei", system-ui, sans-serif';
      g.textBaseline = 'top';
      var lines = s.text.split('\n');
      for (var li = 0; li < lines.length; li++) {
        g.fillText(lines[li], s.x, s.y + li * s.size * 1.3);
      }
    }
  }

  // ---------- 撤销 / 重做 ----------

  function pushUndo() {
    undoStack.push(shapes.slice());
    if (undoStack.length > 60) undoStack.shift();
    redoStack.length = 0;
  }

  function undo() {
    if (!undoStack.length) return;
    redoStack.push(shapes.slice());
    shapes = undoStack.pop();
    draw();
    placeBar();
  }

  function redo() {
    if (!redoStack.length) return;
    undoStack.push(shapes.slice());
    shapes = redoStack.pop();
    draw();
    placeBar();
  }

  // ---------- 工具 ----------

  function setTool(t) {
    tool = t;
    var btns = document.querySelectorAll('#bar .tool');
    for (var i = 0; i < btns.length; i++) {
      btns[i].classList.toggle('active', btns[i].getAttribute('data-tool') === t);
    }
  }

  function shapeBBox(s) {
    if (s.t === 'pen' || s.t === 'marker') {
      var minx = Infinity, miny = Infinity, maxx = -Infinity, maxy = -Infinity;
      for (var i = 0; i < s.pts.length; i++) {
        minx = Math.min(minx, s.pts[i].x);
        miny = Math.min(miny, s.pts[i].y);
        maxx = Math.max(maxx, s.pts[i].x);
        maxy = Math.max(maxy, s.pts[i].y);
      }
      return { x: minx, y: miny, w: maxx - minx, h: maxy - miny };
    }
    if (s.t === 'arrow') {
      return {
        x: Math.min(s.x1, s.x2), y: Math.min(s.y1, s.y2),
        w: Math.abs(s.x2 - s.x1), h: Math.abs(s.y2 - s.y1)
      };
    }
    if (s.t === 'text') {
      ctx.font = s.size + 'px "Segoe UI", "Microsoft YaHei", system-ui, sans-serif';
      var lines = s.text.split('\n');
      var w = 0;
      for (var li = 0; li < lines.length; li++) {
        w = Math.max(w, ctx.measureText(lines[li]).width);
      }
      return { x: s.x, y: s.y, w: w, h: lines.length * s.size * 1.3 };
    }
    return { x: s.x, y: s.y, w: s.w, h: s.h };
  }

  function eraseAt(p) {
    var pad = 8;
    var removed = false;
    shapes = shapes.filter(function (s) {
      var b = shapeBBox(s);
      var hit = p.x >= b.x - pad && p.x <= b.x + b.w + pad &&
        p.y >= b.y - pad && p.y <= b.y + b.h + pad;
      if (hit) removed = true;
      return !hit;
    });
    if (removed) draw();
  }

  function startShape(t, p) {
    if (t === 'text') {
      setTool(null);
      openTextEditor(p.x, p.y);
      return;
    }
    if (t === 'eraser') {
      erasing = true;
      eraserSnap = shapes.slice();
      eraseAt(p);
      return;
    }
    anchor = p;
    if (t === 'pen' || t === 'marker') {
      cur = { t: t, pts: [{ x: p.x, y: p.y }], color: color, lw: WIDTHS[widthIdx] };
    } else if (t === 'arrow') {
      cur = { t: 'arrow', x1: p.x, y1: p.y, x2: p.x, y2: p.y, color: color, lw: WIDTHS[widthIdx] };
    } else {
      cur = { t: t, x: p.x, y: p.y, w: 0, h: 0, color: color, lw: WIDTHS[widthIdx] };
    }
    draw();
  }

  function moveShape(p) {
    if (erasing) {
      eraseAt(p);
      return;
    }
    if (!cur) return;
    if (cur.t === 'pen' || cur.t === 'marker') {
      cur.pts.push({ x: p.x, y: p.y });
    } else if (cur.t === 'arrow') {
      cur.x2 = p.x;
      cur.y2 = p.y;
    } else {
      cur.x = Math.min(anchor.x, p.x);
      cur.y = Math.min(anchor.y, p.y);
      cur.w = Math.abs(p.x - anchor.x);
      cur.h = Math.abs(p.y - anchor.y);
    }
    draw();
  }

  function pathLength(s) {
    var d = 0;
    for (var i = 1; i < s.pts.length; i++) {
      d += Math.hypot(s.pts[i].x - s.pts[i - 1].x, s.pts[i].y - s.pts[i - 1].y);
    }
    return d;
  }

  function finishShape() {
    var s = cur;
    cur = null;
    if (!s) return;
    var ok = false;
    if (s.t === 'rect' || s.t === 'ellipse' || s.t === 'mosaic') {
      ok = s.w >= 3 && s.h >= 3;
    } else if (s.t === 'arrow') {
      ok = Math.hypot(s.x2 - s.x1, s.y2 - s.y1) >= 4;
    } else {
      ok = pathLength(s) >= 3;
    }
    if (ok) {
      pushUndo();
      shapes.push(s);
      placeBar();
      // 一次成型的工具画完即退出；画笔/荧光笔/橡皮保持连续
      if (s.t === 'rect' || s.t === 'ellipse' || s.t === 'arrow' || s.t === 'mosaic') {
        setTool(null);
      }
    }
    draw();
  }

  function endErase() {
    erasing = false;
    if (eraserSnap && eraserSnap.length !== shapes.length) {
      undoStack.push(eraserSnap);
      if (undoStack.length > 60) undoStack.shift();
      redoStack.length = 0;
      placeBar();
    }
    eraserSnap = null;
  }

  // ---------- 文字 ----------

  function openTextEditor(x, y) {
    var fs = TEXT_SIZES[widthIdx];
    x = Math.max(4, Math.min(x, window.innerWidth - 80));
    y = Math.max(4, Math.min(y, window.innerHeight - fs * 2));
    textEditing = { x: x, y: y, size: fs };
    textInput.value = '';
    textInput.style.left = x + 'px';
    textInput.style.top = y + 'px';
    textInput.style.color = color;
    textInput.style.caretColor = color;
    textInput.style.font = fs + 'px/1.3 "Segoe UI", "Microsoft YaHei", system-ui, sans-serif';
    textInput.style.width = '60px';
    textInput.style.height = (fs * 1.3 + 8) + 'px';
    textInput.style.display = 'block';
    textInput.focus();
    autoSizeText();
  }

  function autoSizeText() {
    if (!textEditing) return;
    var fs = textEditing.size;
    ctx.font = fs + 'px "Segoe UI", "Microsoft YaHei", system-ui, sans-serif';
    var lines = textInput.value.split('\n');
    var w = 20;
    for (var i = 0; i < lines.length; i++) {
      w = Math.max(w, ctx.measureText(lines[i] || ' ').width);
    }
    var maxW = window.innerWidth - textEditing.x - 8;
    textInput.style.width = Math.max(60, Math.min(w + 12, maxW)) + 'px';
    textInput.style.height = (lines.length * fs * 1.3 + 8) + 'px';
  }

  function commitText() {
    if (!textEditing) return;
    var v = textInput.value.replace(/\s+$/, '');
    if (v) {
      pushUndo();
      shapes.push({ t: 'text', x: textEditing.x, y: textEditing.y, text: v, color: color, size: textEditing.size });
      placeBar();
    }
    textEditing = null;
    textInput.style.display = 'none';
    textInput.value = '';
    draw();
  }

  function cancelText() {
    textEditing = null;
    textInput.style.display = 'none';
    textInput.value = '';
  }

  textInput.addEventListener('keydown', function (e) {
    e.stopPropagation();
    if (e.key === 'Escape') {
      e.preventDefault();
      cancelText();
    } else if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      commitText();
    }
  });
  textInput.addEventListener('input', autoSizeText);
  textInput.addEventListener('blur', function () {
    if (textEditing) commitText();
  });

  // ---------- 合成与提交 ----------

  // 把"选区原始画面 + 标注"合成一张物理像素的 PNG data URL
  function compositeOverlay() {
    var W = Math.max(1, Math.round(sel.w * scale));
    var H = Math.max(1, Math.round(sel.h * scale));
    var c = document.createElement('canvas');
    c.width = W;
    c.height = H;
    var g = c.getContext('2d');
    g.drawImage(bg, sel.x * scale, sel.y * scale, sel.w * scale, sel.h * scale, 0, 0, W, H);
    g.scale(scale, scale);
    g.translate(-sel.x, -sel.y);
    drawAnnotations(g);
    return c.toDataURL('image/png');
  }

  function submit(action) {
    if (!sel || sel.w < 3 || sel.h < 3) {
      showError('请先拖拽选择一块区域');
      return;
    }
    if (textEditing) commitText();
    var x = Math.round(sel.x * scale);
    var y = Math.round(sel.y * scale);
    var w = Math.max(1, Math.round(sel.w * scale));
    var h = Math.max(1, Math.round(sel.h * scale));
    var payload = { action: action, x: x, y: y, w: w, h: h };
    if (shapes.length > 0) payload.overlay = compositeOverlay();
    bar.style.display = 'none';
    palette.style.display = 'none';
    invoke('screenshot_finish', payload)
      .catch(function (e) {
        showError(String(e));
        bar.style.display = 'flex';
      });
  }

  function cancel() {
    invoke('screenshot_cancel').catch(function () {});
  }

  // 长截图：覆盖层留下（鼠标已由后端设为穿透），用户直接滚动底下的窗口。
  function startLong() {
    if (shapes.length > 0) {
      showError('长图不支持叠加标注，请先撤销标注');
      return;
    }
    if (!sel || sel.w < 3 || sel.h < 3) {
      showError('请先拖拽选择一块区域');
      return;
    }
    var x = Math.round(sel.x * scale);
    var y = Math.round(sel.y * scale);
    var w = Math.max(1, Math.round(sel.w * scale));
    var h = Math.max(1, Math.round(sel.h * scale));
    invoke('screenshot_long_start', { x: x, y: y, w: w, h: h })
      .then(function () {
        longMode = true;
        longRows = 0;
        bg.style.display = 'none'; // 露出底下的真实页面
        bar.style.display = 'none';
        palette.style.display = 'none';
        hint.textContent = '拖动滚动条滚动页面 · 停止滚动 2.5 秒自动完成 · Esc 取消';
        hint.style.display = 'block';
        errorEl.style.display = 'none';
        draw();
        pollLong();
      })
      .catch(function (e) {
        showError(String(e));
      });
  }

  // 轮询拼接进度：rows 是已追加的物理像素行数
  function pollLong() {
    if (!longMode) return;
    invoke('screenshot_long_status')
      .then(function (s) {
        if (!longMode) return;
        if (!s.running) {
          longMode = false;
          hint.textContent = '正在保存长截图…';
          return;
        }
        longRows = s.rows || 0;
        draw();
        longTimer = setTimeout(pollLong, 300);
      })
      .catch(function () {
        longTimer = setTimeout(pollLong, 600);
      });
  }

  // ---------- 工具栏 ----------

  function placeBar() {
    palette.style.display = 'none';
    if (!sel || sel.w < 3 || sel.h < 3) {
      bar.style.display = 'none';
      return;
    }
    bar.style.display = 'flex';
    btnUndo.disabled = undoStack.length === 0;
    btnRedo.disabled = redoStack.length === 0;
    btnLong.disabled = shapes.length > 0;
    // 放在选区右下角内侧；贴边时翻到外侧
    var bw = bar.offsetWidth || 560;
    var bh = bar.offsetHeight || 40;
    var x = sel.x + sel.w - bw;
    var y = sel.y + sel.h + 6;
    if (y + bh > window.innerHeight - 4) y = sel.y - bh - 6;
    if (y < 4) y = sel.y + sel.h - bh - 6;
    x = Math.max(4, Math.min(x, window.innerWidth - bw - 4));
    y = Math.max(4, y);
    bar.style.left = x + 'px';
    bar.style.top = y + 'px';
  }

  function refreshWidthDot() {
    var sizes = [5, 8, 12];
    widthDot.style.width = sizes[widthIdx] + 'px';
    widthDot.style.height = sizes[widthIdx] + 'px';
  }

  function buildPalette() {
    COLORS.forEach(function (c) {
      var b = document.createElement('button');
      b.className = 'swatch';
      b.style.background = c;
      b.title = c;
      b.addEventListener('click', function () {
        color = c;
        colorDot.style.background = c;
        palette.style.display = 'none';
      });
      palette.appendChild(b);
    });
    colorDot.style.background = color;
  }

  var toolBtns = document.querySelectorAll('#bar .tool');
  for (var ti = 0; ti < toolBtns.length; ti++) {
    (function (b) {
      b.addEventListener('click', function () {
        if (textEditing) commitText();
        var t = b.getAttribute('data-tool');
        setTool(tool === t ? null : t);
      });
    })(toolBtns[ti]);
  }

  btnColor.addEventListener('click', function () {
    if (textEditing) commitText();
    if (palette.style.display === 'flex') {
      palette.style.display = 'none';
      return;
    }
    var r = btnColor.getBoundingClientRect();
    var pw = palette.offsetWidth || 220;
    var px = Math.max(4, Math.min(r.left - 40, window.innerWidth - pw - 4));
    var py = r.top > 120 ? r.top - 60 : r.bottom + 8;
    palette.style.left = px + 'px';
    palette.style.top = py + 'px';
    palette.style.display = 'flex';
  });

  btnWidth.addEventListener('click', function () {
    widthIdx = (widthIdx + 1) % WIDTHS.length;
    refreshWidthDot();
  });

  btnUndo.addEventListener('click', undo);
  btnRedo.addEventListener('click', redo);

  // ---------- 鼠标 ----------

  canvas.addEventListener('pointerdown', function (e) {
    if (e.button !== 0) return;
    if (textEditing) {
      commitText();
      return;
    }
    canvas.setPointerCapture(e.pointerId);
    var p = { x: e.clientX, y: e.clientY };
    var inSel = sel && sel.w >= 3 && sel.h >= 3 &&
      p.x >= sel.x && p.x <= sel.x + sel.w &&
      p.y >= sel.y && p.y <= sel.y + sel.h;
    if (tool && inSel) {
      // 文字工具：pointerdown 的默认动作会把焦点移到 body，刚 focus() 的
      // textarea 紧接着就 blur，blur 里又 commitText 掉空内容——表现为
      // "点了没反应、打不了字"。取消默认动作，焦点才能留在 textarea 里。
      if (tool === 'text') e.preventDefault();
      startShape(tool, p);
      return;
    }
    dragging = true;
    anchor = p;
    selBeforeDrag = sel;
    sel = { x: p.x, y: p.y, w: 0, h: 0 };
    bar.style.display = 'none';
    hint.style.display = 'none';
    palette.style.display = 'none';
    errorEl.style.display = 'none';
    draw();
  });

  canvas.addEventListener('pointermove', function (e) {
    var p = { x: e.clientX, y: e.clientY };
    if (cur || erasing) {
      moveShape(p);
      return;
    }
    if (!dragging || !anchor) return;
    sel = {
      x: Math.min(anchor.x, p.x),
      y: Math.min(anchor.y, p.y),
      w: Math.abs(p.x - anchor.x),
      h: Math.abs(p.y - anchor.y)
    };
    draw();
  });

  canvas.addEventListener('pointerup', function (e) {
    if (e.button !== 0) return;
    if (cur) {
      finishShape();
      return;
    }
    if (erasing) {
      endErase();
      return;
    }
    if (!dragging) return;
    dragging = false;
    // 纯点击没拖出新选区：恢复原选区（否则双击复制会丢选区）
    if (sel && sel.w < 3 && sel.h < 3 && selBeforeDrag) {
      sel = selBeforeDrag;
    }
    placeBar();
  });

  canvas.addEventListener('dblclick', function () {
    submit('copy');
  });

  document.addEventListener('contextmenu', function (e) {
    e.preventDefault();
    cancel();
  });

  var TOOL_KEYS = { 1: 'rect', 2: 'ellipse', 3: 'arrow', 4: 'pen', 5: 'marker', 6: 'mosaic', 7: 'text', 8: 'eraser' };

  document.addEventListener('keydown', function (e) {
    if (textInput.style.display === 'block') return;
    if (e.key === 'Escape') {
      e.preventDefault();
      cancel();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      submit('copy');
    } else if (e.ctrlKey && !e.shiftKey && (e.key === 'z' || e.key === 'Z')) {
      e.preventDefault();
      undo();
    } else if ((e.ctrlKey && (e.key === 'y' || e.key === 'Y')) ||
      (e.ctrlKey && e.shiftKey && (e.key === 'z' || e.key === 'Z'))) {
      e.preventDefault();
      redo();
    } else if (e.ctrlKey && (e.key === 's' || e.key === 'S')) {
      e.preventDefault();
      submit('save');
    } else if (!e.ctrlKey && TOOL_KEYS[e.key]) {
      if (bar.style.display === 'flex') {
        e.preventDefault();
        var t = TOOL_KEYS[e.key];
        setTool(tool === t ? null : t);
      }
    }
  });

  document.getElementById('btn-copy').addEventListener('click', function () { submit('copy'); });
  document.getElementById('btn-long').addEventListener('click', startLong);
  document.getElementById('btn-save').addEventListener('click', function () { submit('save'); });
  document.getElementById('btn-pin').addEventListener('click', function () { submit('pin'); });
  document.getElementById('btn-cancel').addEventListener('click', cancel);
  window.addEventListener('resize', resize);

  // ---------- 取图 ----------

  buildPalette();
  refreshWidthDot();

  invoke('screenshot_begin')
    .then(function (info) {
      shotW = info.width;
      shotH = info.height;
      bg.src = info.data_url;
      resize();
    })
    .catch(function (e) {
      showError('读取截图失败：' + e);
    });
})();
