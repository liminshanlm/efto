'use strict';

var SYSTEM_PROMPT =
  '你是运行在 efto 启动器里的 AI 助手。回答准确、简洁，需要展开时用 Markdown。' +
  '不确定的内容要说明，不要编造。';

function errorText(e) {
  if (!e) return '未知错误';
  if (typeof e === 'string') return e;
  return e.message || String(e);
}

efto.onPluginEnter(function (action) {
  efto.showUI({ title: 'AI 对话', width: 980, height: 680 });
  var payload = action && typeof action.payload === 'string' ? action.payload.trim() : '';
  if (payload) efto.ipcMessage('prefill', { text: payload });
});

efto.onMainMessage(function (msg) {
  var name = msg && msg.name;
  var data = (msg && msg.data) || {};

  if (name === 'init') {
    efto.ipcMessage('models', efto.ai.models());
    return;
  }

  if (name === 'ask') {
    var id = data.id;
    var messages = data.messages;
    if (!id || !Array.isArray(messages) || messages.length === 0) {
      efto.ipcMessage('error', { id: id || '', message: '请求缺少 messages' });
      return;
    }
    var options = {
      messages: [{ role: 'system', content: SYSTEM_PROMPT }].concat(messages),
      temperature: 0.7,
      timeoutSecs: 300,
      onDelta: function (delta) {
        efto.ipcMessage('delta', {
          id: id,
          text: delta.text || '',
          reasoning: delta.reasoning || '',
          toolEvent: delta.toolEvent || null
        });
      }
    };
    if (data.model) options.model = data.model;
    if (data.webSearch === true) {
      options.webSearch = true;
      options.readUrl = true;
    }

    try {
      efto.ai.chat(options).then(function (res) {
        efto.ipcMessage('done', {
          id: id,
          content: res.content || '',
          reasoning: res.reasoning || '',
          model: res.modelName || res.model || '',
          usage: res.usage || null,
          serverToolCalls: res.serverToolCalls || null,
          annotations: res.annotations || null
        });
      }).catch(function (e) {
        efto.ipcMessage('error', { id: id, message: errorText(e) });
      });
    } catch (e) {
      efto.ipcMessage('error', { id: id, message: errorText(e) });
    }
    return;
  }

  // 文档 OCR：UI 只给路径，文件字节与密钥都留在宿主侧
  if (name === 'ocr') {
    var ocrPaths = Array.isArray(data.paths) ? data.paths : [];
    for (var k = 0; k < ocrPaths.length; k++) {
      (function (path) {
        var fileName = path.split(/[\\/]/).pop();
        efto.ai.ocr({ path: path }).then(function (res) {
          var text = (res.pages || []).map(function (p) { return p.text || ''; }).join('\n\n').trim();
          if (!text) {
            efto.ipcMessage('ocr', { id: data.id, path: path, ok: false, error: '没有识别出文字' });
            return;
          }
          efto.ipcMessage('ocr', {
            id: data.id,
            path: path,
            ok: true,
            name: fileName,
            pageCount: res.pageCount || (res.pages || []).length,
            text: text
          });
        }).catch(function (e) {
          efto.ipcMessage('ocr', { id: data.id, path: path, ok: false, error: errorText(e) });
        });
      })(ocrPaths[k]);
    }
    return;
  }

  // 选中图片后由 main 读盘做预览（UI 侧拿不到文件字节）
  if (name === 'preview') {
    var paths = Array.isArray(data.paths) ? data.paths : [];
    var items = [];
    for (var i = 0; i < paths.length; i++) {
      var path = paths[i];
      var item = { path: path, preview: '' };
      try {
        item.preview = efto.imagePreview(path);
      } catch (e) {
        item.error = errorText(e);
      }
      items.push(item);
    }
    efto.ipcMessage('previews', { id: data.id, items: items });
    return;
  }

  if (name === 'paste-image') {
    var pasted = '';
    try {
      pasted = efto.pasteImage();
    } catch (e) {
      efto.ipcMessage('paste-image', { path: '', error: errorText(e) });
      return;
    }
    if (!pasted) {
      efto.ipcMessage('paste-image', { path: '' });
      return;
    }
    var preview = '';
    try {
      preview = efto.imagePreview(pasted);
    } catch (e) {
      efto.ipcMessage('paste-image', { path: '', error: errorText(e) });
      return;
    }
    efto.ipcMessage('paste-image', { path: pasted, preview: preview });
  }
});
