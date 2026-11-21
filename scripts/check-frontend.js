// 前端静态守卫：检查 "被 JS 用 .hidden 切换、但 CSS 里设了 display 的元素"。
//
// 为什么需要：浏览器 UA 样式表里的 [hidden] { display: none } 优先级低于作者样式，
// 只要给该元素写过 display（比如 .empty { display:flex }），JS 里 `el.hidden = true`
// 就完全无效——元素继续可见并且继续拦截鼠标事件。这个 bug 静默、难察觉，
// 本项目就被它吃掉过整个列表的 click/dblclick。
//
// 用法：node scripts/check-frontend.js   （非零退出表示有问题）
'use strict';

const fs = require('fs');
const path = require('path');

const root = path.join(__dirname, '..', 'frontend');
const js = fs.readFileSync(path.join(root, 'app.js'), 'utf8');
const css = fs.readFileSync(path.join(root, 'style.css'), 'utf8');

// 1) 找出 JS 里通过 .hidden 切换的元素变量名 -> 对应的 getElementById id
const toggled = new Set();
for (const m of js.matchAll(/(\w+)\.(hidden)\s*=/g)) toggled.add(m[1]);
const ids = [];
for (const name of toggled) {
  const decl = js.match(new RegExp("var\\s+" + name + "\\s*=\\s*document\\.getElementById\\('([^']+)'\\)"));
  if (decl) ids.push({ varName: name, id: decl[1] });
  else if (name === 'panel' || name === 'empty' || name === 'toastEl') {
    // 兜底：变量名与 id 的映射写死在这里，新增元素时上面的正则应当能覆盖
    ids.push({ varName: name, id: name.replace(/El$/, '') });
  }
}

let problems = 0;
for (const { varName, id } of ids) {
  // 该 id 对应的 class 选择器（本项目用 .empty/.panel/.toast 这类 class 承载样式）
  const classGuesses = [id, varName, varName.replace(/El$/, '')];
  const blocks = [];
  for (const cls of new Set(classGuesses)) {
    const re = new RegExp('\\.' + cls + '\\s*\\{([^}]*)\\}', 'g');
    for (const m of css.matchAll(re)) blocks.push({ sel: '.' + cls, body: m[1] });
    const reId = new RegExp('#' + cls + '\\s*\\{([^}]*)\\}', 'g');
    for (const m of css.matchAll(reId)) blocks.push({ sel: '#' + cls, body: m[1] });
  }

  const hasDisplay = blocks.filter((b) => /display\s*:/.test(b.body));
  if (!hasDisplay.length) continue;

  // 有 display 就必须配套一条 [hidden] 规则
  const hiddenRule = new RegExp('\\.' + '(?:' + classGuesses.join('|') + ')\\[hidden\\][^{]*\\{[^}]*display\\s*:\\s*none', );
  if (!hiddenRule.test(css)) {
    console.log('PROBLEM: #' + id + ' 由 JS 的 .hidden 控制，但 CSS 设了 display，且没有对应的 [hidden]{display:none} 规则');
    hasDisplay.forEach((b) => console.log('   ' + b.sel + ' { ' + b.body.trim().replace(/\s+/g, ' ').slice(0, 70) + ' ... }'));
    problems++;
  }
}

// 2) 顺带检查：绝对定位且 inset:0 的覆盖层应当 pointer-events:none
for (const m of css.matchAll(/\.([a-z-]+)\s*\{([^}]*position:\s*absolute[^}]*)\}/g)) {
  const body = m[2];
  if (/inset:\s*0/.test(body) && !/pointer-events:\s*none/.test(body) && !/^\s*(shell|bar|body)/.test(m[1])) {
    console.log('WARN: .' + m[1] + ' 是铺满的绝对定位层，建议 pointer-events:none，否则会拦截列表点击');
  }
}

// 3) Rust 侧用 HitPayload::CopyText 返回的行，前端必须归入"单击即复制"。
//    这类行的语义是"复制就是它的执行动作"；漏接的后果不是报错，而是
//    用户单击没反应、必须双击——静默的交互退化，跨语言改动时最容易漏。
const searchDir = path.join(__dirname, '..', 'crates', 'core', 'src', 'search');
const copyKinds = new Set();
// 只认"构造点"：SearchHit::new(ItemKind::X, ... HitPayload::CopyText { ... }) 里成对出现的 X。
// 早期实现是"文件里出现过 CopyText 就把该文件所有 ItemKind 都算上"，
// 结果 service.rs 的测试里写了一句 `match ... HitPayload::CopyText {`，
// 于是 App/Action/External 全被误判成复制行，守卫报了三个假问题。
// 局限（诚实说明）：这是文本启发式，不是类型系统保证；如果哪天构造写法变得离得很远，
// 这里会漏判 —— 漏判的代价是"新行退化成必须双击"，由下面的 isCopy 断言兜住。
const CTX = /SearchHit::new\(\s*ItemKind::([A-Z]\w+)[\s\S]{0,600}?HitPayload::CopyText\s*\{/g;
for (const f of fs.readdirSync(searchDir)) {
  // mod.rs 里是 HitPayload 枚举自己的定义，不是构造点
  if (!f.endsWith('.rs') || f === 'mod.rs') continue;
  const src = fs.readFileSync(path.join(searchDir, f), 'utf8');
  for (const m of src.matchAll(CTX)) copyKinds.add(m[1].toLowerCase());
}
// 只看那条真正决定"单击是否即复制"的赋值。
// （曾经这里写成"app.js 里出现过 hit.kind === 'time' 就算通过"，
//  结果 tooltip 的三元表达式里也有同样的字面量，注入缺陷时守卫没反应。）
const isCopyDecl = js.match(/(?:var|let|const)\s+isCopy\s*=\s*([^;]+);/);
if (!isCopyDecl) {
  console.log('PROBLEM: app.js 里找不到 isCopy 的赋值（单击即复制的判定被删掉或改名了？）');
  problems++;
} else {
  for (const kind of copyKinds) {
    if (!new RegExp("'" + kind + "'").test(isCopyDecl[1])) {
      console.log("PROBLEM: " + kind + " 行的 payload 是 CopyText，但 isCopy 没把它算进单击即复制");
      console.log('       isCopy = ' + isCopyDecl[1].trim());
      problems++;
    }
  }
}
for (const kind of copyKinds) {
  if (!new RegExp('\\[data-kind="' + kind + '"\\]').test(css)) {
    console.log('WARN: style.css 没有 [data-kind="' + kind + '"] 规则，该行会用默认配色');
  }
}

if (problems) {
  console.log('\n检查未通过：' + problems + ' 处');
  process.exit(1);
}
console.log(
  '前端守卫检查通过（' + ids.length + ' 个 .hidden 控制的元素，' +
  copyKinds.size + ' 类单击即复制的行：' + [...copyKinds].join('/') + '）'
);
