// usage: node nomic.mjs <base> <out> <pack.json>
// PLAY-F05 defect 3: a fresh page load, a join, hero creation, the story and an open dialogue panel
// make zero microphone-related calls. Exit code 1 on any call.
import { launch, session, click, create, settle, watch } from './lib.mjs';
const [base, out, packPath] = process.argv.slice(2);
const s = session(base, out, packPath);
const browser = await launch();
const counters = `
  window.__mic = { getUserMedia: 0, enumerateDevices: 0, permissionsQuery: 0, mediaRecorder: 0, audioInput: 0 };
  const md = navigator.mediaDevices;
  if (md) {
    const gum = md.getUserMedia && md.getUserMedia.bind(md);
    md.getUserMedia = (...a) => { window.__mic.getUserMedia++; return gum ? gum(...a) : Promise.reject(new Error('none')); };
    const en = md.enumerateDevices && md.enumerateDevices.bind(md);
    md.enumerateDevices = (...a) => { window.__mic.enumerateDevices++; return en ? en(...a) : Promise.resolve([]); };
  }
  if (navigator.permissions && navigator.permissions.query) {
    const q = navigator.permissions.query.bind(navigator.permissions);
    navigator.permissions.query = (...a) => { window.__mic.permissionsQuery++; return q(...a); };
  }
  if (window.MediaRecorder) {
    const R = window.MediaRecorder;
    window.MediaRecorder = function (...a) { window.__mic.mediaRecorder++; return new R(...a); };
    window.MediaRecorder.isTypeSupported = R.isTypeSupported.bind(R);
  }
`;
const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
await ctx.addInitScript(counters);
const page = await ctx.newPage();
watch(s, page, 'player');
const readCounts = () => page.evaluate(() => ({ ...window.__mic }));
const tvPage = await (await browser.newContext({ viewport: { width: 1280, height: 720 } })).newPage();
await tvPage.goto(`${base}/gameplay/display`);
await page.goto(`${base}/gameplay/player`);
await settle(3000);
const afterLoad = await readCounts();
s.note(`after load: ${JSON.stringify(afterLoad)}`);
await create(s, page, 'Brakka', 'Copper beard');
await click(s, page, 'Confirm your hero');
const second = await (await browser.newContext({ viewport: { width: 390, height: 844 }, isMobile: true })).newPage();
await second.goto(`${base}/gameplay/player`);
await create(s, second, 'Thrain', 'Missing ear');
await click(s, second, 'Confirm your hero');
await settle();
await click(s, page, s.label('begin-story'));
await settle(6000);
await page.evaluate(() => { const d = document.querySelector('details.say-more'); if (d) d.open = true; });
await settle(1500);
const panel = await page.locator('.talk-hold').count();
s.note(`talk control present: ${panel}`);
const afterPanel = await readCounts();
s.note(`after dialogue panel open: ${JSON.stringify(afterPanel)}`);
const total = Object.values(afterPanel).reduce((a, b) => a + b, 0);
s.note(total === 0 ? 'PASS no microphone call before a press' : 'FAIL microphone API touched before a press');
await browser.close();
process.exit(total === 0 ? 0 : 1);
