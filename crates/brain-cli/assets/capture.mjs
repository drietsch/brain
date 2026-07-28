// Screenshots + typed screencast of the live graph tour, via Playwright's
// bundled Chromium. Input: <out>/.sections.json, written by `brain docs
// generate` just before this runs.
// Output: <out>/img/<id>.png per section, <out>/tour.webm screencast.
//
// Every step is optional: the screenshots are worth having even when the
// screencast fails, so failures are reported and stepped over rather than
// thrown. The exit code is non-zero only when nothing at all was captured,
// which is the signal `brain docs generate` prints its skip message for.
import { readFileSync, renameSync, readdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { createRequire } from 'node:module';

// ESM import ignores NODE_PATH; require() honors it, which lets the global
// playwright install (next to the pre-provisioned browsers) resolve.
const require = createRequire(import.meta.url);

let chromium;
try {
  ({ chromium } = require('playwright'));
} catch (e) {
  console.error(`capture: playwright is not installed (${e.message})`);
  process.exit(1);
}

const [out] = process.argv.slice(2);
if (!out) {
  console.error('capture: usage: capture.mjs <out-dir>');
  process.exit(1);
}

let sections;
try {
  sections = JSON.parse(readFileSync(join(out, '.sections.json'), 'utf8'));
} catch (e) {
  console.error(`capture: cannot read the section list (${e.message})`);
  process.exit(1);
}

const esc = (s) =>
  s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

const style = `
  body { margin: 0; background: #0d1117; }
  .term { padding: 24px 28px; font: 15px/1.45 "DejaVu Sans Mono", monospace;
          color: #c9d1d9; white-space: pre-wrap; word-break: break-all; }
  .prompt { color: #7ee787; font-weight: bold; }
  .cmd { color: #e6edf3; }
  .cursor { display: inline-block; width: 8px; height: 16px;
            background: #c9d1d9; vertical-align: text-bottom; }
`;

const page_html = (body) =>
  `<!doctype html><meta charset="utf-8"><style>${style}</style><div class="term">${body}</div>`;

let browser;
try {
  browser = await chromium.launch();
} catch (e) {
  console.error(`capture: cannot launch chromium (${e.message})`);
  console.error('capture: try `npx playwright install chromium`');
  process.exit(1);
}

let shots = 0;
let screencast = false;

// ---- screenshots: one styled terminal frame per section ----------------
try {
  const page = await browser.newPage({ viewport: { width: 1100, height: 600 } });
  for (const s of sections) {
    const body = `<span class="prompt">$</span> <span class="cmd">${esc(s.cmd)}</span>\n${esc(s.text)}`;
    await page.setContent(page_html(body));
    await page.screenshot({ path: join(out, 'img', `${s.id}.png`), fullPage: true });
    shots += 1;
  }
  await page.close();
} catch (e) {
  console.error(`capture: screenshots stopped after ${shots} (${e.message})`);
}

// ---- screencast: type each command, reveal its output ------------------
try {
  const videoDir = join(out, '.video');
  const context = await browser.newContext({
    viewport: { width: 1280, height: 720 },
    recordVideo: { dir: videoDir, size: { width: 1280, height: 720 } },
  });
  const page = await context.newPage();
  await page.setContent(page_html('<span id="screen"></span><span class="cursor"></span>'));
  await page.evaluate(async (sections) => {
    const screen = document.getElementById('screen');
    const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
    const append = (html) => {
      screen.insertAdjacentHTML('beforeend', html);
      window.scrollTo(0, document.body.scrollHeight);
    };
    for (const s of sections) {
      append('<span class="prompt">$ </span>');
      for (const ch of s.cmd) {
        append(`<span class="cmd">${ch.replace('<', '&lt;')}</span>`);
        await sleep(45);
      }
      await sleep(350);
      const text = s.text.length > 1800 ? s.text.slice(0, 1800) + '\n…' : s.text;
      append(`\n${text.replace(/&/g, '&amp;').replace(/</g, '&lt;')}\n\n`);
      await sleep(2600);
    }
    await sleep(800);
  }, sections);
  await context.close(); // flushes the video

  const video = readdirSync(videoDir).find((f) => f.endsWith('.webm'));
  if (!video) throw new Error('chromium wrote no video file');
  renameSync(join(videoDir, video), join(out, 'tour.webm'));
  rmSync(videoDir, { recursive: true, force: true });
  screencast = true;
} catch (e) {
  console.error(`capture: screencast skipped (${e.message})`);
  rmSync(join(out, '.video'), { recursive: true, force: true });
}

await browser.close().catch(() => {});

if (shots === 0 && !screencast) {
  console.error('capture: nothing was captured');
  process.exit(1);
}
console.log(
  `captured ${shots} screenshot(s)${screencast ? ' + screencast' : ' (no screencast)'} into ${out}`,
);
