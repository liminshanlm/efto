(function (root) {
  'use strict';

  var DEFAULT_INDENT = '  ';
  var DEFAULT_NEWLINE = '\n';

  /* ---------------------------------------------------------------- 基础工具 */

  function isWsChar(c) {
    return c === ' ' || c === '\t' || c === '\n' || c === '\r';
  }

  function isBlank(s) {
    for (var i = 0; i < s.length; i++) {
      if (!isWsChar(s.charAt(i))) return false;
    }
    return true;
  }

  function repeat(str, times) {
    var out = '';
    for (var i = 0; i < times; i++) out += str;
    return out;
  }

  /* 位置 -> 行列 */
  function makeLineIndex(text) {
    var starts = [0];
    for (var i = 0; i < text.length; i++) {
      if (text.charCodeAt(i) === 10) starts.push(i + 1);
    }
    return function locate(pos) {
      if (typeof pos !== 'number' || pos < 0) pos = 0;
      if (pos > text.length) pos = text.length;
      var lo = 0;
      var hi = starts.length - 1;
      while (lo < hi) {
        var mid = (lo + hi + 1) >> 1;
        if (starts[mid] <= pos) lo = mid;
        else hi = mid - 1;
      }
      return { line: lo + 1, column: pos - starts[lo] + 1 };
    };
  }

  /* 内部错误对象：统一带行号的中文说明 */
  function fault(msg, pos, locate) {
    var at = locate(pos);
    return {
      __tbFault: true,
      error: '第 ' + at.line + ' 行：' + msg,
      line: at.line,
      column: at.column
    };
  }

  function normOpts(opts) {
    var o = opts && typeof opts === 'object' ? opts : {};
    var indent = typeof o.indent === 'string' ? o.indent : DEFAULT_INDENT;
    var attrWrap = 0;
    if (typeof o.attrWrap === 'number' && isFinite(o.attrWrap) && o.attrWrap > 0) {
      attrWrap = Math.floor(o.attrWrap);
    }
    var newline = typeof o.newline === 'string' && o.newline.length > 0 ? o.newline : DEFAULT_NEWLINE;
    return { indent: indent, attrWrap: attrWrap, newline: newline };
  }

  /* ---------------------------------------------------------------- 词法分析 */

  /* 判断 pos 处是否是一个标记（tag/注释/指令…）的起始 */
  function markupStart(s, j) {
    if (s.charAt(j) !== '<') return false;
    var c = s.charAt(j + 1);
    if (c === '' || c === '<' || c === '>' || isWsChar(c)) return false;
    if (c === '!' || c === '?') return true;
    if (c === '/') {
      var d = s.charAt(j + 2);
      return d !== '' && !isWsChar(d) && d !== '>' && d !== '<';
    }
    return true;
  }

  /* 从 '<' 开始找到配对 '>'（跳过引号内的内容），返回 '>' 的下标或 -1 */
  function scanTagEnd(s, start) {
    var quote = '';
    for (var i = start + 1; i < s.length; i++) {
      var c = s.charAt(i);
      if (quote) {
        if (c === quote) quote = '';
        continue;
      }
      if (c === '"' || c === "'") {
        quote = c;
        continue;
      }
      if (c === '>') return i;
    }
    return -1;
  }

  /* DOCTYPE 扫描：内部子集 [ ... ] 内的 '>' 不算结束 */
  function scanDoctype(s, start) {
    var i = start + 9;
    var depth = 0;
    var quote = '';
    while (i < s.length) {
      var c = s.charAt(i);
      if (quote) {
        if (c === quote) quote = '';
        i++;
        continue;
      }
      if (c === '"' || c === "'") {
        quote = c;
        i++;
        continue;
      }
      if (c === '[') {
        depth++;
      } else if (c === ']') {
        if (depth > 0) depth--;
      } else if (c === '>' && depth === 0) {
        return i + 1;
      }
      i++;
    }
    return -1;
  }

  function parseOpenTag(raw, pos, locate) {
    var inner = raw.slice(1, raw.length - 1);
    var body = inner.replace(/[ \t\r\n]+$/, '');
    var selfClosing = false;
    if (body.charAt(body.length - 1) === '/') {
      selfClosing = true;
      body = body.slice(0, -1);
    }
    var nm = /^([^\s\/>]+)/.exec(body);
    if (!nm) throw fault('开始标签缺少元素名', pos, locate);
    var name = nm[1];
    var rest = body.slice(name.length);
    var attrs = [];
    var i = 0;
    while (i < rest.length) {
      while (i < rest.length && isWsChar(rest.charAt(i))) i++;
      if (i >= rest.length) break;
      var nameStart = i;
      while (i < rest.length) {
        var c = rest.charAt(i);
        if (isWsChar(c) || c === '=') break;
        i++;
      }
      var attrName = rest.slice(nameStart, i);
      if (!attrName) throw fault('标签 <' + name + '> 的属性名缺失或非法', pos, locate);
      while (i < rest.length && isWsChar(rest.charAt(i))) i++;
      var value = null;
      if (rest.charAt(i) === '=') {
        i++;
        while (i < rest.length && isWsChar(rest.charAt(i))) i++;
        var q = rest.charAt(i);
        if (q === '"' || q === "'") {
          var close = rest.indexOf(q, i + 1);
          if (close < 0) throw fault('标签 <' + name + '> 的属性值引号没有配对', pos, locate);
          value = rest.slice(i, close + 1);
          i = close + 1;
        } else {
          var vs = i;
          while (i < rest.length && !isWsChar(rest.charAt(i))) i++;
          if (i === vs) throw fault('标签 <' + name + '> 的属性 ' + attrName + ' 缺少值', pos, locate);
          value = rest.slice(vs, i);
        }
      }
      attrs.push({ name: attrName, value: value });
    }
    return { type: selfClosing ? 'self' : 'open', name: name, attrs: attrs, pos: pos };
  }

  function pushText(toks, raw, pos) {
    var last = toks.length ? toks[toks.length - 1] : null;
    if (last && last.type === 'text' && last.pos + last.raw.length === pos) {
      last.raw += raw;
      return;
    }
    toks.push({ type: 'text', raw: raw, pos: pos });
  }

  function tokenize(text, locate) {
    var toks = [];
    var i = 0;
    var n = text.length;
    while (i < n) {
      var start = i;
      if (markupStart(text, i)) {
        var two = text.substr(i, 2);

        if (text.substr(i, 4) === '<!--') {
          var cend = text.indexOf('-->', i + 4);
          if (cend < 0) throw fault('注释没有正确闭合（缺少 -->）', i, locate);
          toks.push({ type: 'comment', raw: text.slice(i, cend + 3), pos: i });
          i = cend + 3;
          continue;
        }

        if (text.substr(i, 9) === '<![CDATA[') {
          var dend = text.indexOf(']]>', i + 9);
          if (dend < 0) throw fault('CDATA 段没有正确闭合（缺少 ]]>）', i, locate);
          toks.push({ type: 'cdata', raw: text.slice(i, dend + 3), pos: i });
          i = dend + 3;
          continue;
        }

        if (text.substr(i, 9).toUpperCase() === '<!DOCTYPE') {
          var dtEnd = scanDoctype(text, i);
          if (dtEnd < 0) throw fault('DOCTYPE 声明没有正确闭合（缺少 > 或内部子集缺少 ]）', i, locate);
          toks.push({ type: 'doctype', raw: text.slice(i, dtEnd), pos: i });
          i = dtEnd;
          continue;
        }

        if (two === '<?') {
          var piEnd = text.indexOf('?>', i + 2);
          if (piEnd < 0) throw fault('处理指令没有正确闭合（缺少 ?>）', i, locate);
          toks.push({ type: 'pi', raw: text.slice(i, piEnd + 2), pos: i });
          i = piEnd + 2;
          continue;
        }

        if (two === '</') {
          var closeEnd = scanTagEnd(text, i);
          if (closeEnd < 0) throw fault('结束标签没有正确闭合（缺少 >）', i, locate);
          var closeRaw = text.slice(i, closeEnd + 1);
          var cm = /^<\/\s*([^\s>\/]+)\s*>$/.exec(closeRaw);
          if (!cm) throw fault('结束标签格式非法', i, locate);
          toks.push({ type: 'close', name: cm[1], raw: closeRaw, pos: i });
          i = closeEnd + 1;
          continue;
        }

        if (text.charAt(i + 1) === '!') {
          var mdEnd = scanTagEnd(text, i);
          if (mdEnd < 0) throw fault('标记声明没有正确闭合（缺少 >）', i, locate);
          toks.push({ type: 'raw', raw: text.slice(i, mdEnd + 1), pos: i });
          i = mdEnd + 1;
          continue;
        }

        var tagEnd = scanTagEnd(text, i);
        if (tagEnd < 0) throw fault('开始标签没有正确闭合（缺少 > 或属性值引号未配对）', i, locate);
        toks.push(parseOpenTag(text.slice(i, tagEnd + 1), i, locate));
        i = tagEnd + 1;
        continue;
      }

      /* 文本节点：直到下一个标记开头 */
      var j = i;
      while (j < n && !markupStart(text, j)) j++;
      if (j === start) j = start + 1; /* 理论上不会发生，避免死循环 */
      pushText(toks, text.slice(start, j), start);
      i = j;
    }
    return toks;
  }

  /* ---------------------------------------------------------------- 语法树 */

  function buildTree(toks, locate) {
    var doc = { type: 'document', children: [], pos: 0 };
    var stack = [doc];
    var rootSeen = false;

    for (var k = 0; k < toks.length; k++) {
      var t = toks[k];
      var parent = stack[stack.length - 1];

      if (t.type === 'text') {
        if (parent === doc) {
          if (isBlank(t.raw)) continue;
          throw fault(
            rootSeen ? '根元素结束之后不允许再出现文本内容' : '根元素之外不允许出现文本内容',
            t.pos + (t.raw.length - t.raw.replace(/^\s+/, '').length),
            locate
          );
        }
        parent.children.push({ type: 'text', value: t.raw, pos: t.pos });
        continue;
      }

      if (t.type === 'open' || t.type === 'self') {
        if (parent === doc) {
          if (rootSeen) throw fault('一个 XML 文档只能有一个根元素', t.pos, locate);
          rootSeen = true;
        }
        var el = {
          type: 'element',
          name: t.name,
          attrs: t.attrs,
          selfClosing: t.type === 'self',
          children: [],
          pos: t.pos
        };
        parent.children.push(el);
        if (t.type === 'open') stack.push(el);
        continue;
      }

      if (t.type === 'close') {
        if (stack.length === 1) {
          throw fault('结束标签 </' + t.name + '> 没有与之匹配的开始标签', t.pos, locate);
        }
        var top = stack[stack.length - 1];
        if (top.name !== t.name) {
          throw fault(
            '结束标签 </' + t.name + '> 与开始标签 <' + top.name + '> 不匹配',
            t.pos,
            locate
          );
        }
        stack.pop();
        continue;
      }

      /* comment / cdata / pi / doctype / raw：原样保留 */
      parent.children.push({ type: t.type, raw: t.raw, pos: t.pos });
    }

    if (stack.length > 1) {
      var unclosed = stack[stack.length - 1];
      throw fault('标签 <' + unclosed.name + '> 没有闭合（文档在结尾处结束）', unclosed.pos, locate);
    }

    return doc;
  }

  /* ---------------------------------------------------------------- 输出 */

  function attrText(a) {
    return a.value === null || a.value === undefined ? a.name : a.name + '=' + a.value;
  }

  function flatOpen(el) {
    var out = '<' + el.name;
    for (var i = 0; i < el.attrs.length; i++) out += ' ' + attrText(el.attrs[i]);
    return out + (el.selfClosing ? '/>' : '>');
  }

  function openTag(el, depth, opts) {
    var flat = flatOpen(el);
    if (opts.attrWrap > 0 && flat.length > opts.attrWrap && el.attrs.length > 0) {
      var inner = repeat(opts.indent, depth + 1);
      var parts = ['<' + el.name];
      for (var i = 0; i < el.attrs.length; i++) {
        parts.push(inner + attrText(el.attrs[i]));
      }
      parts.push(repeat(opts.indent, depth) + (el.selfClosing ? '/>' : '>'));
      return parts.join(opts.newline);
    }
    return flat;
  }

  function meaningfulChildren(el) {
    var out = [];
    for (var i = 0; i < el.children.length; i++) {
      var c = el.children[i];
      if (c.type === 'text' && isBlank(c.value)) continue;
      out.push(c);
    }
    return out;
  }

  function renderElement(el, depth, opts, lines) {
    var pad = repeat(opts.indent, depth);

    if (el.selfClosing) {
      lines.push(pad + openTag(el, depth, opts));
      return;
    }

    var kids = meaningfulChildren(el);

    if (kids.length === 0) {
      lines.push(pad + openTag(el, depth, opts) + '</' + el.name + '>');
      return;
    }

    /* 只有一段纯文本：合并成一行（内联结构不参与属性换行） */
    if (kids.length === 1 && kids[0].type === 'text') {
      lines.push(pad + flatOpen(el) + kids[0].value.replace(/^[ \t\r\n]+/, '').replace(/[ \t\r\n]+$/, '') + '</' + el.name + '>');
      return;
    }

    lines.push(pad + openTag(el, depth, opts));
    renderChildren(el.children, depth + 1, opts, lines);
    lines.push(pad + '</' + el.name + '>');
  }

  function renderChildren(children, depth, opts, lines) {
    var pad = repeat(opts.indent, depth);
    for (var i = 0; i < children.length; i++) {
      var c = children[i];
      if (c.type === 'element') {
        renderElement(c, depth, opts, lines);
        continue;
      }
      if (c.type === 'text') {
        var v = c.value.replace(/^[ \t\r\n]+/, '').replace(/[ \t\r\n]+$/, '');
        if (v === '') continue;
        lines.push(pad + v);
        continue;
      }
      /* comment / cdata / pi / doctype / raw：内容原样保留 */
      lines.push(pad + c.raw);
    }
  }

  function render(doc, opts) {
    var lines = [];
    renderChildren(doc.children, 0, opts, lines);
    if (lines.length === 0) return '';
    return lines.join(opts.newline) + opts.newline;
  }

  /* ---------------------------------------------------------------- 入口 */

  function format(text, opts) {
    try {
      var src = text === null || text === undefined ? '' : String(text);
      /* 输入若带 BOM，用等长空格替换，避免被当作根元素之外的文本 */
      if (src.charCodeAt(0) === 0xFEFF) src = ' ' + src.slice(1);
      var options = normOpts(opts);
      var locate = makeLineIndex(src);
      var toks = tokenize(src, locate);
      var doc = buildTree(toks, locate);
      return { ok: true, text: render(doc, options) };
    } catch (e) {
      if (e && e.__tbFault) {
        return { ok: false, error: e.error, line: e.line, column: e.column };
      }
      var msg = e && e.message ? String(e.message) : String(e);
      return { ok: false, error: '无法完成格式化（内部错误）：' + msg };
    }
  }

  var api = { format: format };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  root.TBFormat = root.TBFormat || {};
  root.TBFormat.xml = api;
})(typeof globalThis !== 'undefined' ? globalThis : this);
