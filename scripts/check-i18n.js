// 多语言静态检查：两份词典的 key / 占位符必须一致，页面与脚本引用的 key 必须存在。
// 用法：node scripts/check-i18n.js
'use strict';

var fs = require('fs');
var path = require('path');

var root = path.resolve(__dirname, '..');
var FE = path.join(root, 'frontend');
var LANGS = ['zh-CN', 'en-US'];

function fail(msg) {
  console.error('check-i18n: ' + msg);
  process.exitCode = 1;
}

function readJson(lang) {
  var file = path.join(FE, 'locales', lang + '.json');
  try {
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  } catch (e) {
    fail('读取 ' + lang + '.json 失败: ' + e.message);
    return {};
  }
}

var dicts = {};
LANGS.forEach(function (lang) { dicts[lang] = readJson(lang); });

// 1) key 集合必须一致
var zhKeys = Object.keys(dicts['zh-CN']).sort();
var enKeys = Object.keys(dicts['en-US']).sort();
var keySet = {};
zhKeys.forEach(function (k) { keySet[k] = true; });

zhKeys.forEach(function (k) {
  if (enKeys.indexOf(k) < 0) fail('en-US 缺少 key: ' + k);
});
enKeys.forEach(function (k) {
  if (!keySet[k]) fail('zh-CN 缺少 key: ' + k);
});

// 2) 占位符集合必须一致（{name} 是 tf() 的替换点）
function varsOf(s) {
  var out = [];
  String(s).replace(/\{(\w+)\}/g, function (m, name) { out.push(name); return m; });
  return out.sort().join(',');
}
zhKeys.forEach(function (k) {
  if (!(k in dicts['en-US'])) return;
  var a = varsOf(dicts['zh-CN'][k]);
  var b = varsOf(dicts['en-US'][k]);
  if (a !== b) fail('占位符不一致 ' + k + ': zh={' + a + '} en={' + b + '}');
});

// 3) HTML 里的 data-i18n* 引用的 key 必须存在
var htmlFiles = ['index.html', 'settings.html', 'help.html'];
var attrRe = /data-i18n(?:-html|-placeholder|-title|-aria-label)?="([^"]+)"/g;
htmlFiles.forEach(function (name) {
  var src = fs.readFileSync(path.join(FE, name), 'utf8');
  var m;
  while ((m = attrRe.exec(src))) {
    if (!keySet[m[1]]) fail(name + ' 引用了不存在的 key: ' + m[1]);
  }
});

// 4) JS 里 T('key') / TF('key') 的字面量 key 必须存在
//    动态拼接（'main.group.' + g）以点结尾，跳过并由前缀白名单兜底
var jsFiles = ['app.js', 'settings.js'];
var callRe = /\bT(?:F)?\(\s*'([^']+)'/g;
var DYNAMIC_PREFIXES = ['main.group.'];
jsFiles.forEach(function (name) {
  var src = fs.readFileSync(path.join(FE, name), 'utf8');
  var m;
  while ((m = callRe.exec(src))) {
    var key = m[1];
    if (/\.$/.test(key)) {
      if (DYNAMIC_PREFIXES.indexOf(key) < 0) fail(name + ' 出现未登记的动态 key 前缀: ' + key);
      continue;
    }
    if (!keySet[key]) fail(name + ' 引用了不存在的 key: ' + key);
  }
});

// 5) HTML 里不应残留未走词典的中文（语言名等白名单除外）
//    按标签扫描：带 data-i18n* 的元素连同其文本/属性一起放过，其余逐个检查。
var ALLOW = [/简体中文/];
var VOID = { input: 1, br: 1, img: 1, hr: 1, meta: 1, link: 1 };
var ATTRS = /(placeholder|title|aria-label|alt)="([^"]*)"/g;

function hasChinese(s) {
  return /[\u4e00-\u9fa5]/.test(s);
}

function scanHtml(name, src) {
  // 注释换成等长空白，保持行号不变
  src = src.replace(/<!--[\s\S]*?-->/g, function (m) { return m.replace(/[^\n]/g, ' '); });
  var re = /<[^>]*>|[^<]+/g;
  var stack = [{ name: 'root', skip: false }];
  var m;
  while ((m = re.exec(src))) {
    var tok = m[0];
    var line = src.slice(0, m.index).split('\n').length;
    if (tok[0] === '<') {
      var close = /^<\s*\/\s*([a-zA-Z0-9-]+)/.exec(tok);
      var open = /^<\s*([a-zA-Z0-9-]+)/.exec(tok);
      if (close) {
        var tagName = close[1].toLowerCase();
        for (var i = stack.length - 1; i > 0; i--) {
          if (stack[i].name === tagName) { stack.length = i; break; }
        }
      } else if (open) {
        var tag = open[1].toLowerCase();
        var tagged = /data-i18n/.test(tok);
        if (!tagged) {
          ATTRS.lastIndex = 0;
          var a;
          while ((a = ATTRS.exec(tok))) {
            if (hasChinese(a[2]) && !ALLOW.some(function (re) { return re.test(a[2]); })) {
              fail(name + ':' + line + ' 属性 ' + a[1] + ' 有未走词典的中文: ' + a[2]);
            }
          }
        }
        var selfClose = /\/>\s*$/.test(tok) || VOID[tag];
        if (!selfClose) {
          var parent = stack[stack.length - 1];
          stack.push({
            name: tag,
            skip: parent.skip || tagged || tag === 'script' || tag === 'style'
          });
        }
      }
    } else if (!stack[stack.length - 1].skip && hasChinese(tok)) {
      if (!ALLOW.some(function (re) { return re.test(tok); })) {
        fail(name + ':' + line + ' 有未走词典的中文: ' + tok.trim());
      }
    }
  }
}

htmlFiles.forEach(function (name) {
  scanHtml(name, fs.readFileSync(path.join(FE, name), 'utf8'));
});

if (!process.exitCode) console.log('check-i18n: OK (' + zhKeys.length + ' keys, ' + LANGS.length + ' languages)');
