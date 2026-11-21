// efto 帮助窗口：从宿主导入编译期嵌入的 Markdown，用极简渲染器转成 HTML。
// 不引第三方渲染库：文档由仓库自己维护，只需覆盖标题/段落/列表/表格/代码块/引用。
(function () {
  'use strict';

  var I = window.__TAURI_INTERNALS__;
  var doc = document.getElementById('doc');
  var toc = document.getElementById('toc');
  var filter = document.getElementById('filter');
  var docTitle = document.getElementById('doc-title');
  var closeBtn = document.getElementById('close');
  var sections = [];

  function invoke(cmd, args) {
    if (!I || typeof I.invoke !== 'function') return Promise.reject(new Error('no tauri'));
    return I.invoke(cmd, args || {});
  }

  function esc(s) {
    return String(s)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  // 行内标记：先转义，再只处理我们支持的几种；顺序保证代码块里的符号不被二次解释。
  function inline(s) {
    var out = esc(s);
    out = out.replace(/`([^`]+)`/g, '<code>$1</code>');
    out = out.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
    out = out.replace(
      /\[([^\]]+)\]\((https?:\/\/[^)\s]+|mailto:[^)\s]+)\)/g,
      function (_, text, url) {
        return '<a href="' + url + '">' + text + '</a>';
      }
    );
    return out;
  }

  function splitRow(line) {
    var t = line.trim();
    if (t.charAt(0) === '|') t = t.slice(1);
    if (t.charAt(t.length - 1) === '|') t = t.slice(0, -1);
    return t.split('|').map(function (c) { return c.trim(); });
  }

  function isTableSeparator(line) {
    return /^[\s|:-]+$/.test(line) && line.indexOf('-') >= 0;
  }

  function isSpecial(line) {
    return (
      /^\s*$/.test(line) ||
      /^```/.test(line) ||
      /^#{1,4}\s+/.test(line) ||
      /^\s*(---|\*\*\*)\s*$/.test(line) ||
      /^>\s?/.test(line) ||
      /^\s*[-*]\s+/.test(line) ||
      /^\s*\d+\.\s+/.test(line)
    );
  }

  function render(md) {
    var lines = String(md).replace(/\r\n/g, '\n').split('\n');
    var out = [];
    var used = {};
    var i = 0;

    function headingId(text) {
      var base = text
        .toLowerCase()
        .replace(/[^\w\u4e00-\u9fa5]+/g, '-')
        .replace(/^-+|-+$/g, '') || 'section';
      var n = used[base] || 0;
      used[base] = n + 1;
      return n ? base + '-' + n : base;
    }

    while (i < lines.length) {
      var line = lines[i];

      if (/^\s*$/.test(line)) {
        i++;
        continue;
      }

      if (/^```/.test(line)) {
        var code = [];
        i++;
        while (i < lines.length && !/^```/.test(lines[i])) {
          code.push(lines[i]);
          i++;
        }
        if (i < lines.length) i++;
        out.push('<pre><code>' + esc(code.join('\n')) + '</code></pre>');
        continue;
      }

      var h = /^(#{1,4})\s+(.*)$/.exec(line);
      if (h) {
        var level = h[1].length;
        var text = h[2].trim();
        var id = headingId(text);
        out.push('<h' + level + ' id="' + id + '">' + inline(text) + '</h' + level + '>');
        i++;
        continue;
      }

      if (/^\s*(---|\*\*\*)\s*$/.test(line)) {
        out.push('<hr />');
        i++;
        continue;
      }

      if (/^>\s?/.test(line)) {
        var quote = [];
        while (i < lines.length && /^>\s?/.test(lines[i])) {
          quote.push(lines[i].replace(/^>\s?/, ''));
          i++;
        }
        out.push('<blockquote>' + inline(quote.join(' ')) + '</blockquote>');
        continue;
      }

      if (
        line.indexOf('|') >= 0 &&
        i + 1 < lines.length &&
        isTableSeparator(lines[i + 1])
      ) {
        var head = splitRow(line);
        i += 2;
        var rows = [];
        while (i < lines.length && lines[i].indexOf('|') >= 0 && !/^\s*$/.test(lines[i])) {
          rows.push(splitRow(lines[i]));
          i++;
        }
        out.push('<div class="table-wrap"><table><thead><tr>');
        head.forEach(function (cell) {
          out.push('<th>' + inline(cell) + '</th>');
        });
        out.push('</tr></thead><tbody>');
        rows.forEach(function (row) {
          out.push('<tr>');
          head.forEach(function (_, k) {
            out.push('<td>' + inline(row[k] || '') + '</td>');
          });
          out.push('</tr>');
        });
        out.push('</tbody></table></div>');
        continue;
      }

      if (/^\s*[-*]\s+/.test(line)) {
        var items = [];
        while (i < lines.length && /^\s*[-*]\s+/.test(lines[i])) {
          var m = /^(\s*)[-*]\s+(.*)$/.exec(lines[i]);
          items.push({ sub: m[1].length >= 2, text: m[2] });
          i++;
        }
        out.push('<ul>');
        items.forEach(function (it) {
          out.push('<li' + (it.sub ? ' class="sub"' : '') + '>' + inline(it.text) + '</li>');
        });
        out.push('</ul>');
        continue;
      }

      if (/^\s*\d+\.\s+/.test(line)) {
        var ol = [];
        while (i < lines.length && /^\s*\d+\.\s+/.test(lines[i])) {
          ol.push(lines[i].replace(/^\s*\d+\.\s+/, ''));
          i++;
        }
        out.push('<ol>');
        ol.forEach(function (t) {
          out.push('<li>' + inline(t) + '</li>');
        });
        out.push('</ol>');
        continue;
      }

      var para = [line];
      i++;
      while (i < lines.length && !isSpecial(lines[i]) && lines[i].indexOf('|') < 0) {
        para.push(lines[i]);
        i++;
      }
      out.push('<p>' + inline(para.join(' ')) + '</p>');
    }

    doc.innerHTML = out.join('\n');
    buildToc();
    applyFilter('');
    filter.value = '';
  }

  function buildToc() {
    toc.textContent = '';
    sections = [];
    var children = Array.prototype.slice.call(doc.children);
    var current = null;
    children.forEach(function (el) {
      if (el.tagName === 'H2') {
        current = { el: el, text: el.textContent, els: [el] };
        sections.push(current);
      } else if (current) {
        current.els.push(el);
      }
    });
    sections.forEach(function (s) {
      var a = document.createElement('a');
      a.href = '#' + s.el.id;
      a.textContent = s.text;
      a.addEventListener('click', function (e) {
        e.preventDefault();
        s.el.scrollIntoView({ block: 'start' });
      });
      toc.appendChild(a);
    });
  }

  function applyFilter(q) {
    q = String(q || '').trim().toLowerCase();
    sections.forEach(function (s) {
      var on = !q || s.text.toLowerCase().indexOf(q) >= 0;
      s.els.forEach(function (el) { el.hidden = !on; });
    });
    Array.prototype.forEach.call(toc.children, function (a) {
      a.hidden = !!q && a.textContent.toLowerCase().indexOf(q) < 0;
    });
  }

  function load() {
    return invoke('help_document')
      .then(function (data) {
        var title = data && data.title ? data.title : 'efto';
        docTitle.textContent = title;
        document.title = title;
        var en = data && data.language === 'en-US';
        filter.placeholder = en ? 'Filter chapters…' : '按章节标题筛选…';
        closeBtn.textContent = en ? 'Close' : '关闭';
        render(data && data.markdown ? data.markdown : '');
      })
      .catch(function (e) {
        doc.innerHTML = '<p>帮助文档载入失败：' + esc(e) + '</p>';
      });
  }

  filter.addEventListener('input', function () {
    applyFilter(filter.value);
  });

  closeBtn.addEventListener('click', function () {
    invoke('hide_help').catch(function () {});
  });

  doc.addEventListener('click', function (e) {
    var a = e.target.closest ? e.target.closest('a[href]') : null;
    if (!a) return;
    e.preventDefault();
    invoke('open_url', { url: a.getAttribute('href') }).catch(function () {});
  });

  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape') {
      e.preventDefault();
      invoke('hide_help').catch(function () {});
    } else if ((e.ctrlKey || e.metaKey) && e.key === 'f') {
      e.preventDefault();
      filter.focus();
      filter.select();
    }
  });

  // 托盘每次打开都会广播：语言可能刚改过，重新拉一遍文档
  if (I && typeof I.transformCallback === 'function') {
    try {
      var cb = I.transformCallback(function () { load(); }, false);
      I.invoke('plugin:event|listen', {
        event: 'help:show',
        target: { kind: 'Any' },
        handler: cb
      }).catch(function () {});
    } catch (e) {
      // 非 Tauri 环境下忽略
    }
  }

  load();
})();
