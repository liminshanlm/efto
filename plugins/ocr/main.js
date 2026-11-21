// ocr/main.js —— efto 插件主进程（内嵌 V8：无 Node、无 DOM、无 fetch）。
//
// 识别直接走 设置 → 模型 里配置的模型（efto.ai.chat），插件不保存任何接口地址或密钥。
// 模型必须支持图片输入（例如 gpt-4o、qwen-vl）；图片路径交给宿主，由宿主读盘后随请求发出。
//
// 数据流：UI(选图/拖图/粘贴) --ipcMessage--> main --efto.ai.chat--> 模型 --> 结果回 UI
// 剪贴板图片由宿主落成临时 PNG（efto.pasteImage），main 拿路径后与选图走同一条流程。
// Node 只在"保存结果到文件"时用到（需在 设置 → 运行时 配 Node）。

'use strict';

var script = require('./node-script.js');

var MODES = {
  text: {
    label: '文字',
    system: '你是 OCR 引擎。只输出从图片里识别出的文字，保持原有换行与段落。'
      + '不要解释、不要补全、不要使用 markdown 代码块。看不清的字用 □ 代替。',
    ask: '识别这张图片里的全部文字。',
  },
  table: {
    label: '表格',
    system: '你是表格识别引擎。把图片里的表格转成 CSV：第一行为表头，逗号分隔，'
      + '单元格内的逗号与换行用双引号包裹。只输出 CSV，不要解释，不要 markdown 代码块。',
    ask: '识别这张图片里的表格，输出 CSV。',
  },
  formula: {
    label: '公式',
    system: '你是公式识别引擎。把图片里的公式转成 LaTeX。'
      + '只输出 LaTeX 源码，不要 $ 或 $$ 包裹，不要解释，不要 markdown 代码块。',
    ask: '识别这张图片里的公式，输出 LaTeX。',
  },
};

var state = {
  ready: false,
  busy: false,
  modelName: '',
  file: '',
  preview: '',
  mode: 'text',
  status: '',
  error: '',
  result: '',
};

function errText(e) {
  return (e && e.message) ? e.message : String(e);
}

// 表格/公式偶尔仍被模型套一层 ```csv / ```latex，去掉外层围栏。
function stripFence(text) {
  var s = String(text || '').trim();
  var m = s.match(/^```[a-zA-Z]*\s*\r?\n([\s\S]*?)\r?\n?```$/);
  return m ? m[1].trim() : s;
}

// 当前生效的模型名（仅用于显示；真正用哪个由 efto.ai.chat 的宿主侧决定）。
function modelLabel() {
  try {
    var info = efto.ai.models();
    var list = (info && info.models) || [];
    for (var i = 0; i < list.length; i++) {
      if (list[i].id === info.default && list[i].name) return list[i].name;
    }
    return list.length && list[0].name ? list[0].name : '';
  } catch (e) {
    return '';
  }
}

function push() {
  try {
    efto.ipcMessage('state', state);
  } catch (e) {
    // UI 未就绪时宿主会排队
  }
}

function doOcr(file) {
  var mode = MODES[state.mode] || MODES.text;

  state.busy = true;
  state.error = '';
  state.result = '';
  state.file = file;
  state.status = '正在识别…';
  // 左侧预览：大图由宿主缩到 640px 再回传 data URL；失败不影响识别
  try {
    state.preview = String(efto.imagePreview(file) || '');
  } catch (e) {
    state.preview = '';
    console.warn('预览失败：' + errText(e));
  }
  push();

  efto.ai.chat({
    messages: [
      { role: 'system', content: mode.system },
      {
        role: 'user',
        content: [
          { type: 'text', text: mode.ask },
          { type: 'image', path: file },
        ],
      },
    ],
    temperature: 0,
  }).then(function (res) {
    state.busy = false;
    state.status = '识别完成';
    var text = String((res && res.content) || '').trim();
    if (state.mode !== 'text') text = stripFence(text);
    state.result = text;
    if (!state.result) state.error = '模型返回了空内容，请换个更清晰的图片重试';
    // CLI（cli plugin-run）只抓日志与 output，这里补一条便于无 GUI 时验证
    console.log('OCR 完成：' + (state.error || state.result.slice(0, 200)));
    push();
  }, function (e) {
    state.busy = false;
    state.status = '';
    state.error = errText(e);
    if (state.error.indexOf('未配置模型') >= 0) {
      state.error += '\n请到 设置 → 模型 添加模型并设为默认（需支持图片输入）。';
    }
    console.error('OCR 失败：' + state.error);
    push();
  });
}

efto.onPluginEnter(function (info) {
  state.busy = false;
  state.error = '';
  state.result = '';
  state.file = '';
  state.preview = '';
  state.status = '';
  state.mode = 'text';
  state.modelName = modelLabel();
  state.ready = true;

  efto.showUI({ title: 'OCR 文字识别', width: 820, height: 640 });

  // 关键字后面的输入可以带模式前缀和图片路径，例如 "ocr 表格 D:\a.png"
  var payload = info && typeof info.payload === 'string' ? info.payload.trim() : '';
  var modeMap = { '文字': 'text', '表格': 'table', '公式': 'formula' };
  var file = payload;
  var m = payload.match(/^(文字|表格|公式)\s+(.+)$/);
  if (m) {
    state.mode = modeMap[m[1]] || 'text';
    file = m[2].trim();
  }

  if (file && /\.(png|jpe?g|webp|gif|bmp)$/i.test(file)) {
    state.file = file;
    doOcr(file);
  } else {
    // UI 未就绪时先排队，showUI 之后再补推一次
    setTimeout(function () { push(); }, 120);
  }
});

efto.onMainMessage(function (msg) {
  var name = msg && msg.name;
  var data = (msg && msg.data) || {};

  if (name === 'init') {
    push();
    return;
  }

  if (name === 'pick-image') {
    var path = typeof data.path === 'string' ? data.path.trim() : '';
    if (!path) {
      state.error = '没有拿到图片路径';
      push();
      return;
    }
    if (data.mode && MODES[data.mode]) state.mode = data.mode;
    doOcr(path);
    return;
  }

  if (name === 'paste-image') {
    var imgPath = '';
    try {
      imgPath = String(efto.pasteImage() || '');
    } catch (e) {
      state.error = errText(e);
      push();
      return;
    }
    if (!imgPath) {
      state.error = '剪贴板里没有图片：先截图或复制一张图再粘贴';
      push();
      return;
    }
    doOcr(imgPath);
    return;
  }

  if (name === 'retry') {
    if (state.file) doOcr(state.file);
    return;
  }

  if (name === 'copy') {
    var t = String(data.text || '');
    if (t) efto.copyText(t);
    return;
  }

  if (name === 'save-result') {
    var text = String(data.text || '');
    if (!text) {
      state.error = '没有可保存的内容';
      push();
      return;
    }
    var ext = state.mode === 'table' ? '.csv' : state.mode === 'formula' ? '.tex' : '.txt';
    var dir = efto.getPathByName('downloads');
    var outPath = dir + '\\OCR_' + Date.now() + ext;
    efto.node.run({
      code: script.SOURCE,
      args: [JSON.stringify({ op: 'save', outPath: outPath, text: text })],
      timeoutMs: 15000,
    }).then(function (res) {
      if (!res || !res.ok) throw new Error(String((res && res.stderr) || '写入失败').slice(0, 300));
      var out = JSON.parse(String(res.stdout || '{}'));
      if (!out.ok) throw new Error(out.error || '写入失败');
      state.status = '已保存到 ' + out.path;
      state.error = '';
      push();
      efto.shellShowItemInFolder(out.path);
    }).catch(function (e) {
      var m2 = errText(e);
      if (m2.indexOf('未配置') >= 0 || m2.indexOf('运行时') >= 0) {
        m2 += '\n保存需要 Node：请到 设置 → 运行时 填写 node.exe 路径并点「检测」。';
      }
      state.error = m2;
      push();
    });
    return;
  }

  if (name === 'open-external') {
    var u = String(data.url || '');
    if (u) efto.openExternal(u);
    return;
  }
});

efto.onUiClosed(function () {
  state.ready = false;
});
