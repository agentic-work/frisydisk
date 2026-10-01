import assert from "node:assert/strict";
import { test } from "node:test";
import { hitSunburst, hitTreemap, prepare, sankey, squarify, sunburst, sunburstGeometry, treemap } from "./layout.js";
import { fmtBytes, markdown } from "./util.js";

const file = (id, name, size) => ({ id, name, size, files: 1, dir: false, group: 0, children: [] });

/** root(100) = big(60: x 40, y 20) + mid(30) + group of ten small files (10). */
function sample() {
  return prepare({
    id: 0, name: "r", size: 100, files: 13, dir: true, group: 0,
    children: [
      { id: 1, name: "big", size: 60, files: 2, dir: true, group: 0, children: [file(2, "x.mov", 40), file(3, "y.zip", 20)] },
      file(4, "mid.mov", 30),
      { id: null, name: "10 smaller items", size: 10, files: 10, dir: false, group: 10, children: [] },
    ],
  });
}

test("prepare gives each node its slice of the wheel", () => {
  const t = sample();
  const [big, mid, group] = t.children;
  assert.deepEqual([big.h0, big.h1], [0, 0.6]);
  assert.ok(Math.abs(mid.h1 - 0.9) < 1e-9 && Math.abs(group.h1 - 1) < 1e-9);
  assert.equal(big.children[1].parent, big);
  assert.equal(big.children[0].depth, 2);
  assert.ok(Math.abs(big.children[1].h1 - 0.6) < 1e-9);
});

test("sunburst arcs nest and can be hit", () => {
  const t = sample();
  const segs = sunburst(t);
  assert.equal(segs.length, 5);
  const geo = sunburstGeometry(400, 400);
  assert.equal(hitSunburst(segs, geo, geo.cx, geo.cy), "center");
  const r1 = (geo.edges[0] + geo.edges[1]) / 2;
  assert.equal(segs[hitSunburst(segs, geo, geo.cx + 2, geo.cy - r1)].node.name, "big");
  assert.equal(segs[hitSunburst(segs, geo, geo.cx - 2, geo.cy - r1)].node.id, null);
  const r2 = (geo.edges[1] + geo.edges[2]) / 2;
  assert.equal(segs[hitSunburst(segs, geo, geo.cx + 2, geo.cy - r2)].node.name, "x.mov");
  assert.equal(hitSunburst(segs, geo, 1, 1), -1);
  assert.ok(geo.edges[1] - geo.edges[0] > geo.edges[6] - geo.edges[5], "inner rings are wider");
});

test("squarify fills the rectangle without overlap", () => {
  const weights = [6, 6, 4, 3, 2, 2, 1];
  const total = weights.reduce((a, b) => a + b);
  const rects = squarify(weights.map((w) => (w / total) * 240000), 0, 0, 600, 400);
  rects.forEach((r, i) => {
    assert.ok(Math.abs(r.w * r.h - (weights[i] / total) * 240000) < 1);
    assert.ok(r.x >= -0.01 && r.y >= -0.01 && r.x + r.w <= 600.01 && r.y + r.h <= 400.01);
    assert.ok(Math.max(r.w / r.h, r.h / r.w) < 4);
  });
  for (let i = 0; i < rects.length; i++)
    for (let j = i + 1; j < rects.length; j++) {
      const ox = Math.min(rects[i].x + rects[i].w, rects[j].x + rects[j].w) - Math.max(rects[i].x, rects[j].x);
      const oy = Math.min(rects[i].y + rects[i].h, rects[j].y + rects[j].h) - Math.max(rects[i].y, rects[j].y);
      assert.ok(ox < 0.01 || oy < 0.01, `tiles ${i} and ${j} overlap`);
    }
});

test("treemap nests children inside their folder", () => {
  const tiles = treemap(sample(), 0, 0, 800, 600);
  const big = tiles.find((t) => t.node.name === "big");
  const x = tiles.find((t) => t.node.name === "x.mov");
  assert.ok(big.nested);
  assert.ok(x.x >= big.x && x.y >= big.y && x.x + x.w <= big.x + big.w && x.y + x.h <= big.y + big.h);
  assert.equal(tiles[hitTreemap(tiles, x.x + x.w / 2, x.y + x.h / 2)].node.name, "x.mov");
  assert.equal(hitTreemap(tiles, -5, -5), -1);
});

test("sankey opens only the largest folders and conserves size", () => {
  const t = sample();
  const flow = sankey(t, 900, 500);
  const level = (n) => flow.nodes.filter((x) => x.level === n);
  assert.equal(level(0).length, 1);
  assert.deepEqual(level(1).map((n) => n.node.name), ["big", "mid.mov", "10 other items"]);
  assert.deepEqual(level(2).map((n) => n.node.name), ["x.mov", "y.zip"]);
  // One scale everywhere: heights are proportional to size.
  const root = level(0)[0];
  const big = level(1)[0];
  assert.ok(Math.abs(big.h / root.h - 0.6) < 1e-6);
  const kids = level(2).reduce((s, n) => s + n.h, 0);
  assert.ok(Math.abs(kids - big.h) < 1e-6);
  assert.equal(flow.links.length, 5);
  assert.ok(flow.nodes.every((n) => n.y >= -0.01 && n.y + n.h <= 500.01));

  // A folder below the share threshold stays closed.
  const closed = sankey(t, 900, 500, { expandShare: 0.7 });
  assert.equal(closed.nodes.filter((n) => n.level === 2).length, 0);
  // Only `keep` children are shown per column; the rest merge into one node.
  const few = sankey(t, 900, 500, { keep: [1, 1, 1] });
  assert.deepEqual(few.nodes.filter((n) => n.level === 1).map((n) => n.node.name), ["big", "11 other items"]);
});

test("formatting and markdown escape their input", () => {
  assert.equal(fmtBytes(1_500_000_000), "1.50 GB");
  assert.equal(fmtBytes(999), "999 bytes");
  const html = markdown("## Title\n- **bold** `rm <x>`\n\n```\na < b\n```\n<script>alert(1)</script>");
  assert.ok(html.includes("<h3>Title</h3>") && html.includes("<strong>bold</strong>"));
  assert.ok(html.includes("<code>rm &lt;x&gt;</code>") && html.includes("<pre>a &lt; b</pre>"));
  assert.ok(!html.includes("<script>"));
});
