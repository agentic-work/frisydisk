// Canvas drawing and hit testing for the sunburst, treemap and Sankey views.

import { hitSunburst, hitTreemap, sankey, sunburst, sunburstGeometry, treemap, HEADER } from "./layout.js";
import { fmtBytes } from "./util.js";

const DEPTH = 6;
const FONT = '"Avenir Next", "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif';

function theme() {
  const css = getComputedStyle(document.documentElement);
  const v = (name) => css.getPropertyValue(name).trim();
  return { stage: v("--stage"), panel: v("--panel"), text: v("--text"), dim: v("--dim"), line: v("--line"), dark: v("--is-dark") === "1" };
}

/** Colour of a chart item: hue from its place around the focused folder. */
export function colorFor(node, opts = {}) {
  const { lift = false, alpha = 1, dark = true } = opts;
  if (!node || node.id == null) return `oklch(${dark ? 0.5 : 0.72} 0.015 250 / ${0.55 * alpha})`;
  const hue = 28 + ((node.h0 + node.h1) / 2) * 325;
  const d = Math.max(0, (node.depth || 1) - 1);
  let l = (node.dir ? 0.74 : 0.82) - d * 0.03;
  if (!dark) l -= 0.06;
  if (lift) l += 0.07;
  const c = Math.max(0.05, (node.dir ? 0.14 : 0.075) - d * 0.008);
  return `oklch(${l.toFixed(3)} ${c.toFixed(3)} ${hue.toFixed(1)} / ${alpha})`;
}

function fit(ctx, text, max) {
  if (ctx.measureText(text).width <= max) return text;
  let lo = 0;
  let hi = text.length;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (ctx.measureText(text.slice(0, mid) + "…").width <= max) lo = mid;
    else hi = mid - 1;
  }
  return lo > 1 ? text.slice(0, lo) + "…" : "";
}

function related(node, hover) {
  if (!hover) return true;
  for (let n = node; n; n = n.parent) if (n === hover) return true;
  return false;
}

export class Chart {
  constructor(canvas) {
    this.canvas = canvas;
    this.ctx = canvas.getContext("2d");
    this.mode = "sunburst";
    this.tree = null;
    this.hover = null;
    this.progress = 1;
    this.shapes = null;
  }

  resize() {
    const dpr = window.devicePixelRatio || 1;
    const { clientWidth: w, clientHeight: h } = this.canvas;
    if (this.canvas.width !== Math.round(w * dpr) || this.canvas.height !== Math.round(h * dpr)) {
      this.canvas.width = Math.round(w * dpr);
      this.canvas.height = Math.round(h * dpr);
    }
    this.w = w;
    this.h = h;
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  }

  /** Set what to draw. `animate` plays the short opening sweep. */
  set(tree, mode, animate) {
    this.tree = tree;
    this.mode = mode;
    this.hover = null;
    this.layout();
    cancelAnimationFrame(this.raf);
    const calm = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (!animate || calm) {
      this.progress = 1;
      this.draw();
      return;
    }
    const start = performance.now();
    const step = (now) => {
      const t = Math.min(1, (now - start) / 320);
      this.progress = 1 - (1 - t) ** 3;
      this.draw();
      if (t < 1) this.raf = requestAnimationFrame(step);
    };
    this.raf = requestAnimationFrame(step);
  }

  layout() {
    this.resize();
    if (!this.tree) return;
    const pad = 14;
    if (this.mode === "sunburst") {
      this.geo = sunburstGeometry(this.w, this.h, DEPTH);
      this.shapes = sunburst(this.tree, DEPTH);
    } else if (this.mode === "treemap") {
      this.shapes = treemap(this.tree, pad, pad, this.w - pad * 2, this.h - pad * 2);
    } else {
      const labelSpace = Math.min(190, Math.max(110, this.w * 0.2));
      this.flow = sankey(this.tree, this.w - 44, this.h - 56, { labelSpace });
      this.shapes = this.flow.nodes;
    }
  }

  /** The node under a point, or 'center' (sunburst hub), or null. */
  hit(x, y) {
    if (!this.tree || !this.shapes) return null;
    if (this.mode === "sunburst") {
      const i = hitSunburst(this.shapes, this.geo, x, y);
      return i === "center" ? "center" : i >= 0 ? this.shapes[i].node : null;
    }
    if (this.mode === "treemap") {
      const i = hitTreemap(this.shapes, x, y);
      return i >= 0 ? this.shapes[i].node : null;
    }
    const ox = 22;
    const oy = 28;
    for (const n of this.flow.nodes) {
      if (n.level > 0 && x >= ox + n.x - 3 && x < ox + n.x + n.w + (n.labelWidth || 0) + 8 && y >= oy + n.y - 2 && y < oy + n.y + Math.max(n.h, 12) + 2) return n.node;
    }
    const dpr = window.devicePixelRatio || 1;
    for (const l of this.flow.links) {
      if (l.path && this.ctx.isPointInPath(l.path, x * dpr, y * dpr)) return l.node;
    }
    return null;
  }

  setHover(node) {
    const next = node === "center" ? null : node;
    if (next === this.hover) return;
    this.hover = next;
    if (this.progress >= 1) this.draw();
  }

  draw() {
    const ctx = this.ctx;
    this.resize();
    ctx.clearRect(0, 0, this.w, this.h);
    if (!this.tree || !this.tree.size) return;
    const th = theme();
    if (this.mode === "sunburst") this.drawSunburst(th);
    else if (this.mode === "treemap") this.drawTreemap(th);
    else this.drawSankey(th);
  }

  drawSunburst(th) {
    const ctx = this.ctx;
    const { cx, cy, edges, hole } = this.geo;
    const p = this.progress;
    const hover = this.hover;
    const turn = Math.PI * 2;

    // Soft halo so the disc sits on the stage instead of floating.
    const halo = ctx.createRadialGradient(cx, cy, hole, cx, cy, edges[DEPTH] + 30);
    halo.addColorStop(0, th.dark ? "rgba(255,255,255,0.05)" : "rgba(20,32,43,0.05)");
    halo.addColorStop(1, "rgba(0,0,0,0)");
    ctx.fillStyle = halo;
    ctx.beginPath();
    ctx.arc(cx, cy, edges[DEPTH] + 30, 0, turn);
    ctx.fill();

    ctx.lineWidth = 1.25;
    ctx.strokeStyle = th.stage;
    for (const s of this.shapes) {
      const lit = hover && s.node === hover;
      const dim = hover && !related(s.node, hover);
      const a0 = s.a0 * p * turn - Math.PI / 2;
      const a1 = s.a1 * p * turn - Math.PI / 2;
      const r0 = edges[s.depth - 1] + 1;
      const r1 = edges[s.depth] + (lit ? 5 : 0);
      ctx.beginPath();
      ctx.arc(cx, cy, r1, a0, a1);
      ctx.arc(cx, cy, r0, a1, a0, true);
      ctx.closePath();
      ctx.fillStyle = colorFor(s.node, { lift: lit, alpha: dim ? 0.3 : 1, dark: th.dark });
      ctx.fill();
      ctx.stroke();
    }

    // Names on arcs that are long enough to hold one.
    if (p >= 1) {
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      for (const s of this.shapes) {
        if (s.depth > 3 || s.node.id == null) continue;
        const rm = (edges[s.depth - 1] + edges[s.depth]) / 2;
        const band = edges[s.depth] - edges[s.depth - 1];
        const arc = (s.a1 - s.a0) * turn * rm;
        if (arc < 54 || band < 20) continue;
        if (hover && !related(s.node, hover)) continue;
        const mid = ((s.a0 + s.a1) / 2) * turn - Math.PI / 2;
        ctx.save();
        ctx.translate(cx + Math.cos(mid) * rm, cy + Math.sin(mid) * rm);
        let rot = mid + Math.PI / 2;
        if (Math.sin(mid) > 0) rot += Math.PI; // keep text upright in the lower half
        ctx.rotate(rot);
        ctx.fillStyle = "rgba(12,22,30,0.86)";
        ctx.font = `600 11.5px ${FONT}`;
        const name = fit(ctx, s.node.name, arc - 14);
        if (name) {
          const two = band > 34 && arc > 70;
          ctx.fillText(name, 0, two ? -6 : 0);
          if (two) {
            ctx.font = `500 10.5px ${FONT}`;
            ctx.fillStyle = "rgba(12,22,30,0.62)";
            ctx.fillText(fmtBytes(s.node.size), 0, 8);
          }
        }
        ctx.restore();
      }
    }

    // Hub.
    const hub = ctx.createRadialGradient(cx - hole * 0.3, cy - hole * 0.35, hole * 0.1, cx, cy, hole);
    hub.addColorStop(0, th.dark ? "#2a3f52" : "#ffffff");
    hub.addColorStop(1, th.panel);
    ctx.fillStyle = hub;
    ctx.beginPath();
    ctx.arc(cx, cy, hole - 4, 0, turn);
    ctx.fill();
    ctx.strokeStyle = th.line;
    ctx.lineWidth = 1;
    ctx.stroke();
  }

  drawTreemap(th) {
    const ctx = this.ctx;
    const hover = this.hover;
    ctx.globalAlpha = this.progress;
    ctx.textBaseline = "top";
    ctx.textAlign = "left";
    for (const t of this.shapes) {
      const lit = hover && t.node === hover;
      const dim = hover && !related(t.node, hover) && !related(hover, t.node);
      const x = t.x + 0.75;
      const y = t.y + 0.75;
      const w = t.w - 1.5;
      const h = t.h - 1.5;
      if (w <= 0 || h <= 0) continue;
      ctx.beginPath();
      ctx.roundRect(x, y, w, h, t.nested ? 5 : Math.min(4, w / 3, h / 3));
      if (t.nested) {
        ctx.fillStyle = colorFor(t.node, { alpha: dim ? 0.1 : 0.22, dark: th.dark });
        ctx.fill();
        ctx.strokeStyle = colorFor(t.node, { alpha: lit ? 1 : 0.45, dark: th.dark });
        ctx.lineWidth = lit ? 2 : 1;
        ctx.stroke();
        if (w > 46) {
          ctx.font = `600 11px ${FONT}`;
          ctx.fillStyle = th.text;
          const size = fmtBytes(t.node.size);
          const name = fit(ctx, t.node.name, w - 14 - (w > 150 ? ctx.measureText(size).width + 10 : 0));
          ctx.fillText(name, x + 6, y + 3.5);
          if (w > 150 && name) {
            ctx.fillStyle = th.dim;
            ctx.font = `500 11px ${FONT}`;
            ctx.textAlign = "right";
            ctx.fillText(size, x + w - 6, y + 3.5);
            ctx.textAlign = "left";
          }
        }
      } else {
        ctx.fillStyle = colorFor(t.node, { lift: lit, alpha: dim ? 0.35 : 1, dark: th.dark });
        ctx.fill();
        if (lit) {
          ctx.strokeStyle = th.text;
          ctx.lineWidth = 2;
          ctx.stroke();
        }
        if (w > 58 && h > 20) {
          ctx.font = `600 11px ${FONT}`;
          ctx.fillStyle = "rgba(12,22,30,0.86)";
          const name = fit(ctx, t.node.name, w - 12);
          ctx.fillText(name, x + 6, y + 5);
          if (h > 36 && name) {
            ctx.font = `500 10.5px ${FONT}`;
            ctx.fillStyle = "rgba(12,22,30,0.6)";
            ctx.fillText(fmtBytes(t.node.size), x + 6, y + 19);
          }
        }
      }
    }
    ctx.globalAlpha = 1;
  }

  drawSankey(th) {
    const ctx = this.ctx;
    const { nodes, links } = this.flow;
    const hover = this.hover;
    const p = this.progress;
    const dpr = window.devicePixelRatio || 1;
    const ox = 22;
    const oy = 28;

    for (const l of links) {
      const lit = hover && related(l.node, hover);
      const dim = hover && !lit;
      const x0 = ox + l.x0;
      const x1 = ox + l.x0 + (l.x1 - l.x0) * p;
      const xm = (x0 + x1) / 2;
      const y0 = oy + l.y0;
      const y1 = oy + l.y1;
      const path = new Path2D();
      path.moveTo(x0, y0);
      path.bezierCurveTo(xm, y0, xm, y1, x1, y1);
      path.lineTo(x1, y1 + l.h);
      path.bezierCurveTo(xm, y1 + l.h, xm, y0 + l.h, x0, y0 + l.h);
      path.closePath();
      // Hit testing happens in device pixels, so keep an unscaled copy.
      const hitPath = new Path2D();
      hitPath.addPath(path, new DOMMatrix().scale(dpr, dpr));
      l.path = hitPath;
      const grad = ctx.createLinearGradient(x0, 0, x1, 0);
      grad.addColorStop(0, colorFor(l.node, { alpha: dim ? 0.08 : lit ? 0.55 : 0.22, dark: th.dark }));
      grad.addColorStop(1, colorFor(l.node, { alpha: dim ? 0.12 : lit ? 0.75 : 0.42, dark: th.dark }));
      ctx.fillStyle = grad;
      ctx.fill(path);
    }

    ctx.textBaseline = "middle";
    ctx.textAlign = "left";
    for (const n of nodes) {
      const lit = hover && n.node === hover;
      const dim = hover && !related(n.node, hover) && !related(hover, n.node);
      const x = ox + n.x * (n.level === 0 ? 1 : p);
      const y = oy + n.y;
      ctx.beginPath();
      ctx.roundRect(x, y, n.w, n.h, Math.min(4, n.h / 2));
      ctx.fillStyle = n.level === 0 ? th.text : colorFor(n.node, { lift: lit, alpha: dim ? 0.35 : 1, dark: th.dark });
      ctx.fill();
      if (p < 1 || n.level === 0) continue;
      if (n.h >= 11) {
        ctx.globalAlpha = dim ? 0.4 : 1;
        ctx.font = `600 11.5px ${FONT}`;
        ctx.fillStyle = th.text;
        // Labels may run up to the next column; only the last column gets the rest of the width.
        const next = nodes.find((m) => m.level === n.level + 1);
        const room = next ? ox + next.x - x - n.w - 18 : this.w - x - n.w - 16;
        const name = fit(ctx, n.node.name, Math.max(40, room));
        const two = n.h >= 27;
        ctx.fillText(name, x + n.w + 7, y + n.h / 2 - (two ? 7 : 0));
        n.labelWidth = ctx.measureText(name).width;
        if (two) {
          ctx.font = `500 10.5px ${FONT}`;
          ctx.fillStyle = th.dim;
          ctx.fillText(fmtBytes(n.node.size), x + n.w + 7, y + n.h / 2 + 7);
        }
        ctx.globalAlpha = 1;
      }
    }

    // The focused folder's own label, above its bar.
    const root = nodes[0];
    ctx.font = `600 12px ${FONT}`;
    ctx.fillStyle = th.dim;
    ctx.textBaseline = "alphabetic";
    ctx.fillText(`${fit(ctx, root.node.name, 220)}  ${fmtBytes(root.node.size)}`, ox, oy + root.y - 9);
  }
}

export { HEADER };
