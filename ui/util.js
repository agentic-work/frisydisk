// Small helpers shared by the UI modules.

/** Decimal units, matching the Rust side. */
export function fmtBytes(n) {
  const units = ["bytes", "KB", "MB", "GB", "TB", "PB"];
  let v = n;
  let i = 0;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
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

/** Minimal Markdown to HTML: headings, lists, code fences, bold, italics, inline code. Input is escaped first. */
export function markdown(text) {
  const inline = (s) =>
    escapeHtml(s)
      .replace(/`([^`]+)`/g, "<code>$1</code>")
      .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
      .replace(/(^|[\s(])\*([^*\s][^*]*)\*/g, "$1<em>$2</em>");
  const out = [];
  let code = null;
  let list = null;
  const closeList = () => {
    if (list) out.push(`</${list}>`);
    list = null;
  };
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (line.startsWith("```")) {
      if (code) {
        out.push(`<pre>${escapeHtml(code.join("\n"))}</pre>`);
        code = null;
      } else {
        closeList();
        code = [];
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
  if (code) out.push(`<pre>${escapeHtml(code.join("\n"))}</pre>`);
  closeList();
  return out.join("");
}
