'use strict';
// Ctool → efto 适配层（main 侧）。
// 职责只有一件：把「用户敲的触发词 + 后面的输入」翻译成 Ctool 的路由 + 预填内容，
// 推给 UI 页面。Ctool 本体是纯前端应用，所有能力都在 UI 里跑，main 不承担业务逻辑。
//
// 运行在 efto 的内嵌 V8：没有 Node、没有 DOM、没有 fetch，只能用 efto.*。

var routes = require('./lib/routes');

var current = { route: '/', input: '' };

function normalize(s) {
  return typeof s === 'string' ? s.trim().toLowerCase() : '';
}

/** 触发词 → 路由。命中不了就回首页（首页会跳到上次使用的工具）。 */
function resolveRoute(cmd, payload) {
  var key = normalize(cmd);
  var input = typeof payload === 'string' ? payload : (payload == null ? '' : String(payload));

  // 首页总入口：把后面的输入原样带过去，由 Ctool 自己决定怎么处理。
  if (!key || key === 'ctool' || key === '程序开发常用工具' || key === '开发工具') {
    return { route: '/', input: input };
  }

  var hit = routes.triggers[key];
  if (!hit) return { route: '/', input: input };

  var tool = routes.tools[hit.tool];
  if (!tool) return { route: '/', input: input };

  var target = tool.router;
  if (hit.feature) {
    for (var i = 0; i < tool.features.length; i++) {
      if (tool.features[i].feature === hit.feature) { target = tool.features[i].router; break; }
    }
  } else if (!tool.simple) {
    // 多功能工具：没有指定功能时进第一个功能页，和 Ctool 自己的入口行为一致。
    target = tool.features[0].router;
  }
  return { route: target, input: input };
}

function push() {
  try {
    efto.ipcMessage('route', current);
  } catch (e) {
    // UI 还没就绪时宿主会排队，这里一般不会失败；失败也只是白屏，交给页面超时兜底。
  }
}

efto.onPluginEnter(function (info) {
  var cmd = info && info.code ? String(info.code) : '';
  var payload = info && info.payload;
  current = resolveRoute(cmd, payload);
  push();
});

// UI 页面每次加载（含 reload）都来拉一次当前路由：
// 宿主复用同一个 UI 窗口，页面重载后收不到之前的消息，只能主动问。
efto.onMainMessage(function (msg) {
  if (!msg || msg.name !== 'pull') return;
  push();
});
