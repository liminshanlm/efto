// ipmsg/ui.js —— 插件 UI（普通 webview）。
// 本机 Node 服务由 main 拉起；这里直接访问 127.0.0.1 的本地控制口，
// 轮询状态、发送消息、处理授权，并把聊天记录 / 信任列表存进插件 kv。
// 协议：服务只认查询串里的令牌，请求体用 text/plain 避免 CORS 预检。

(function () {
  'use strict';

  var statusEl = document.getElementById('status');
  var statusTextEl = document.getElementById('statusText');
  var selfInfoEl = document.getElementById('selfInfo');
  var errorEl = document.getElementById('error');
  var errorTextEl = document.getElementById('errorText');
  var retryBtnEl = document.getElementById('retryBtn');
  var requestBoxEl = document.getElementById('requests');
  var peersEl = document.getElementById('peers');
  var peerCountEl = document.getElementById('peerCount');
  var emptyPeersEl = document.getElementById('emptyPeers');
  var portInfoEl = document.getElementById('portInfo');
  var chatEmptyEl = document.getElementById('chatEmpty');
  var chatPaneEl = document.getElementById('chatPane');
  var chatNameEl = document.getElementById('chatName');
  var chatAddrEl = document.getElementById('chatAddr');
  var chatBadgeEl = document.getElementById('chatBadge');
  var messagesEl = document.getElementById('messages');
  var approvalEl = document.getElementById('approval');
  var approvalNameEl = document.getElementById('approvalName');
  var approveBtnEl = document.getElementById('approveBtn');
  var denyBtnEl = document.getElementById('denyBtn');
  var composerHintEl = document.getElementById('composerHint');
  var composerRowEl = document.getElementById('composerRow');
  var inputEl = document.getElementById('input');
  var sendBtnEl = document.getElementById('sendBtn');
  var toastEl = document.getElementById('toast');

  var svc = { state: 'waiting', port: 24251, token: '' };
  var localReady = false; // kv 里的信任列表必须先载入，之后才能 /init，否则会覆盖服务端状态
  var state = { running: false, port: 24250, self: null, peers: [], requests: [], error: '', unread: {} };
  var threads = {};
  var trusted = [];
  var approved = [];
  var selected = '';
  var live = {};
  var pollTimer = 0;
  var connecting = false;
  var connected = false;
  var failCount = 0;
  var toastTimer = 0;
  var lastSentText = '';

  function errText(e) {
    return e && e.message ? e.message : String(e);
  }

  function toast(text) {
    toastEl.textContent = text;
    toastEl.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(function () { toastEl.hidden = true; }, 2000);
  }

  function fmtTime(ts) {
    var d = new Date(Number(ts) || Date.now());
    var h = d.getHours();
    var m = d.getMinutes();
    return (h < 10 ? '0' : '') + h + ':' + (m < 10 ? '0' : '') + m;
  }

  // ---------------- kv（UI 侧是异步的） ----------------

  function kvGet(key) {
    return efto.kv.get(key).catch(function () { return null; });
  }

  function strList(v) {
    var out = [];
    if (Array.isArray(v)) {
      for (var i = 0; i < v.length; i++) {
        if (typeof v[i] === 'string' && v[i]) out.push(v[i]);
      }
    }
    return out;
  }

  function parseThreads(v) {
    var out = {};
    if (!v || typeof v !== 'object') return out;
    for (var id in v) {
      var t = v[id];
      if (!t || typeof t !== 'object' || !Array.isArray(t.messages)) continue;
      var list = [];
      for (var i = 0; i < t.messages.length; i++) {
        var m = t.messages[i];
        if (!m || (m.dir !== 'in' && m.dir !== 'out') || typeof m.text !== 'string') continue;
        list.push({ dir: m.dir, text: m.text, ts: Number(m.ts) || 0, pending: !!m.pending });
      }
      out[id] = { name: typeof t.name === 'string' && t.name ? t.name : id, messages: list };
      trimThread(out[id]);
    }
    return out;
  }

  function saveThreads() {
    efto.kv.set('threads', threads).catch(function () {});
  }

  function saveTrust() {
    efto.kv.set('trusted', trusted).catch(function () {});
    efto.kv.set('approved', approved).catch(function () {});
  }

  function loadLocal() {
    return Promise.all([kvGet('trusted'), kvGet('approved'), kvGet('threads')])
      .then(function (res) {
        trusted = strList(res[0]);
        approved = strList(res[1]);
        threads = parseThreads(res[2]);
      });
  }

  // ---------------- 本地控制口 ----------------

  function api(path, body) {
    if (!svc.token) return Promise.reject(new Error('服务尚未就绪'));
    var url = 'http://127.0.0.1:' + svc.port + path + '?t=' + encodeURIComponent(svc.token);
    var opts = { method: body ? 'POST' : 'GET', cache: 'no-store' };
    if (body) {
      opts.headers = { 'content-type': 'text/plain;charset=UTF-8' };
      opts.body = JSON.stringify(body);
    }
    return fetch(url, opts).then(function (res) {
      return res.text().then(function (text) {
        var data = null;
        try {
          data = text ? JSON.parse(text) : null;
        } catch (e) {
          data = null;
        }
        if (!res.ok || !data) {
          throw new Error((data && data.error) || ('HTTP ' + res.status));
        }
        return data;
      });
    });
  }

  function serviceError(e) {
    connected = false;
    connecting = false;
    if (pollTimer) {
      clearInterval(pollTimer);
      pollTimer = 0;
    }
    state.running = false;
    state.error = '消息服务不可用：' + errText(e) +
      '\n需要本机安装 Node，并在 设置 → 运行时 中配置 node.exe 后点“检测”；' +
      '另请确认 24250 / 24251 端口未被占用。';
    render();
  }

  function connect(attempt) {
    if (connected || connecting) return;
    connecting = true;
    api('/init', { trusted: trusted, approved: approved }).then(function () {
      connecting = false;
      connected = true;
      failCount = 0;
      state.running = true;
      state.error = '';
      render();
      poll();
      if (!pollTimer) pollTimer = setInterval(poll, 800);
    }).catch(function (e) {
      connecting = false;
      if (attempt < 15 && svc.state === 'ready') {
        setTimeout(function () {
          if (svc.state === 'ready' && !connected) connect(attempt + 1);
        }, 300);
      } else {
        serviceError(e);
      }
    });
  }

  function poll() {
    if (!connected) return;
    api('/status').then(function (st) {
      failCount = 0;
      state.running = !!st.running;
      state.error = '';
      apply(st);
      render();
    }).catch(function (e) {
      failCount++;
      if (failCount >= 5) serviceError(e);
    });
  }

  function restart() {
    connected = false;
    connecting = false;
    if (pollTimer) {
      clearInterval(pollTimer);
      pollTimer = 0;
    }
    svc.state = 'starting';
    state.running = false;
    state.error = '';
    render();
    efto.ipcMessage('restart', {}).catch(function () {});
  }

  // ---------------- 状态应用 ----------------

  function ensureThread(peerId, name) {
    var t = threads[peerId];
    if (!t) {
      t = threads[peerId] = { name: name || peerId, messages: [] };
    } else if (name && t.name !== name) {
      t.name = name;
    }
    return t;
  }

  function trimThread(t) {
    if (t.messages.length > 200) t.messages.splice(0, t.messages.length - 200);
  }

  function apply(st) {
    state.self = st.self || state.self;
    state.port = st.port || state.port;
    state.peers = Array.isArray(st.peers) ? st.peers : [];
    state.requests = Array.isArray(st.requests) ? st.requests : [];

    var trustChanged = false;
    var refreshed = {};
    var refreshedAny = false;

    for (var i = 0; i < state.peers.length; i++) {
      var p = state.peers[i];
      if (p.outboundOk && approved.indexOf(p.id) < 0) {
        approved.push(p.id);
        trustChanged = true;
      }
      if (p.allowed && trusted.indexOf(p.id) < 0) {
        trusted.push(p.id);
        trustChanged = true;
      }
      if (threads[p.id] && p.name) threads[p.id].name = p.name;
      if (p.outboundOk && threads[p.id]) {
        var msgs = threads[p.id].messages;
        for (var k = 0; k < msgs.length; k++) {
          if (msgs[k].dir === 'out' && msgs[k].pending) {
            msgs[k].pending = false;
            refreshed[p.id] = true;
          }
        }
      }
    }
    if (trustChanged) saveTrust();
    for (var rid in refreshed) refreshedAny = true;
    if (refreshedAny) saveThreads();

    var list = Array.isArray(st.messages) ? st.messages : [];
    var dirty = false;
    for (var j = 0; j < list.length; j++) {
      var m = list[j];
      if (!m || typeof m.text !== 'string') continue;
      var t = ensureThread(m.from, m.fromName);
      t.messages.push({ dir: 'in', text: m.text, ts: Number(m.ts) || Date.now() });
      trimThread(t);
      if (selected !== m.from) state.unread[m.from] = (state.unread[m.from] || 0) + 1;
      dirty = true;
    }
    if (dirty) saveThreads();
  }

  // ---------------- 渲染 ----------------

  function refreshLive() {
    live = {};
    var list = state.peers || [];
    for (var i = 0; i < list.length; i++) {
      if (list[i] && list[i].id) live[list[i].id] = list[i];
    }
  }

  function requestFor(peerId) {
    var list = state.requests || [];
    for (var i = 0; i < list.length; i++) {
      if (list[i] && list[i].id === peerId) return list[i];
    }
    return null;
  }

  function conversations() {
    var out = [];
    var seen = {};
    var list = state.peers || [];
    for (var i = 0; i < list.length; i++) {
      var p = list[i];
      if (!p || !p.id) continue;
      seen[p.id] = true;
      out.push({ id: p.id, name: p.name || p.id, ip: p.ip || '', online: true, peer: p });
    }
    var offline = [];
    for (var id in threads) {
      if (seen[id]) continue;
      var t = threads[id];
      var last = t.messages.length ? t.messages[t.messages.length - 1].ts : 0;
      offline.push({ id: id, name: t.name || id, ip: '', online: false, peer: null, last: last });
    }
    offline.sort(function (a, b) { return b.last - a.last; });
    return out.concat(offline);
  }

  function conversationOf(peerId) {
    if (live[peerId]) {
      return {
        id: peerId,
        name: live[peerId].name || peerId,
        ip: live[peerId].ip || '',
        online: true,
        peer: live[peerId]
      };
    }
    var t = threads[peerId];
    if (t) return { id: peerId, name: t.name || peerId, ip: '', online: false, peer: null };
    return null;
  }

  function peerRow(c) {
    var li = document.createElement('li');
    li.className = 'peer' + (c.id === selected ? ' active' : '') + (c.online ? '' : ' offline');
    li.dataset.peerId = c.id;

    var dot = document.createElement('span');
    dot.className = 'dot';
    li.appendChild(dot);

    var main = document.createElement('span');
    main.className = 'peer-main';

    var name = document.createElement('span');
    name.className = 'peer-name';
    name.textContent = c.name;
    main.appendChild(name);

    var sub = document.createElement('span');
    sub.className = 'peer-sub';
    sub.textContent = c.online ? (c.ip || '在线') : '离线';
    main.appendChild(sub);
    li.appendChild(main);

    var tags = document.createElement('span');
    tags.className = 'peer-tags';

    if (requestFor(c.id)) {
      var tag = document.createElement('span');
      tag.className = 'tag warn';
      tag.textContent = '待你同意';
      tags.appendChild(tag);
    }
    var n = (state.unread || {})[c.id] || 0;
    if (n > 0) {
      var badge = document.createElement('span');
      badge.className = 'tag unread';
      badge.textContent = n > 99 ? '99+' : String(n);
      tags.appendChild(badge);
    }
    li.appendChild(tags);

    li.addEventListener('click', function () { select(c.id); });
    return li;
  }

  function renderPeers() {
    refreshLive();
    peersEl.innerHTML = '';
    var list = conversations();
    for (var i = 0; i < list.length; i++) peersEl.appendChild(peerRow(list[i]));

    peerCountEl.textContent = String(Object.keys(live).length);
    emptyPeersEl.hidden = list.length > 0;
    if (svc.state === 'error') {
      emptyPeersEl.textContent = '消息服务未启动：需要本机安装 Node 并在 设置 → 运行时 中配置。';
    } else if (state.running) {
      emptyPeersEl.textContent = '正在发现同网段设备…若一直为空，请检查 Windows 防火墙是否允许 node.exe 访问专用网络。';
    } else {
      emptyPeersEl.textContent = '正在启动本机消息服务…';
    }
  }

  function renderRequests() {
    requestBoxEl.innerHTML = '';
    var list = state.requests || [];
    requestBoxEl.hidden = list.length === 0;
    for (var i = 0; i < list.length; i++) {
      var req = list[i];
      var row = document.createElement('div');
      row.className = 'request';

      var text = document.createElement('span');
      text.className = 'request-text';
      text.textContent = (req.name || req.id) + ' 请求与你通信';
      row.appendChild(text);

      var actions = document.createElement('span');
      actions.className = 'request-actions';

      var ok = document.createElement('button');
      ok.type = 'button';
      ok.className = 'btn mini primary';
      ok.textContent = '同意';
      ok.addEventListener('click', function (id) {
        return function () { doRespond(id, true); };
      }(req.id));
      actions.appendChild(ok);

      var no = document.createElement('button');
      no.type = 'button';
      no.className = 'btn mini danger';
      no.textContent = '拒绝';
      no.addEventListener('click', function (id) {
        return function () { doRespond(id, false); };
      }(req.id));
      actions.appendChild(no);

      row.appendChild(actions);
      requestBoxEl.appendChild(row);
    }
  }

  function badgeOf(c) {
    if (!c) return { text: '', cls: '' };
    if (!c.online) return { text: '离线', cls: 'off' };
    var p = c.peer || {};
    if (p.outboundOk && p.allowed) return { text: '已互信', cls: 'ok' };
    if (p.outboundOk) return { text: '可发送', cls: 'ok' };
    if (p.pending) return { text: '待你同意', cls: 'warn' };
    if (p.allowed) return { text: '可接收', cls: 'warn' };
    return { text: '未授权', cls: 'warn' };
  }

  function emptyLine(text) {
    var p = document.createElement('p');
    p.className = 'msg-empty';
    p.textContent = text;
    return p;
  }

  function renderMessages(c) {
    var t = threads[c.id] || { messages: [] };
    messagesEl.innerHTML = '';
    if (!t.messages.length) {
      messagesEl.appendChild(emptyLine(c.online
        ? '还没有消息，打个招呼吧。'
        : '对方当前离线，历史消息仍保留在这里。'));
      return;
    }
    for (var i = 0; i < t.messages.length; i++) {
      var m = t.messages[i];
      var row = document.createElement('div');
      row.className = 'msg ' + (m.dir === 'in' ? 'in' : 'out');

      var bubble = document.createElement('div');
      bubble.className = 'bubble';
      bubble.textContent = m.text;
      row.appendChild(bubble);

      var meta = document.createElement('div');
      meta.className = 'msg-meta';
      meta.textContent = fmtTime(m.ts) + (m.pending ? ' · 待对方同意' : '');
      row.appendChild(meta);

      messagesEl.appendChild(row);
    }
    messagesEl.scrollTop = messagesEl.scrollHeight;
  }

  function renderChat() {
    var c = selected ? conversationOf(selected) : null;
    if (!c) {
      chatEmptyEl.hidden = false;
      chatPaneEl.hidden = true;
      return;
    }
    chatEmptyEl.hidden = true;
    chatPaneEl.hidden = false;

    chatNameEl.textContent = c.name;
    chatAddrEl.textContent = c.online ? (c.ip || '') : '离线';
    var badge = badgeOf(c);
    chatBadgeEl.textContent = badge.text;
    chatBadgeEl.className = 'badge ' + badge.cls;

    var pendingReq = c.peer && c.peer.pending;
    approvalEl.hidden = !pendingReq;
    composerRowEl.hidden = !!pendingReq;
    if (pendingReq) {
      approvalNameEl.textContent = c.name;
    } else {
      var hint = '';
      if (!c.online) hint = '对方当前不在线，消息暂时发不出去。';
      else if (!c.peer.outboundOk) hint = '首次发送需要对方同意，消息会暂存并在对方同意后自动发出。';
      composerHintEl.textContent = hint;
      composerHintEl.hidden = !hint;
    }

    renderMessages(c);
  }

  function renderStatus() {
    var on = svc.state === 'ready' && state.running;
    statusEl.dataset.state = on ? 'on' : 'off';
    if (svc.state === 'starting') statusTextEl.textContent = '正在启动';
    else if (svc.state === 'error') statusTextEl.textContent = '需要 Node';
    else statusTextEl.textContent = on ? ('已启动 · ' + state.port) : '未启动';

    var self = state.self || {};
    if (self.name || self.ip) {
      selfInfoEl.textContent = (self.name || '') + (self.ip ? ' · ' + self.ip : '');
      selfInfoEl.title = self.id ? ('标识：' + self.id) : '';
      selfInfoEl.hidden = false;
    } else {
      selfInfoEl.hidden = true;
    }

    portInfoEl.textContent = state.running ? ('监听端口 ' + state.port) : '';
    errorTextEl.textContent = state.error || '';
    errorEl.hidden = !state.error;
    retryBtnEl.hidden = !(svc.state === 'error' || (state.error && !connected));
  }

  function render() {
    renderStatus();
    renderPeers();
    renderRequests();
    renderChat();
  }

  // ---------------- 交互 ----------------

  function select(peerId) {
    selected = peerId;
    if (state.unread) state.unread[peerId] = 0;
    renderPeers();
    renderChat();
    inputEl.focus();
  }

  function sendText() {
    var text = inputEl.value.trim();
    if (!selected) {
      toast('请先选择联系人');
      return;
    }
    if (!text) return;
    if (!connected) {
      toast('消息服务尚未启动');
      return;
    }
    lastSentText = text;
    inputEl.value = '';
    var peerId = selected;
    api('/send', { peerId: peerId, text: text }).then(function (r) {
      if (!r.ok) {
        toast('发送失败：' + (r.error || '未知错误'));
        if (!inputEl.value) inputEl.value = lastSentText;
        return;
      }
      var t = ensureThread(peerId);
      t.messages.push({ dir: 'out', text: text, ts: Date.now(), pending: !!r.pending });
      trimThread(t);
      saveThreads();
      renderChat();
      if (r.pending) toast('对方尚未同意，消息已暂存');
      poll();
    }).catch(function (e) {
      toast('发送失败：' + errText(e));
      if (!inputEl.value) inputEl.value = lastSentText;
    });
  }

  function doRespond(peerId, accept) {
    if (!connected) {
      toast('消息服务尚未启动');
      return;
    }
    api('/respond', { peerId: peerId, accept: accept }).then(function (r) {
      if (!r.ok) {
        toast('处理请求失败：' + (r.error || '未知错误'));
        return;
      }
      var idx = trusted.indexOf(peerId);
      if (accept) {
        if (idx < 0) trusted.push(peerId);
      } else if (idx >= 0) {
        trusted.splice(idx, 1);
      }
      saveTrust();
      if (accept) toast(r.offline ? '已同意；对方当前离线，上线后会重试' : '已同意接收');
      else toast('已拒绝');
      poll();
    }).catch(function (e) {
      toast('处理请求失败：' + errText(e));
    });
  }

  sendBtnEl.addEventListener('click', sendText);

  inputEl.addEventListener('keydown', function (ev) {
    if (ev.key === 'Enter' && !ev.shiftKey && !ev.isComposing) {
      ev.preventDefault();
      sendText();
    }
  });

  approveBtnEl.addEventListener('click', function () {
    if (selected) doRespond(selected, true);
  });
  denyBtnEl.addEventListener('click', function () {
    if (selected) doRespond(selected, false);
  });

  retryBtnEl.addEventListener('click', restart);

  efto.onMessage('svc', function (info) {
    if (!info) return;
    svc.state = info.state || 'waiting';
    svc.port = info.port || 24251;
    svc.token = info.token || svc.token;

    if (svc.state === 'ready') {
      failCount = 0;
      if (connected) poll();
      else if (localReady) connect(0);
      render();
    } else if (svc.state === 'error') {
      serviceError(new Error(info.error || 'Node 服务启动失败'));
    } else {
      state.running = false;
      state.error = '';
      render();
    }
  });

  // UI 关闭时通知服务退出；main 不再重复发停止请求，避免「刚关就重开」抢跑。
  window.addEventListener('pagehide', function () {
    if (!connected || !svc.token) return;
    try {
      navigator.sendBeacon('http://127.0.0.1:' + svc.port + '/stop?t=' + encodeURIComponent(svc.token));
    } catch (e) { /* 服务可能已经退出 */ }
  });

  loadLocal().then(function () {
    localReady = true;
    render();
    if (svc.state === 'ready' && !connected) connect(0);
    efto.ipcMessage('init', {}).catch(function () {});
  });
})();
