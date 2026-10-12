// PLAY-F06: ONE real mouse click (pointer down, then up, at the control's coordinates; no synthetic
// element.click()) on the locked "Enable sound" control must leave the display with sound ON; only a later,
// separate click mutes. Also a real key press (Enter) on the focused locked control. Runs in a browser that
// has no autoplay allowance, like a desktop browser. usage: node sound-click.mjs [frames-dir]
import { runner, startServer, settle, framesDir, chromium, exe } from './stage.mjs';

const { s, out, check, finish } = runner('sound-click', process.argv[2] || framesDir);
await startServer();
const browser = await chromium.launch({ headless: true, executablePath: exe });
const base = process.env.DF_BASE;

async function freshDisplay() {
  const page = await (await browser.newContext({ viewport: { width: 1920, height: 1080 } })).newPage();
  await page.goto(`${base}/gameplay/display`);
  const control = page.locator('.tv-voice-control');
  await control.waitFor({ state: 'visible', timeout: 20000 });
  return { page, control };
}
const state = (control) => control.getAttribute('data-voice-state');
const labelOf = (control) => control.getAttribute('aria-label');

// Real mouse: move, press, wait the way a hand does, release.
{
  const { page, control } = await freshDisplay();
  check((await state(control)) === 'locked', 'a new display is locked');
  const box = await control.boundingBox();
  const [x, y] = [box.x + box.width / 2, box.y + box.height / 2];
  await page.mouse.move(x, y);
  await page.mouse.down();
  await settle(90);
  await page.mouse.up();
  await settle(300);
  await page.screenshot({ path: `${out}-1-after-one-click.png` });
  check((await state(control)) === 'on', `one real click on the locked control ends with sound on (${await state(control)}, "${await labelOf(control)}")`);
  await settle(900);
  // The label is gone and the control is smaller: aim at where it is now.
  const small = await control.boundingBox();
  await page.mouse.move(small.x + small.width / 2, small.y + small.height / 2);
  await page.mouse.down();
  await settle(60);
  await page.mouse.up();
  await settle(300);
  check((await state(control)) === 'muted', `a later, separate click mutes (${await state(control)})`);
  await page.mouse.down();
  await settle(60);
  await page.mouse.up();
  await settle(300);
  check((await state(control)) === 'on', `and the next click unmutes (${await state(control)})`);
}

// Real key press on the focused control.
{
  const { page, control } = await freshDisplay();
  await control.focus();
  await page.keyboard.press('Enter');
  await settle(300);
  check((await state(control)) === 'on', `Enter on the focused locked control ends with sound on (${await state(control)})`);
}

// A click elsewhere unlocks without muting, and the control then mutes on its own click.
{
  const { page, control } = await freshDisplay();
  await page.mouse.click(40, 500);
  await settle(900);
  check((await state(control)) === 'on', `a click elsewhere unlocks sound (${await state(control)})`);
  await control.click();
  await settle(300);
  check((await state(control)) === 'muted', `then the control mutes (${await state(control)})`);
}
await finish(browser);
