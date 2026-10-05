// Small helpers shared by the UI modules.

// Byte units: "decimal" (1 KB = 1000 B) or "binary" (1 KiB = 1024 B).
// Set once from saved settings; every fmtBytes call then follows it.
let unitMode = "decimal";
export function setUnits(mode) {
  unitMode = mode === "binary" ? "binary" : "decimal";
}

/** Format a byte count in the configured unit system. */
export function fmtBytes(n) {
  const binary = unitMode === "binary";
  const step = binary ? 1024 : 1000;
  const units = binary
    ? ["bytes", "KiB", "MiB", "GiB", "TiB", "PiB"]
    : ["bytes", "KB", "MB", "GB", "TB", "PB"];
  let v = n;
  let i = 0;
  while (v >= step && i < units.length - 1) {
    v /= step;
    i++;
  }
  if (i === 0) return `${n} bytes`;
  return `${v >= 100 ? v.toFixed(0) : v >= 10 ? v.toFixed(1) : v.toFixed(2)} ${units[i]}`;
}

export function fmtCount(n) {
  return Number(n).toLocaleString("en-US");
}

export function plural(n, word) {
  return `${fmtCount(n)} ${word}${n === 1 ? "" : "s"}`;
}

export function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

/** A URL is safe to put in an href only if it uses a known-harmless scheme. */
export function safeUrl(url) {
  const u = String(url).trim();
  // Relative or anchor links are fine; absolute ones must use an allowed scheme.
  if (/^(https?:|mailto:|tel:)/i.test(u)) return u;
  if (/^[a-z][a-z0-9+.-]*:/i.test(u)) return null; // some other scheme: reject
  if (/^(\/|#|\.|\?)/.test(u)) return u; // relative path or fragment
  return null;
}

// Tags and attributes the advisor may emit as raw HTML/SVG. Everything else is dropped.
const HTML_TAGS = new Set([
  "p", "br", "hr", "span", "div", "b", "strong", "i", "em", "u", "s", "code", "pre", "kbd",
  "blockquote", "ul", "ol", "li", "h1", "h2", "h3", "h4", "h5", "h6", "a",
  "table", "thead", "tbody", "tr", "th", "td", "caption", "sub", "sup", "small", "mark",
]);
const SVG_TAGS = new Set([
  "svg", "g", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon",
  "text", "tspan", "defs", "lineargradient", "radialgradient", "stop", "title",
  "clippath", "use", "symbol", "marker",
]);
// Attributes allowed on any tag (presentation and SVG geometry). No event handlers, ever.
const ATTRS = new Set([
  "class", "title", "viewbox", "width", "height", "x", "y", "x1", "y1", "x2", "y2",
  "cx", "cy", "r", "rx", "ry", "d", "points", "fill", "stroke", "stroke-width",
  "stroke-linecap", "stroke-linejoin", "stroke-dasharray", "opacity", "fill-opacity",
  "stroke-opacity", "transform", "text-anchor", "font-size", "font-family", "font-weight",
  "dominant-baseline", "offset", "stop-color", "stop-opacity", "gradientunits",
  "gradienttransform", "clip-path", "colspan", "rowspan", "align", "dir", "lang", "id",
]);
// style is not allowed (CSS can smuggle url()/expression()); href only on <a>/<use> with a safe URL.

/**
 * Sanitize an untrusted HTML/SVG fragment to a strict allow-list. Parses tags
 * with a tokenizer (no innerHTML, works under node), keeps only known-safe tags
 * and attributes, drops event handlers, style, and unsafe URLs, and strips the
 * whole subtree of dangerous elements like <script> and <foreignObject>.
 */
export function sanitizeHtml(src) {
  const out = [];
  const stack = [];
  let skipDepth = 0; // inside a dropped-subtree element (e.g. <script>, <foreignObject>)
  let i = 0;
  const re = /<\/?([a-zA-Z][a-zA-Z0-9]*)((?:[^<>"']|"[^"]*"|'[^']*')*?)\/?>/g;
  let last = 0;
  let m;
  const DROP_SUBTREE = new Set(["script", "style", "foreignobject", "iframe", "object", "embed", "template", "noscript"]);
  while ((m = re.exec(src))) {
    const text = src.slice(last, m.index);
    if (!skipDepth && text) out.push(escapeHtml(text));
    last = re.lastIndex;
    const closing = m[0][1] === "/";
    const tag = m[1].toLowerCase();
    const selfClose = /\/>\s*$/.test(m[0]);
    const known = HTML_TAGS.has(tag) || SVG_TAGS.has(tag);

    if (closing) {
      if (skipDepth) {
        if (DROP_SUBTREE.has(tag)) skipDepth--;
        continue;
      }
      const idx = stack.lastIndexOf(tag);
      if (idx !== -1) {
        while (stack.length > idx) out.push(`</${stack.pop()}>`);
      }
      continue;
    }
    if (skipDepth) {
      if (DROP_SUBTREE.has(tag) && !selfClose) skipDepth++;
      continue;
    }
    if (DROP_SUBTREE.has(tag)) {
      if (!selfClose) skipDepth++;
      continue;
    }
    if (!known) continue; // unknown tag: drop the tag, keep its children/text

    const attrs = sanitizeAttrs(tag, m[2]);
    const voidEl = tag === "br" || tag === "hr" || selfClose;
    out.push(`<${tag}${attrs}${voidEl ? " />" : ">"}`);
    if (!voidEl) stack.push(tag);
  }
  const tail = src.slice(last);
  if (!skipDepth && tail) out.push(escapeHtml(tail));
  while (stack.length) out.push(`</${stack.pop()}>`);
  return out.join("");
}

function sanitizeAttrs(tag, raw) {
  const out = [];
  const re = /([a-zA-Z_:][-a-zA-Z0-9_:.]*)\s*=\s*("([^"]*)"|'([^']*)'|([^\s"'>]+))/g;
  let m;
  while ((m = re.exec(raw))) {
    const name = m[1].toLowerCase();
    let value = m[3] ?? m[4] ?? m[5] ?? "";
    if (name.startsWith("on")) continue; // event handler: never
    if (name === "href" || name === "xlink:href" || name === "src") {
      if (tag !== "a" && tag !== "use") continue;
      const safe = safeUrl(value);
      if (!safe) continue;
      value = safe;
      out.push(` href="${escapeHtml(value)}"`);
      if (tag === "a") out.push(' rel="noopener noreferrer" target="_blank"');
      continue;
    }
    if (!ATTRS.has(name)) continue;
    out.push(` ${name}="${escapeHtml(value)}"`);
  }
  return out.join("");
}

// Very small, escape-first syntax highlighter. Works on already-escaped text and
// only ever wraps spans around tokens, so it cannot introduce markup of its own.
const KEYWORDS = {
  js: "const let var function return if else for while do switch case break continue new class extends super import export from default async await yield typeof instanceof in of this null true false undefined try catch finally throw delete void",
  ts: "const let var function return if else for while do switch case break continue new class extends super import export from default async await yield typeof instanceof in of this null true false undefined interface type enum public private protected readonly as implements try catch finally throw",
  rust: "fn let mut const static struct enum impl trait pub use mod match if else for while loop return break continue self Self as where ref move dyn async await unsafe crate super in",
  py: "def class return if elif else for while import from as pass break continue with try except finally raise lambda yield global nonlocal None True False and or not in is async await",
  sh: "if then elif else fi for while do done case esac function return export local echo cd exit set unset",
};
export function highlight(code, lang) {
  const kw = KEYWORDS[lang] || KEYWORDS[{ javascript: "js", typescript: "ts", bash: "sh", shell: "sh", python: "py" }[lang]] || null;
  // Escape first; all further work is on safe text, wrapping <span> around tokens.
  let s = escapeHtml(code);
  // Strings, comments and numbers via a single tokenizer so matches don't overlap.
  s = s.replace(
    /("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`)|(\/\/[^\n]*|#[^\n]*|\/\*[\s\S]*?\*\/)|(\b\d[\d_.]*\b)/g,
    (full, str, comment, num) => {
      if (str) return `<span class="tok-str">${str}</span>`;
      if (comment) return `<span class="tok-com">${comment}</span>`;
      if (num) return `<span class="tok-num">${num}</span>`;
      return full;
    },
  );
  if (kw) {
    const words = kw.split(" ").filter(Boolean).join("|");
    // Only match identifiers not already inside a span tag we just added.
    s = s.replace(new RegExp(`\\b(${words})\\b`, "g"), (w, _1, offset, whole) => {
      const before = whole.slice(0, offset);
      const opens = (before.match(/<span/g) || []).length;
      const closes = (before.match(/<\/span>/g) || []).length;
      return opens > closes ? w : `<span class="tok-kw">${w}</span>`;
    });
  }
  return s;
}

/** Markdown to HTML: headings, lists, code fences (highlighted), tables, links,
 *  bold/italic/inline-code, plus a sanitized allow-list for raw HTML/SVG.
 *  Plain text is always escaped first; only vetted structure becomes markup. */
export function markdown(text) {
  const inline = (s) =>
    escapeHtml(s)
      .replace(/\[([^\]]+)\]\(([^)]+)\)/g, (full, label, url) => {
        const safe = safeUrl(url);
        const text = escapeHtml(label);
        return safe ? `<a href="${escapeHtml(safe)}" rel="noopener noreferrer" target="_blank">${text}</a>` : text;
      })
      .replace(/`([^`]+)`/g, "<code>$1</code>")
      .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
      .replace(/(^|[\s(])\*([^*\s][^*]*)\*/g, "$1<em>$2</em>");
  const splitRow = (line) =>
    line
      .replace(/^\s*\|/, "")
      .replace(/\|\s*$/, "")
      .split("|")
      .map((c) => c.trim());
  const out = [];
  let code = null;
  let codeLang = "";
  let list = null;
  const closeList = () => {
    if (list) out.push(`</${list}>`);
    list = null;
  };
  const lines = text.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    const line = raw.trim();
    if (line.startsWith("```")) {
      if (code) {
        out.push(`<pre class="hl"${codeLang ? ` data-lang="${escapeHtml(codeLang)}"` : ""}>${highlight(code.join("\n"), codeLang)}</pre>`);
        code = null;
        codeLang = "";
      } else {
        closeList();
        code = [];
        codeLang = line.slice(3).trim().toLowerCase();
      }
      continue;
    }
    if (code) {
      code.push(raw);
      continue;
    }
    if (!line) {
      closeList();
      continue;
    }
    // Table: a header row followed by a |---|---| separator.
    if (line.includes("|") && i + 1 < lines.length && /^\s*\|?[\s:|-]*-[\s:|-]*\|?\s*$/.test(lines[i + 1]) && lines[i + 1].includes("-")) {
      closeList();
      const head = splitRow(line);
      out.push("<table><thead><tr>");
      for (const c of head) out.push(`<th>${inline(c)}</th>`);
      out.push("</tr></thead><tbody>");
      i++; // skip the separator
      while (i + 1 < lines.length && lines[i + 1].includes("|") && lines[i + 1].trim()) {
        i++;
        out.push("<tr>");
        for (const c of splitRow(lines[i].trim())) out.push(`<td>${inline(c)}</td>`);
        out.push("</tr>");
      }
      out.push("</tbody></table>");
      continue;
    }
    // Raw HTML/SVG block: a line that opens with a tag. Sanitized, not escaped.
    if (/^<[a-zA-Z]/.test(line)) {
      closeList();
      const block = [raw];
      // Gather a multi-line element (e.g. an <svg>…</svg>) until tags balance out.
      const opensSvg = /^<svg\b/i.test(line);
      if (opensSvg && !/<\/svg>/i.test(line)) {
        while (i + 1 < lines.length && !/<\/svg>/i.test(lines[i])) {
          i++;
          block.push(lines[i]);
        }
      }
      out.push(sanitizeHtml(block.join("\n")));
      continue;
    }
    let m;
    if ((m = line.match(/^(#{1,6})\s*(.*)$/))) {
      closeList();
      const level = Math.min(4, m[1].length + 1);
      out.push(`<h${level}>${inline(m[2])}</h${level}>`);
    } else if ((m = line.match(/^[-*]\s+(.*)$/))) {
      if (list !== "ul") {
        closeList();
        out.push("<ul>");
        list = "ul";
      }
      out.push(`<li>${inline(m[1])}</li>`);
    } else if ((m = line.match(/^(\d+)\.\s+(.*)$/))) {
      if (list !== "ol") {
        closeList();
        out.push(`<ol start="${m[1]}">`);
        list = "ol";
      }
      out.push(`<li>${inline(m[2])}</li>`);
    } else if (/^(-{3,}|\*{3,})$/.test(line)) {
      closeList();
      out.push("<hr>");
    } else {
      closeList();
      out.push(`<p>${inline(line)}</p>`);
    }
  }
  if (code) out.push(`<pre class="hl"${codeLang ? ` data-lang="${escapeHtml(codeLang)}"` : ""}>${highlight(code.join("\n"), codeLang)}</pre>`);
  closeList();
  return out.join("");
}
