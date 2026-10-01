// Pure layout maths for the three chart types. No DOM access, so it can be
// tested with `node --test`.

/** Give every node its depth, parent and the slice [h0, h1) of the colour wheel it owns. */
export function prepare(tree) {
  const walk = (node, depth, h0, h1, parent) => {
    node.depth = depth;
    node.parent = parent;
    node.h0 = h0;
    node.h1 = h1;
    let cursor = h0;
    const span = h1 - h0;
    const total = node.size || 1;
    for (const c of node.children || []) {
      const w = (span * c.size) / total;
      walk(c, depth + 1, cursor, Math.min(h1, cursor + w), node);
      cursor += w;
    }
  };
  walk(tree, 0, 0, 1, null);
  return tree;
}

// ---- Sunburst -------------------------------------------------------------

/** Arcs for every descendant down to maxDepth. Angles are fractions of a turn, clockwise from 12 o'clock. */
export function sunburst(tree, maxDepth = 6) {
  const out = [];
  const walk = (node) => {
    for (const c of node.children || []) {
      if (c.depth > maxDepth || c.h1 <= c.h0) continue;
      out.push({ node: c, depth: c.depth, a0: c.h0, a1: c.h1 });
      walk(c);
    }
  };
  walk(tree);
  return out;
}

/** Ring radii: inner rings are wider than outer ones. */
export function sunburstGeometry(width, height, maxDepth = 6) {
  const radius = Math.max(40, Math.min(width, height) / 2 - 14);
  const hole = radius * 0.25;
  const shrink = 0.88;
  let sum = 0;
  for (let d = 0; d < maxDepth; d++) sum += shrink ** d;
  const first = (radius - hole) / sum;
  const edges = [hole];
  for (let d = 0; d < maxDepth; d++) edges.push(edges[d] + first * shrink ** d);
  return { cx: width / 2, cy: height / 2, hole, edges, maxDepth };
}

/** 'center', a segment index, or -1. */
export function hitSunburst(segs, geo, x, y) {
  const dx = x - geo.cx;
  const dy = y - geo.cy;
  const r = Math.hypot(dx, dy);
  if (r < geo.hole) return "center";
  let depth = -1;
  for (let d = 1; d <= geo.maxDepth; d++) {
    if (r < geo.edges[d]) {
      depth = d;
      break;
    }
  }
  if (depth < 0) return -1;
  let frac = (Math.atan2(dy, dx) + Math.PI / 2) / (2 * Math.PI);
  if (frac < 0) frac += 1;
  for (let i = 0; i < segs.length; i++) {
    const s = segs[i];
    if (s.depth === depth && frac >= s.a0 && frac < s.a1) return i;
  }
  return -1;
}

// ---- Treemap --------------------------------------------------------------

export const HEADER = 17;

/** Squarified treemap (Bruls, Huizing, van Wijk). `areas` sorted descending, summing to the rect's area. */
export function squarify(areas, x, y, w, h) {
  const rects = [];
  let i = 0;
  const worst = (row, sum, side) => {
    if (!row.length || sum <= 0) return Infinity;
    const max = row[0];
    const min = row[row.length - 1];
    const s2 = sum * sum;
    const w2 = side * side;
    return Math.max((w2 * max) / s2, s2 / (w2 * min));
  };
  while (i < areas.length) {
    const side = Math.min(w, h);
    if (side <= 0) {
      for (; i < areas.length; i++) rects.push({ x, y, w: 0, h: 0 });
      break;
    }
    const row = [areas[i]];
    let sum = areas[i];
    let j = i + 1;
    while (j < areas.length && worst([...row, areas[j]], sum + areas[j], side) <= worst(row, sum, side)) {
      row.push(areas[j]);
      sum += areas[j];
      j++;
    }
    const thick = sum / side;
    let off = 0;
    const tall = w >= h; // the row is a column down the left edge
    for (const a of row) {
      const len = (a / sum) * side;
      rects.push(tall ? { x, y: y + off, w: thick, h: len } : { x: x + off, y, w: len, h: thick });
      off += len;
    }
    if (tall) {
      x += thick;
      w = Math.max(0, w - thick);
    } else {
      y += thick;
      h = Math.max(0, h - thick);
    }
    i = j;
  }
  return rects;
}

/** Nested tiles; parents come before their children. */
export function treemap(tree, x, y, w, h, maxDepth = 5, minSide = 6) {
  const out = [];
  const place = (node, x, y, w, h) => {
    const kids = (node.children || []).filter((c) => c.size > 0);
    const total = kids.reduce((s, c) => s + c.size, 0);
    if (!kids.length || total <= 0 || w < 2 || h < 2) return;
    const area = w * h;
    const rects = squarify(kids.map((c) => (c.size / total) * area), x, y, w, h);
    kids.forEach((c, i) => {
      const r = rects[i];
      if (r.w < 1 || r.h < 1) return;
      const nested =
        c.dir && c.children && c.children.length > 0 && c.depth < maxDepth && r.w > minSide * 4 && r.h > minSide * 3 + HEADER;
      out.push({ node: c, x: r.x, y: r.y, w: r.w, h: r.h, depth: c.depth, nested });
      if (nested) place(c, r.x + 3, r.y + HEADER + 1, r.w - 6, r.h - HEADER - 4);
    });
  };
  place(tree, x, y, w, h);
  return out;
}

export function hitTreemap(tiles, x, y) {
  for (let i = tiles.length - 1; i >= 0; i--) {
    const t = tiles[i];
    if (x >= t.x && x < t.x + t.w && y >= t.y && y < t.y + t.h) return i;
  }
  return -1;
}

// ---- Sankey ---------------------------------------------------------------

/**
 * Flow diagram of where the space goes, left to right. Only the largest
 * folders are opened up: a node gets a next column when it is a folder holding
 * at least `expandShare` of the focused folder. Everything else in a column is
 * merged into one "other" node.
 */
export function sankey(tree, width, height, opts = {}) {
  const { columns = 4, keep = [9, 5, 4], expandShare = 0.035, nodeWidth = 14, gap = 7, labelSpace = 150 } = opts;
  const total = tree.size || 1;

  // Pick what to show: a pruned copy of the tree.
  const pick = (node, level) => {
    const item = { node, level, size: node.size, kids: [] };
    const canExpand = level === 0 || (node.dir && node.id != null && node.size / total >= expandShare);
    if (level >= columns - 1 || !canExpand || !(node.children || []).length) return item;
    const kids = node.children.filter((c) => c.size > 0);
    const shown = kids.slice(0, keep[level] ?? 4).filter((c) => c.id != null);
    const rest = kids.filter((c) => !shown.includes(c));
    for (const c of shown) item.kids.push(pick(c, level + 1));
    if (rest.length) {
      const size = rest.reduce((s, c) => s + c.size, 0);
      const count = rest.reduce((s, c) => s + (c.group || 1), 0);
      const files = rest.reduce((s, c) => s + c.files, 0);
      const last = rest[rest.length - 1];
      item.kids.push({
        node: { id: null, name: `${count} other items`, size, files, dir: false, group: count, h0: rest[0].h0, h1: last.h1, depth: node.depth + 1, parent: node },
        level: level + 1,
        size,
        kids: [],
      });
    }
    return item;
  };
  const root = pick(tree, 0);

  // Columns in parent order, so links never cross within a parent.
  const cols = [];
  const collect = (item) => {
    (cols[item.level] ||= []).push(item);
    item.kids.forEach(collect);
  };
  collect(root);

  const used = cols.length;
  const colGap = used > 1 ? (width - labelSpace - nodeWidth) / (used - 1) : 0;
  // One scale for every column, set by the most crowded one.
  let scale = Infinity;
  for (const col of cols) {
    const sum = col.reduce((s, i) => s + i.size, 0);
    scale = Math.min(scale, (height - gap * (col.length - 1)) / sum);
  }

  const nodes = [];
  const links = [];
  cols.forEach((col, ci) => {
    const sum = col.reduce((s, i) => s + i.size, 0);
    let y = (height - (sum * scale + gap * (col.length - 1))) / 2;
    for (const item of col) {
      item.x = ci * colGap;
      item.y = y;
      item.h = Math.max(1, item.size * scale);
      y += item.h + gap;
      nodes.push({ node: item.node, x: item.x, y: item.y, w: nodeWidth, h: item.h, level: ci, leaf: item.kids.length === 0 });
    }
  });
  const link = (item) => {
    let off = item.y;
    for (const k of item.kids) {
      links.push({ node: k.node, x0: item.x + nodeWidth, x1: k.x, y0: off, y1: k.y, h: k.h });
      off += k.h;
      link(k);
    }
  };
  link(root);
  return { nodes, links, columns: used, nodeWidth };
}
