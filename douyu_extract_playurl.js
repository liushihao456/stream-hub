const { chromium } = require('playwright');

async function main() {
  const room = process.argv[2];
  if (!room) {
    console.error('Usage: node douyu_extract_playurl.js <room-id-or-url>');
    process.exit(1);
  }
  const url = /^https?:\/\//.test(room) ? room : `https://www.douyu.com/${room}`;
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({
    viewport: { width: 1280, height: 720 },
    userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36',
  });
  const page = await context.newPage();
  await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 60000 });
  await page.waitForTimeout(5000);

  const result = await page.evaluate(async () => {
    function getUrl() {
      const logs = window.__playLog?.logs || [];
      for (let i = logs.length - 1; i >= 0; i--) {
        const line = logs[i];
        const m = line.match(/https?:[^\s\"]+\.(?:flv|m3u8)\?[^\s\"]*/i);
        if (m) return { url: m[0], log: line };
        if (/主播未开播/.test(line)) return { offline: true, log: line };
      }
    }
    let found = getUrl();
    const deadline = Date.now() + 20000;
    while (!found && Date.now() < deadline) {
      await new Promise(r => setTimeout(r, 1000));
      found = getUrl();
    }
    return found || { error: 'No stream URL found in __playLog', logs: (window.__playLog?.logs || []).slice(-20) };
  });

  console.log(JSON.stringify(result, null, 2));
  await browser.close();
}

main().catch(err => {
  console.error(err);
  process.exit(1);
});
