// Markdown 渲染器（纯函数，无 DOM 依赖）：插件 UI 与 Node 测试共用。
// 只覆盖常用子集：标题 / 段落 / 列表 / 任务项 / 引用 / 代码块 / 表格 / 分割线 /
// 链接 / 图片 / 粗体 / 斜体 / 删除线 / 行内代码。
// 安全基线：先转义 HTML 再生成标签，不解析原始 HTML，避免脚本注入。
(function (root) {
  'use strict';

  function esc(s) {
    return String(s).replace(/[&<>"']/g, function (c) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c];
    });
  }

  // 只放行常规链接协议，javascript:/vbscript:/data: 一律按纯文本处理
  function safeUrl(u) {
    var s = String(u).trim();
    return /^(javascript|vbscript|data):/i.test(s) ? '' : s;
  }

  // 行内格式：输入是原始文本，输出是已转义并带标签的 HTML 片段。
  function inline(raw) {
    var codes = [];
    var s = esc(raw);
    // 行内代码先占位，内部不再做强调 / 链接替换
    s = s.replace(/`([^`]+)`/g, function (_, code) {
      codes.push(code);
      return '\u0000' + (codes.length - 1) + '\u0000';
    });
    s = s.replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g, function (_, alt, src) {
      var u = safeUrl(src);
      return u ? '<img src="' + u + '" alt="' + alt + '" loading="lazy">' : alt;
    });
    s = s.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, function (_, text, href) {
      var u = safeUrl(href);
      return u ? '<a href="' + u + '" rel="noreferrer noopener">' + text + '</a>' : text;
    });
    s = s.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
    s = s.replace(/(^|[^*])\*([^*\n]+)\*/g, '$1<em>$2</em>');
    s = s.replace(/~~([^~]+)~~/g, '<del>$1</del>');
    s = s.replace(/\u0000(\d+)\u0000/g, function (_, i) {
      return '<code>' + codes[+i] + '</code>';
    });
    return s;
  }

  function isHr(line) {
    return /^ {0,3}([-*_])( *\1){2,} *$/.test(line);
  }

  function isList(line) {
    return /^ {0,3}([-*+]|\d+\.)\s+/.test(line);
  }

  function isTableSep(line) {
    var t = line.trim();
    if (!t || !/^[|:\- ]+$/.test(t)) return false;
    return /-{3,}/.test(t);
  }

  function isBlockStart(line) {
    return (
      /^ {0,3}```/.test(line) ||
      /^ {0,3}#{1,6}\s+/.test(line) ||
      /^ {0,3}>/.test(line) ||
      isHr(line) ||
      isList(line)
    );
  }

  function splitRow(line) {
    var t = line.trim();
    if (t.charAt(0) === '|') t = t.slice(1);
    if (t.charAt(t.length - 1) === '|') t = t.slice(0, -1);
    return t.split('|').map(function (c) {
      return c.trim();
    });
  }

  function alignOf(cell) {
    var l = cell.charAt(0) === ':';
    var r = cell.charAt(cell.length - 1) === ':';
    if (l && r) return 'center';
    if (r) return 'right';
    if (l) return 'left';
    return '';
  }

  function parseTable(lines, start) {
    var header = splitRow(lines[start]);
    var align = splitRow(lines[start + 1]).map(alignOf);
    var rows = [];
    var i = start + 2;
    while (i < lines.length && lines[i].trim() && lines[i].indexOf('|') >= 0) {
      rows.push(splitRow(lines[i]));
      i++;
    }
    var html = '<table><thead><tr>';
    header.forEach(function (c, idx) {
      html += styleCell('th', align[idx]) + inline(c) + '</th>';
    });
    html += '</tr></thead><tbody>';
    rows.forEach(function (row) {
      html += '<tr>';
      for (var k = 0; k < header.length; k++) {
        html += styleCell('td', align[k]) + inline(row[k] || '') + '</td>';
      }
      html += '</tr>';
    });
    return { html: html + '</tbody></table>', next: i };
  }

  function styleCell(tag, align) {
    return '<' + tag + (align ? ' style="text-align:' + align + '"' : '') + '>';
  }

  function parseList(lines, start) {
    var ordered = /^\d+\.\s+/.test(lines[start].trim());
    var items = [];
    var i = start;
    while (i < lines.length) {
      var m = /^ {0,3}([-*+]|\d+\.)\s+(.*)$/.exec(lines[i]);
      if (!m) break;
      if (/^\d+\.$/.test(m[1]) !== ordered) break;
      var body = m[2];
      var task = /^\[([ xX])\]\s+/.exec(body);
      if (task) {
        var checked = task[1].toLowerCase() === 'x' ? ' checked' : '';
        items.push(
          '<li class="task"><input type="checkbox" disabled' +
            checked +
            '> ' +
            inline(body.slice(task[0].length)) +
            '</li>'
        );
      } else {
        items.push('<li>' + inline(body) + '</li>');
      }
      i++;
    }
    var tag = ordered ? 'ol' : 'ul';
    return { html: '<' + tag + '>' + items.join('') + '</' + tag + '>', next: i };
  }

  function render(src) {
    var lines = String(src == null ? '' : src).replace(/\r\n?/g, '\n').split('\n');
    var out = [];
    var i = 0;
    while (i < lines.length) {
      var line = lines[i];
      if (!line.trim()) {
        i++;
        continue;
      }

      var fence = /^ {0,3}```\s*([^\s`]*)\s*$/.exec(line);
      if (fence) {
        var lang = fence[1];
        var buf = [];
        i++;
        while (i < lines.length && !/^ {0,3}```\s*$/.test(lines[i])) {
          buf.push(lines[i]);
          i++;
        }
        i++; // 跳过结束围栏；缺失时按“延伸到文末”收尾（与常见实现一致）
        var cls = lang ? ' class="language-' + esc(lang) + '"' : '';
        out.push('<pre><code' + cls + '>' + esc(buf.join('\n')) + '</code></pre>');
        continue;
      }

      if (isHr(line)) {
        out.push('<hr>');
        i++;
        continue;
      }

      var h = /^ {0,3}(#{1,6})\s+(.*?)\s*#*\s*$/.exec(line);
      if (h) {
        var lv = h[1].length;
        out.push('<h' + lv + '>' + inline(h[2]) + '</h' + lv + '>');
        i++;
        continue;
      }

      if (/^ {0,3}>/.test(line)) {
        var quote = [];
        while (i < lines.length && /^ {0,3}>/.test(lines[i])) {
          quote.push(lines[i].replace(/^ {0,3}> ?/, ''));
          i++;
        }
        out.push('<blockquote>' + render(quote.join('\n')) + '</blockquote>');
        continue;
      }

      if (
        line.indexOf('|') >= 0 &&
        i + 1 < lines.length &&
        isTableSep(lines[i + 1])
      ) {
        var table = parseTable(lines, i);
        out.push(table.html);
        i = table.next;
        continue;
      }

      if (isList(line)) {
        var list = parseList(lines, i);
        out.push(list.html);
        i = list.next;
        continue;
      }

      var para = [];
      while (i < lines.length && lines[i].trim() && !isBlockStart(lines[i])) {
        para.push(lines[i].trim());
        i++;
      }
      if (!para.length) {
        // 理论到不了这里；兜底消费一行，避免任何情况下死循环
        para.push(lines[i].trim());
        i++;
      }
      out.push('<p>' + inline(para.join('\n')).replace(/\n/g, '<br>') + '</p>');
    }
    return out.join('\n');
  }

  var api = { renderMarkdown: render, escapeHtml: esc };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  root.MarkdownRender = api;
})(typeof window !== 'undefined' ? window : globalThis);
