// Number formatting and colours, matching the desktop app.

export function bytes(n) {
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let v = n;
  let i = 0;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
    i++;
  }
  if (i === 0) return `${n} B`;
  return `${v >= 100 ? v.toFixed(0) : v >= 10 ? v.toFixed(1) : v.toFixed(2)} ${units[i]}`;
}

export const count = (n) => Number(n).toLocaleString("en-US");
export const plural = (n, w) => `${count(n)} ${w}${n === 1 ? "" : "s"}`;

/** Hue for an item at fraction `t` (0..1) around its folder, as #rrggbb. */
export function hue(t, light = 0.68, sat = 0.55) {
  const h = (28 + t * 325) % 360;
  const a = sat * Math.min(light, 1 - light);
  const f = (k) => {
    const x = (k + h / 30) % 12;
    const c = light - a * Math.max(-1, Math.min(x - 3, 9 - x, 1));
    return Math.round(c * 255).toString(16).padStart(2, "0");
  };
  return `#${f(0)}${f(8)}${f(4)}`;
}

/** A bar of `width` cells filled to `share`, using eighth blocks for precision. */
export function bar(share, width) {
  const eighths = Math.round(Math.max(0, Math.min(1, share)) * width * 8);
  const full = Math.floor(eighths / 8);
  const part = eighths % 8;
  const partial = part ? "▏▎▍▌▋▊▉"[part - 1] : "";
  return ("█".repeat(full) + partial).padEnd(width, " ");
}

/** Shorten from the middle to fit `max` columns. */
export function fit(text, max) {
  if (text.length <= max) return text;
  if (max <= 1) return text.slice(0, max);
  const keep = max - 1;
  return text.slice(0, Math.ceil(keep / 2)) + "…" + text.slice(text.length - Math.floor(keep / 2));
}
