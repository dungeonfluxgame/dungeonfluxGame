// PLAY-V14 (evaluator tooling, not application code): the layout check of the shared display. At one
// moment of a screen it reads the boxes of every visible text-bearing element (one box per rendered line,
// from the text itself, not from its container) and reports:
//   overlap    two elements' text lines cross by more than 2 px (an element and its own descendant never do:
//              one contains the other by design)
//   clipped    a line is cut off by an ancestor's overflow or by the viewport
//   ellipsis   text shown with an ellipsis (text-overflow, or a line clamp that hides lines), unless the element
//              or an ancestor carries data-single-line (marked as intentionally one line)
//   unsafe     a line lies outside the 5 % TV-safe margin
// A violation is only reported when two samples taken a moment apart both show it, so an entrance animation
// in flight is not mistaken for a layout.
export const TOLERANCE = 2;
export const SAFE_MARGIN = 0.05;
export const VIEWPORTS = [[1280, 720], [1920, 1080], [3840, 2160]];

// Runs in the page. Returns { lines, violations }.
const MEASURE = ({ tolerance, safe }) => {
  const width = innerWidth;
  const height = innerHeight;
  const describe = (el) => {
    const slot = el.getAttribute('data-slot');
    const cls = String(el.className && el.className.baseVal !== undefined ? el.className.baseVal : el.className || '').trim().split(/\s+/).filter(Boolean).slice(0, 2).join('.');
    return `${el.tagName.toLowerCase()}${cls ? `.${cls}` : ''}${slot ? `[${slot}]` : ''}`;
  };
  const textOf = (el) => (el.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 70);
  const alpha = (color) => {
    const match = color.match(/rgba?\(([^)]+)\)/);
    if (!match) return 1;
    const parts = match[1].split(/[ ,/]+/).filter(Boolean);
    return parts.length > 3 ? Number(parts[3]) : 1;
  };
  const shown = (el) => {
    let opacity = 1;
    for (let up = el; up; up = up.parentElement) {
      const cs = getComputedStyle(up);
      if (cs.display === 'none' || cs.visibility === 'hidden') return false;
      opacity *= Number(cs.opacity);
    }
    if (opacity < 0.3) return false;
    const cs = getComputedStyle(el);
    const fill = cs.webkitTextFillColor || cs.color;
    const clipsToText = (cs.webkitBackgroundClip || cs.backgroundClip || '').includes('text');
    return clipsToText || (alpha(cs.color) > 0.05 && alpha(fill) > 0.05);
  };
  const byElement = new Map();
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const el = node.parentElement;
    if (!el || !node.nodeValue.trim() || ['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEXTAREA', 'OPTION'].includes(el.tagName)) continue;
    if (!byElement.has(el)) byElement.set(el, []);
    byElement.get(el).push(node);
  }
  // The boxes that cut an element's text off: its own overflow, then that of each ancestor in its containing-block
  // chain (an absolutely positioned or fixed element is not cut by the overflow of ancestors between it and its
  // containing block). An inline element's overflow clips nothing; a transformed box is read as drawn.
  const transformed = (cs) => cs.transform !== 'none' || cs.filter !== 'none' || cs.perspective !== 'none' || /transform/.test(cs.willChange);
  const clipRects = (el) => {
    const clips = [];
    const add = (node) => {
      const cs = getComputedStyle(node);
      if ([cs.overflowX, cs.overflowY].some((v) => v !== 'visible') && !['inline', 'contents'].includes(cs.display)) {
        const box = node.getBoundingClientRect();
        clips.push({ el: node, left: box.left, top: box.top, right: box.right, bottom: box.bottom });
      }
    };
    add(el);
    let node = el;
    while (node && node !== document.documentElement) {
      const cs = getComputedStyle(node);
      let next = node.parentElement;
      if (cs.position === 'fixed') {
        while (next && next !== document.documentElement && !transformed(getComputedStyle(next))) next = next.parentElement;
      } else if (cs.position === 'absolute') {
        while (next && next !== document.documentElement && getComputedStyle(next).position === 'static' && !transformed(getComputedStyle(next))) next = next.parentElement;
      }
      if (!next || next === document.documentElement) break;
      add(next);
      node = next;
    }
    return clips;
  };
  const items = [];
  for (const [el, nodes] of byElement) {
    if (!shown(el)) continue;
    const lines = [];
    for (const node of nodes) {
      const range = document.createRange();
      range.selectNodeContents(node);
      for (const rect of range.getClientRects()) {
        if (rect.width > 1 && rect.height > 1) lines.push({ left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom });
      }
    }
    if (!lines.length) continue;
    items.push({ el, lines, name: describe(el), text: textOf(el), clips: clipRects(el) });
  }
  const violations = [];
  const intersect = (a, b) => Math.min(a.right, b.right) - Math.max(a.left, b.left) > tolerance && Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top) > tolerance;
  const area = (r) => Math.max(0, r.right - r.left) * Math.max(0, r.bottom - r.top);
  const cut = (line, clip) => ({ left: Math.max(line.left, clip.left), top: Math.max(line.top, clip.top), right: Math.min(line.right, clip.right), bottom: Math.min(line.bottom, clip.bottom) });
  const isSingleLine = (el) => Boolean(el.closest('[data-single-line]'));
  for (const item of items) {
    // Lines wholly hidden by a clip are not on screen at all (a collapsed panel); a line only partly there is clipped.
    const partly = [];
    let anyShown = false;
    for (const line of item.lines) {
      let seen = { ...line };
      let clippedBy = null;
      for (const clip of item.clips) {
        const next = cut(seen, clip);
        if (area(next) < area(seen) * 0.9 && !clippedBy) clippedBy = clip.el;
        seen = next;
      }
      if (area(seen) > 0.1 * area(line)) anyShown = true;
      if (clippedBy && area(seen) > 0) partly.push({ line, clippedBy, seen });
      item.seen = item.seen || [];
      item.seen.push(seen);
    }
    if (anyShown && partly.length) {
      const first = partly[0];
      violations.push({ kind: 'clipped', a: { name: item.name, text: item.text }, detail: `cut by ${describe(first.clippedBy)}`, boxes: partly.map((p) => p.line) });
    }
    // A viewport clip (the page is the clip of last resort).
    const outside = item.lines.filter((line) => line.left < -tolerance || line.top < -tolerance || line.right > width + tolerance || line.bottom > height + tolerance);
    if (outside.length && item.seen.some((s) => area(s) > 0)) {
      violations.push({ kind: 'clipped', a: { name: item.name, text: item.text }, detail: 'beyond the viewport', boxes: outside });
    }
    const style = getComputedStyle(item.el);
    const clamped = style.webkitLineClamp && style.webkitLineClamp !== 'none';
    const lineHeight = parseFloat(style.lineHeight) || parseFloat(style.fontSize) * 1.2;
    // Under half a line of overflow is a descender or rounding, not a hidden line.
    const cutVertical = item.el.scrollHeight - item.el.clientHeight > Math.max(tolerance, lineHeight * 0.5) && style.overflowY !== 'visible';
    const cutHorizontal = item.el.scrollWidth > item.el.clientWidth + 1 && style.textOverflow === 'ellipsis';
    if ((clamped && cutVertical) || (cutHorizontal && !isSingleLine(item.el))) {
      violations.push({ kind: 'ellipsis', a: { name: item.name, text: item.text }, detail: clamped ? 'a line clamp hides lines' : 'text-overflow ellipsis', boxes: item.lines });
    }
    const margin = { left: width * safe, top: height * safe, right: width * (1 - safe), bottom: height * (1 - safe) };
    const unsafe = item.seen.filter((s) => area(s) > 0 && (s.left < margin.left - tolerance || s.top < margin.top - tolerance || s.right > margin.right + tolerance || s.bottom > margin.bottom + tolerance));
    if (unsafe.length) {
      violations.push({ kind: 'unsafe', a: { name: item.name, text: item.text }, detail: 'outside the 5% TV-safe margin', boxes: unsafe });
    }
  }
  // A box drawn over the text hides it without any text crossing: look at what is on top at three points of each
  // line (pointer-events is switched on for the moment so the translucent layers of the display are hit too).
  const probe = document.createElement('style');
  probe.textContent = '*{pointer-events:auto!important}';
  document.head.appendChild(probe);
  const opaque = (cover) => {
    const cs = getComputedStyle(cover);
    if (['IMG', 'CANVAS', 'VIDEO'].includes(cover.tagName)) return true;
    if (alpha(cs.backgroundColor) >= 0.8) return true;
    const box = cover.getBoundingClientRect();
    return cs.backgroundImage !== 'none' && box.width * box.height < 0.25 * width * height;
  };
  const effective = (cover) => {
    let opacity = 1;
    for (let up = cover; up; up = up.parentElement) opacity *= Number(getComputedStyle(up).opacity);
    return opacity;
  };
  for (const item of items) {
    const coverers = new Map();
    for (const line of item.seen) {
      if (area(line) <= 0) continue;
      const y = (line.top + line.bottom) / 2;
      for (const x of [line.left + 3, (line.left + line.right) / 2, line.right - 3]) {
        for (const top of document.elementsFromPoint(x, y)) {
          if (item.el === top || item.el.contains(top) || top.contains(item.el)) break;
          if (opaque(top) && effective(top) >= 0.5 && !coverers.has(top)) coverers.set(top, line);
        }
      }
    }
    for (const [cover, line] of coverers) {
      const box = cover.getBoundingClientRect();
      violations.push({ kind: 'covered', a: { name: item.name, text: item.text }, b: { name: describe(cover), text: textOf(cover) }, detail: '', boxes: [line, { left: box.left, top: box.top, right: box.right, bottom: box.bottom }] });
    }
  }
  probe.remove();
  for (let i = 0; i < items.length; i += 1) {
    for (let j = i + 1; j < items.length; j += 1) {
      const a = items[i];
      const b = items[j];
      if (a.el.contains(b.el) || b.el.contains(a.el)) continue;
      const crossing = [];
      for (const la of a.seen) for (const lb of b.seen) if (area(la) > 0 && area(lb) > 0 && intersect(la, lb)) crossing.push(la, lb);
      if (crossing.length) violations.push({ kind: 'overlap', a: { name: a.name, text: a.text }, b: { name: b.name, text: b.text }, detail: '', boxes: crossing });
    }
  }
  return { lines: items.length, violations, seen: items.map((item) => `${item.name} "${item.text}" ${item.seen.filter((r) => area(r) > 0).map((r) => `${Math.round(r.left)},${Math.round(r.top)}-${Math.round(r.right)},${Math.round(r.bottom)}`).join(' ')}`) };
};

const key = (v) => `${v.kind}|${v.a.name}|${v.a.text}|${v.b ? `${v.b.name}|${v.b.text}` : ''}`;

// Measures twice, a moment apart, and keeps what both samples show.
export async function measure(page, wait = 350) {
  const first = await page.evaluate(MEASURE, { tolerance: TOLERANCE, safe: SAFE_MARGIN });
  await new Promise((resolve) => setTimeout(resolve, wait));
  const second = await page.evaluate(MEASURE, { tolerance: TOLERANCE, safe: SAFE_MARGIN });
  const seen = new Set(first.violations.map(key));
  if (process.env.DF_DEBUG) console.log(`  samples: ${first.violations.length} then ${second.violations.length}: ${second.violations.map(key).join(' ## ').slice(0, 400)}`);
  return { lines: second.lines, seen: second.seen, violations: second.violations.filter((v) => seen.has(key(v))) };
}

// Draws the offending boxes (red) and the safe margin (dashed) over the page, takes the frame, takes them off.
export async function outlined(page, violations, path) {
  await page.evaluate(({ boxes, safe }) => {
    const layer = document.createElement('div');
    layer.id = 'layout-check-overlay';
    layer.style.cssText = 'position:fixed;inset:0;z-index:2147483647;pointer-events:none';
    const add = (rect, css) => {
      const box = document.createElement('div');
      box.style.cssText = `position:fixed;left:${rect.left}px;top:${rect.top}px;width:${rect.right - rect.left}px;height:${rect.bottom - rect.top}px;box-sizing:border-box;${css}`;
      layer.appendChild(box);
    };
    add({ left: innerWidth * safe, top: innerHeight * safe, right: innerWidth * (1 - safe), bottom: innerHeight * (1 - safe) }, 'border:2px dashed #38e8ff');
    boxes.forEach(({ rects, kind }) => rects.forEach((rect) => add(rect, `border:3px solid ${kind === 'overlap' || kind === 'covered' ? '#ff2d2d' : kind === 'ellipsis' ? '#ffb000' : kind === 'clipped' ? '#ff00d4' : '#38e8ff'};background:#ff2d2d22`)));
    document.body.appendChild(layer);
  }, { boxes: violations.map((v) => ({ rects: v.boxes, kind: v.kind })), safe: SAFE_MARGIN });
  await page.screenshot({ path });
  await page.evaluate(() => document.getElementById('layout-check-overlay')?.remove());
}

export const describeViolation = (v) => (v.b
  ? `${v.kind}: "${v.a.text}" [${v.a.name}] ${v.kind === 'covered' ? 'is covered by' : 'crosses'} "${v.b.text}" [${v.b.name}]`
  : `${v.kind}: "${v.a.text}" [${v.a.name}] ${v.detail}`);
