// hosts/main.js —— 运行在内嵌 V8（无 Node、无 DOM）。
// 读写与提权都在宿主：efto.hosts.read() 同步，efto.hosts.save() 会弹 UAC（Promise）。
// 这里只负责状态中转：UI 编辑完整文本，main 交给宿主校验并落盘。

'use strict';

var state = {
  path: '',
  backup: '',
  encoding: 'utf8',
  text: '',
  bytes: 0,
  saving: false,
  status: '',
  error: ''
};

function errText(e) {
  return (e && e.message) ? e.message : String(e);
}

function push() {
  try {
    efto.ipcMessage('state', state);
  } catch (e) {
    // UI 还没就绪时宿主会排队；这里只可能是插件进程正在退出
  }
}

function read() {
  state.error = '';
  state.status = '';
  try {
    var res = efto.hosts.read() || {};
    state.path = res.path || '';
    state.backup = res.backup || '';
    state.encoding = res.encoding === 'ansi' ? 'ansi' : 'utf8';
    state.text = typeof res.text === 'string' ? res.text : '';
    state.bytes = Number(res.bytes) || 0;
  } catch (e) {
    state.error = '读取 hosts 失败：' + errText(e);
    state.text = '';
  }
  push();
}

efto.onPluginEnter(function () {
  read();
  efto.showUI({ title: 'hosts 修改', width: 880, height: 640 });
});

efto.onMainMessage(function (msg) {
  var name = msg && msg.name;
  var data = (msg && msg.data) || {};

  if (name === 'init' || name === 'reload') {
    read();
    return;
  }

  if (name === 'save') {
    if (state.saving) return;
    state.saving = true;
    state.error = '';
    state.status = '正在请求管理员权限…';
    push();

    var text = typeof data.text === 'string' ? data.text : '';
    efto.hosts.save({ text: text, encoding: state.encoding })
      .then(function (res) {
        state.saving = false;
        var backup = res && res.backup ? res.backup : state.backup;
        read();
        state.status = '已保存' + (backup ? '，原文件备份到 ' + backup : '');
        push();
      })
      .catch(function (e) {
        state.saving = false;
        state.status = '';
        state.error = errText(e);
        push();
      });
  }
});
