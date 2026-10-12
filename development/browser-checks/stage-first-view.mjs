// PLAY-F06: the first view of the first story scene of a standard generated adventure (fixed seed). Required order on a
// scene's first view: transition, backdrop, cast enters, the scene's own narration (voiced), then the speaker's line,
// then the choice chips. The lobby's text is never the narrator box's text, no bubble opens before a figure stands on
// the stage, and the choice chips are hidden while any line is presenting (they never overlap the narrator box).
// A typed question afterwards must not replay the scene's narration.
// usage: [SEED=0xd5f94172edb1d404] node stage-first-view.mjs [frames-dir]
import fs from 'node:fs';
import path from 'node:path';
import { runner, startServer, launchGpu, playToOpening, click, settle, until, framesDir, root } from './stage.mjs';

const SEED = process.env.SEED || '0xd5f94172edb1d404';
const { s, out, check, finish } = runner('stage-first-view', process.argv[2] || framesDir);
const env = { DF_GAMEPLAY_ADVENTURE: 'random', DF_GAMEPLAY_ADVENTURE_SEED: SEED, DF_GAMEPLAY_ADVENTURE_TABLES: path.join(root, 'content', 'adventure-tables') };
await startServer(env);
const browser = await launchGpu(false);
const { tv, pa } = await playToOpening(s, browser, { base: process.env.DF_BASE, sound: true, voice: { latencyMs: 150, lengthFor: () => 2.5 } });
const UTILITY = ['Full character sheet', 'Ask for clarification', 'Play', 'Hero', 'Menu', 'Reconnect', 'Confirm saved choice', 'Leave this game on this device', 'Close'];
const buttons = (page) => page.evaluate((utility) => [...document.querySelectorAll('button:not([disabled])')].map((b) => b.textContent.trim().replace(/\s+/g, ' ')).filter((t) => t && !utility.includes(t)), UTILITY);
const snapshot = () => tv.evaluate(() => {
  const stage = document.querySelector('.tv-stage');
  const visible = (el) => { if (!el) return false; const cs = getComputedStyle(el); return cs.visibility !== 'hidden' && cs.display !== 'none' && Number(cs.opacity) > 0.3; };
  const dm = document.querySelector('.tv-dm');
  const chips = document.querySelector('.tv-choices');
  const rect = (el) => { if (!el) return null; const r = el.getBoundingClientRect(); return { top: r.top, bottom: r.bottom, left: r.left, right: r.right }; };
  return {
    t: Date.now(), scene: stage?.getAttribute('data-scene-state'), line: stage?.getAttribute('data-line-state'), stage: stage?.getAttribute('data-stage'),
    narrator: dm ? { text: dm.textContent.trim().slice(0, 140), state: dm.getAttribute('data-line-state'), visible: visible(dm), rect: rect(dm) } : null,
    chipsVisible: visible(chips), chipsRect: visible(chips) ? rect(chips) : null,
    figures: [...document.querySelectorAll('.tv-fig')].filter((f) => ['idle', 'speaking', 'entering'].includes(f.getAttribute('data-actor-state'))).length,
    bubbles: [...document.querySelectorAll('.tv-bubble')].filter(visible).map((b) => ({ kind: b.getAttribute('data-kind'), state: b.getAttribute('data-line-state'), text: b.textContent.trim().slice(0, 90) })),
  };
});
const lobby = (await snapshot()).narrator?.text || '';
const lobbyScreen = await tv.evaluate(() => document.body.innerText.replace(/\s+/g, ' ').slice(0, 400));
s.note(`lobby narrator text: ${JSON.stringify(lobby)}; screen: ${lobbyScreen.slice(0, 200)}`);
const begin = (await buttons(pa)).find((t) => /enter|begin|start|step/i.test(t));
s.note(`begin: ${begin}`);
const samples = [];
await click(s, pa, begin, { wait: 100 });
const stop = Date.now() + 90000;
let ready = 0;
while (Date.now() < stop) {
  const now = await snapshot();
  samples.push(now);
  if (now.scene === 'awaiting-choice' && now.chipsVisible && !now.narrator?.visible) ready += 1; else ready = 0;
  if (ready > 15) break;
  await settle(80);
}
await tv.screenshot({ path: `${out}-1-first-scene.png` });
fs.writeFileSync(`${out}-samples.json`, JSON.stringify(samples, null, 1));
const narrations = samples.filter((m) => m.narrator?.visible).map((m) => m.narrator.text).filter((v, i, a) => i === 0 || v !== a[i - 1]);
s.note(`narrator box texts in order: ${JSON.stringify(narrations)}`);
check(narrations.length > 0, 'the scene\'s own narration was shown in the narrator box');
check(!narrations.some((text) => lobby && text.startsWith(lobby.slice(0, 40))), 'the narrator box never showed the lobby text');
check(!narrations.some((text) => /share the room with your companions|bring your heroes into the story/i.test(text)), 'no lobby wording in the narrator box');
const firstBubble = samples.findIndex((m) => m.bubbles.length > 0);
const firstFigure = samples.findIndex((m) => m.figures > 0);
const firstNarration = samples.findIndex((m) => m.narrator?.visible);
check(firstFigure >= 0 && (firstBubble < 0 || firstBubble >= firstFigure), 'no bubble opened before a figure stood on the stage');
check(firstNarration < 0 || firstFigure < 0 || firstFigure <= firstNarration, 'the cast entered before the narration');
check(firstBubble < 0 || firstNarration < 0 || firstNarration <= firstBubble, 'the narration came before the speaker\'s line');
const overlapping = samples.filter((m) => m.chipsVisible && m.narrator?.visible && m.narrator.rect && m.chipsRect && m.narrator.rect.top < m.chipsRect.bottom && m.chipsRect.top < m.narrator.rect.bottom);
check(overlapping.length === 0, `the chips never overlap a visible narrator box (${overlapping.length} samples)`);
const chipsWhilePresenting = samples.filter((m) => m.chipsVisible && ['presenting', 'entering'].includes(m.scene));
check(chipsWhilePresenting.length === 0, `no chips while a line is presenting (${chipsWhilePresenting.length})`);
const last = samples[samples.length - 1];
check(last.scene === 'awaiting-choice' && last.chipsVisible, 'the chips came last');
// A typed question: the NPC answers in a bubble and the scene's narration is not told again.
const told = narrations[0];
await pa.evaluate(() => { const d = document.querySelector('details.say-more'); if (d) d.open = true; });
await pa.selectOption('select[aria-label="Input context"]', { label: 'Conversation' }).catch(() => null);
await pa.fill('textarea.dialogue-draft', 'Who gave you that shield?');
await pa.locator('[data-dialogue=submit]').click();
const after = [];
const stopAsk = Date.now() + 25000;
while (Date.now() < stopAsk) { after.push(await snapshot()); await settle(100); }
const replayed = after.filter((m) => m.narrator?.visible && told && m.narrator.text === told);
const answered = after.some((m) => m.bubbles.some((b) => b.kind === 'speech'));
s.note(`reply bubble seen: ${answered}; narration replayed in ${replayed.length} samples`);
check(replayed.length === 0, 'a reply to table talk did not replay the scene narration');
await finish(browser);
