// 预览窗口逻辑：文本 / 图片 / 视频三种内容。
// 网页预览走独立的 preview-web 窗口（直接加载目标地址），不经过这个页面。
(function () {
  'use strict';

  var I = window.__TAURI_INTERNALS__;
  var titleEl = document.getElementById('pv-title');
  var subEl = document.getElementById('pv-sub');
  var kindEl = document.getElementById('pv-kind');
  var noteEl = document.getElementById('pv-note');
  var textEl = document.getElementById('pv-text');
  var mediaEl = document.getElementById('pv-media');
  var imgEl = document.getElementById('pv-img');
  var videoEl = document.getElementById('pv-video');
  var errorEl = document.getElementById('pv-error');
  var frameEl = document.getElementById('pv-frame');
  var metaEl = document.getElementById('pv-meta');
  var openBtn = document.getElementById('pv-open');
  var closeBtn = document.getElementById('pv-close');

  var KIND_LABEL = { text: '文本', image: '图片', video: '视频', html: '网页', web: '网页', pdf: 'PDF', meta: '文件' };

  function invoke(cmd, payload) {
    if (!I || typeof I.invoke !== 'function') return Promise.reject(new Error('IPC 未就绪'));
    return I.invoke(cmd, payload || {});
  }

  function parsePayload(p) {
    if (typeof p !== 'string') return p;
    try { return JSON.parse(p); } catch (e) { return p; }
  }

  function listen(name, handler) {
    var cb = I.transformCallback(function (ev) {
      handler(parsePayload(ev && ev.payload));
    }, false);
    return invoke('plugin:event|listen', {
      event: name,
      target: { kind: 'Any' },
      handler: cb
    });
  }

  function toSrc(path) {
    try { return I.convertFileSrc(path); } catch (e) { return path; }
  }

  function reset() {
    textEl.hidden = true;
    textEl.textContent = '';
    mediaEl.hidden = true;
    imgEl.hidden = true;
    imgEl.removeAttribute('src');
    videoEl.hidden = true;
    videoEl.removeAttribute('src');
    frameEl.hidden = true;
    frameEl.removeAttribute('src');
    metaEl.hidden = true;
    metaEl.textContent = '';
    errorEl.hidden = true;
  }

  function showError() {
    errorEl.hidden = false;
  }

  function render(p) {
    if (!p) return;
    titleEl.textContent = p.title || '预览';
    subEl.textContent = p.subtitle || '';
    kindEl.textContent = KIND_LABEL[p.kind] || '';
    noteEl.textContent = p.truncated ? '内容已截断：仅显示前 512 KiB' : '';
    reset();

    if (p.kind === 'text') {
      textEl.textContent = p.content || '';
      textEl.hidden = false;
      return;
    }
    if (p.kind === 'image') {
      mediaEl.hidden = false;
      if (!p.path) { showError(); return; }
      imgEl.hidden = false;
      imgEl.src = toSrc(p.path);
      return;
    }
    if (p.kind === 'video') {
      mediaEl.hidden = false;
      if (!p.path) { showError(); return; }
      videoEl.hidden = false;
      videoEl.src = toSrc(p.path);
      return;
    }
    if (p.kind === 'html' || p.kind === 'pdf') {
      // 本地页面 / PDF 走 iframe：外层是我们自己的窗口，按钮与 Shift 关闭都可用
      if (!p.path) { mediaEl.hidden = false; showError(); return; }
      frameEl.hidden = false;
      frameEl.src = toSrc(p.path);
      return;
    }
    if (p.kind === 'meta') {
      // 不支持内联预览的文件/目录：展示属性信息
      (p.fields || []).forEach(function (pair) {
        var dt = document.createElement('dt');
        dt.textContent = pair[0];
        var dd = document.createElement('dd');
        dd.textContent = pair[1];
        metaEl.appendChild(dt);
        metaEl.appendChild(dd);
      });
      metaEl.hidden = false;
      return;
    }
  }

  imgEl.addEventListener('error', showError);
  videoEl.addEventListener('error', showError);

  // 关闭：收起预览（主窗口会把焦点收回搜索框）
  closeBtn.addEventListener('click', function () {
    invoke('preview_hide').catch(function () {});
  });

  // 打开：交给系统默认程序（文件用关联程序，网址用默认浏览器）
  openBtn.addEventListener('click', function () {
    invoke('preview_open').catch(function (err) {
      console.error('打开失败', err);
    });
  });

  // 焦点落到预览窗口时（WebView2 有时会把键盘焦点拿过去），
  // 再按一次 Shift 也要能关闭，和主窗口里的开关行为保持一致。
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Shift' && !e.repeat) invoke('preview_hide').catch(function () {});
  });

  if (I && I.transformCallback) {
    listen('preview:update', render);
    // 窗口是懒创建的：这一轮内容可能比监听先到，加载完主动取一次最新值
    invoke('preview_ready').then(render).catch(function () {});
  }
})();
