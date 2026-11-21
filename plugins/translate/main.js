// translate/main.js —— 原生插件（无 UI）：把关键字后的文本交给默认模型翻译。
//
// 用法：搜索框输入 “翻译 hello” / “fy 你好世界” 回车。
// 模型配置在 设置 → 模型；这里不接触 base_url / api_key（宿主只接受设置里的模型）。
//
// 结果分两种形态（模型按输入自动判断）：
// - 单词/短语 → 词典卡片：音标、词性、英英释义、多个中文义项、2~3 个双语例句；
// - 句子/段落 → 自然译文 + 原文 + 少量难点注释。
// 每行都可以单击复制，展示形态参考主流词典（如 chat 的词典条目）。

'use strict';

var SYSTEM_PROMPT = [
  '你是给中文用户用的词典与翻译引擎。只输出一个 JSON 对象：不要 markdown 代码块，不要注释，不要任何前后解释。',
  '',
  '【第一步：判定形态】',
  'A 型 kind="word"：能在词典里查到独立词条的单个词或固定短语，例如 chat、take off、画蛇添足。',
  'B 型 kind="sentence"：完整句子、段落，以及问候语、口号、标语、感叹等成句的表达，例如 你好世界、今天好热啊、Thank you very much。',
  '判定顺序：',
  '1) 有句末标点（。！？.!?）、多分句，或英文 3 个词以上，或中文 5 个字以上 → 一律 B 型；',
  '2) 只有能确定是词典词条时才用 A 型；',
  '3) 拿不准时用 B 型。',
  '',
  '【A 型：词典卡片】',
  '{"kind":"word","headword":"原形","phonetic_uk":"/.../","phonetic_us":"/.../","part_of_speech":"verb","definition_en":"...","meanings_zh":["...","..."],"examples":[{"en":"...","zh":"..."}]}',
  '- headword 用词典原形（输入是变形就还原；输入是中文词就给最贴切的英文对应词）；',
  '- phonetic_uk / phonetic_us 写标准国际音标；英式和美式相同就只填 phonetic_uk；',
  '- part_of_speech 用小写英文缩写：noun / verb / adj / adv / prep / phrase 等；',
  '- definition_en 写一句学习者词典风格的英文释义，简明并点出用法或常见搭配，全英文；',
  '- meanings_zh 给 2~4 条按常用度排序的中文义项，每条点出语境差异，不要同义反复；',
  '- examples 给 2~3 个真实自然的例句，覆盖不同搭配或词性（如 chat to / chat with / have a chat），英文配简体中文译文；不要教科书腔，不要生造。',
  '',
  '【B 型：自然译文】',
  '{"kind":"sentence","translation":"...","key_points":["..."]}',
  '- translation 按目标语言习惯意译，不要逐字直译；不加引号，不复述原文，不加解释；',
  '- 多句输入保留原有分句和语气（疑问还是陈述、正式还是口语）；',
  '- key_points 给 0~3 条简短注释：习语、语气、易错点或文化背景，没有就给空数组。',
  '',
  '语言方向：输入以中文为主译成英文，否则译成简体中文；人名、品牌等专有名词保持原样。',
  '所有字符串用纯文本，不要出现 **、*、# 等 markdown 标记。',
  '所有字段都必须出现，没有内容用空字符串或空数组。'
].join('\n');

function errText(e) {
  return (e && e.message) ? e.message : String(e);
}

function say(title, subtitle, copy) {
  efto.output([{ title: title, subtitle: subtitle || '', copy: copy || '' }]);
}

// 模型偶尔会套一层 ```json 或前后带说明，这里只截取第一个完整 JSON 对象。
function parseModelJson(text) {
  var s = String(text || '').trim();
  s = s.replace(/^```(?:json)?\s*/i, '').replace(/\s*```$/, '');
  var start = s.indexOf('{');
  var end = s.lastIndexOf('}');
  if (start < 0 || end <= start) return null;
  try {
    return JSON.parse(s.slice(start, end + 1));
  } catch (e) {
    return null;
  }
}

function cleanList(v, max) {
  if (!Array.isArray(v)) return [];
  var out = [];
  for (var i = 0; i < v.length && out.length < max; i++) {
    var s = typeof v[i] === 'string' ? v[i].trim() : '';
    if (s) out.push(s);
  }
  return out;
}

// 只判定"明显是句子"的输入：有句读标点、换行，或英文 3 个词以上。
// 宁可放过（give up 这类短语不能被误判成句子），命中时也只是让模型重答一次。
function looksLikeSentence(text) {
  if (/[。！？!?；;，,\n]/.test(text)) return true;
  var words = text.match(/[A-Za-z][A-Za-z'’-]*/g) || [];
  return words.length >= 3;
}

// 词典卡片：每行一个可复制的信息块，和列表渲染的 title/subtitle 对齐。
function wordRows(d) {
  var rows = [];
  var headword = String(d.headword || '').trim();
  var pos = String(d.part_of_speech || '').trim();
  var head = [headword, pos].filter(Boolean).join(' · ');
  var phon = [];
  var uk = String(d.phonetic_uk || '').trim();
  var us = String(d.phonetic_us || '').trim();
  if (uk) phon.push('英 ' + uk);
  if (us) phon.push('美 ' + us);
  if (head) rows.push({ title: head, subtitle: phon.join('   ') || '词典', copy: headword || head });

  var defEn = String(d.definition_en || '').trim();
  if (defEn) rows.push({ title: defEn, subtitle: '英文释义', copy: defEn });

  var zh = cleanList(d.meanings_zh, 4);
  if (zh.length) {
    rows.push({ title: zh.join('；'), subtitle: '中文释义', copy: zh.join('；') });
  }

  var examples = Array.isArray(d.examples) ? d.examples : [];
  var n = 0;
  for (var i = 0; i < examples.length && n < 3; i++) {
    var ex = examples[i] || {};
    var en = String(ex.en || '').trim();
    if (!en) continue;
    n++;
    rows.push({ title: en, subtitle: String(ex.zh || '').trim(), copy: en });
  }
  return rows;
}

function sentenceRows(d, text) {
  var rows = [];
  var tr = String(d.translation || '').trim();
  if (tr) rows.push({ title: tr, subtitle: '译文 · 单击复制', copy: tr });
  rows.push({ title: text, subtitle: '原文', copy: text });
  cleanList(d.key_points, 3).forEach(function (p) {
    rows.push({ title: p, subtitle: '注释', copy: p });
  });
  return rows;
}

function ask(text, forceSentence) {
  var user = forceSentence
    ? text + '\n\n（程序判定：以上是句子或问候语，只能返回 B 型 kind="sentence"。）'
    : text;
  return efto.ai.chat({
    messages: [
      { role: 'system', content: SYSTEM_PROMPT },
      { role: 'user', content: user }
    ],
    temperature: 0
  });
}

function emit(res, text) {
  var raw = String((res && res.content) || '').trim();
  var data = parseModelJson(raw);
  var rows = null;
  if (data && data.kind === 'word') rows = wordRows(data);
  else if (data && data.kind === 'sentence') rows = sentenceRows(data, text);

  if (rows && rows.length) {
    efto.output(rows);
    return;
  }
  if (raw) {
    // 模型没按约定返回 JSON：原样展示，避免把结果吞掉。
    console.warn('翻译结果不是预期 JSON，按原文展示');
    efto.output([
      { title: raw, subtitle: '模型原始输出 · 单击复制', copy: raw },
      { title: text, subtitle: '原文', copy: text }
    ]);
    return;
  }
  console.error('翻译失败：模型返回空内容');
  say('翻译失败', '模型返回空内容，请检查 设置 → 模型', text);
}

efto.onPluginEnter(function (action) {
  var text = action && typeof action.payload === 'string' ? action.payload.trim() : '';
  if (!text) {
    say('请输入要翻译的文本', '例如：fy hello world，或 翻译 你好世界');
    return;
  }

  say('正在翻译…', '调用默认模型', text);

  ask(text).then(function (res) {
    var data = parseModelJson(String((res && res.content) || ''));
    // 输入明显是句子，模型却给了词条（如「你好世界」→ Hello, World · phrase）：
    // 带着明确指令重问一次，第二次仍不合规就按原样展示，不做无限重试。
    if (data && data.kind === 'word' && looksLikeSentence(text)) {
      return ask(text, true).then(function (retry) {
        emit(retry, text);
      });
    }
    emit(res, text);
  }).catch(function (e) {
    var msg = errText(e);
    // 同时进日志：CLI（cli plugin-run）能拿到 [plugin-log:error]，不依赖列表行
    console.error('翻译失败：' + msg);
    say('翻译失败', msg, text);
  });
});
