'use strict';

const assert = require('assert');
const md = require('../render.js');

let pass = 0;
function ok(name, cond, extra) {
  if (!cond) {
    console.error('FAIL: ' + name + (extra ? '\n' + extra : ''));
    process.exitCode = 1;
  } else {
    pass++;
    console.log('ok   ' + name);
  }
}

function render(src) {
  return md.renderMarkdown(src);
}

// 1. 原始 HTML 一律转义，不生成可执行标签
let html = render('<script>alert(1)</script>');
ok('HTML 被转义', html.includes('&lt;script&gt;') && !html.includes('<script>'), html);

// 1b. 危险协议不出链接标签
html = render('[x](javascript:alert(1)) ![y](data:text/html;base64,xx)');
ok('javascript/data 链接降级为文本', !html.includes('<a') && !html.includes('<img'), html);

// 2. 标题
html = render('# 标题\n\n### 三级');
ok('h1', html.includes('<h1>标题</h1>'), html);
ok('h3', html.includes('<h3>三级</h3>'), html);

// 3. 段落软换行变 <br>
html = render('第一行\n第二行');
ok('段落内软换行', html.includes('第一行<br>第二行'), html);

// 4. 行内格式
html = render('**粗** *斜* ~~删~~ `code *x*`');
ok('粗体', html.includes('<strong>粗</strong>'), html);
ok('斜体', html.includes('<em>斜</em>'), html);
ok('删除线', html.includes('<del>删</del>'), html);
ok('行内代码不被强调', html.includes('<code>code *x*</code>'), html);

// 5. 列表与任务项
html = render('- a\n- b\n\n1. x\n2. y\n\n- [x] done\n- [ ] todo');
ok('无序列表', html.includes('<ul><li>a</li><li>b</li></ul>'), html);
ok('有序列表', html.includes('<ol><li>x</li><li>y</li></ol>'), html);
ok('任务项勾选', html.includes('disabled checked'), html);
ok('任务项未勾选', html.includes('disabled> todo'), html);

// 6. 引用与代码块
html = render('> 引用 **加粗**\n\n```js\nvar a = 1 < 2;\n```');
ok('引用内渲染', html.includes('<blockquote><p>引用 <strong>加粗</strong></p></blockquote>'), html);
ok('代码块语言类名', html.includes('<pre><code class="language-js">'), html);
ok('代码块内容转义', html.includes('var a = 1 &lt; 2;'), html);

// 7. 未闭合围栏延伸到文末（与常见实现一致），且不能死循环
html = render('```\ncode\n\n# 标题');
ok('未闭合围栏延伸到文末', html === '<pre><code>code\n\n# 标题</code></pre>', html);

// 8. 表格与对齐
html = render('| a | b |\n| :--- | ---: |\n| 1 | 2 |');
ok('表格结构', html.includes('<table><thead><tr><th style="text-align:left">a</th>'), html);
ok('表格右对齐', html.includes('<th style="text-align:right">b</th>'), html);
ok('表格数据行', html.includes('<td style="text-align:left">1</td>'), html);

// 9. 分割线、链接、图片
html = render('---\n\n[efto](https://example.com) ![图](https://example.com/a.png)');
ok('分割线', html.includes('<hr>'), html);
ok('链接', html.includes('<a href="https://example.com" rel="noreferrer noopener">efto</a>'), html);
ok('图片', html.includes('<img src="https://example.com/a.png" alt="图"'), html);

// 10. 空输入不报错
ok('空输入返回空串', render('') === '' && render(null) === '');

console.log('\n' + pass + ' 项通过');
if (process.exitCode) console.error('有用例失败');
