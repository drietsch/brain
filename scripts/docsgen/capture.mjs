// Screenshots + typed screencast of the live graph tour, via Playwright's
// bundled Chromium. Input: <out>/.sections.json written by generate.sh.
// Output: <out>/img/<id>.png per section, <out>/tour.webm screencast.
//
// Run with NODE_PATH pointing at a global playwright install if it is not
// resolvable locally (generate.sh does this).
import { readFileSync, renameSync, readdirSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { createRequire } from 'node:module';

// ESM import ignores NODE_PATH; require() honors it, which lets the global
// playwright install (next to the pre-provisioned browsers) resolve.
const require = createRequire(import.meta.url);
const { chromium } = require('playwright');

const [out] = process.argv.slice(2);
const sections = JSON.parse(readFileSync(join(out, '.sections.json'), 'utf8'));

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

const browser = await chromium.launch();

// ---- screenshots: one styled terminal frame per section ----------------
{
  const page = await browser.newPage({ viewport: { width: 1100, height: 600 } });
  for (const s of sections) {
    const body = `<span class="prompt">$</span> <span class="cmd">${esc(s.cmd)}</span>\n${esc(s.text)}`;
    await page.setContent(page_html(body));
    await page.screenshot({ path: join(out, 'img', `${s.id}.png`), fullPage: true });
  }
  await page.close();
}

// ---- screencast: type each command, reveal its output ------------------
{
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
  renameSync(join(videoDir, video), join(out, 'tour.webm'));
  rmSync(videoDir, { recursive: true, force: true });
}

await browser.close();
console.log(`captured ${sections.length} screenshots + screencast into ${out}`);
