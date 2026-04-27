const fs = require("fs");
const path = require("path");
const vm = require("vm");
const crypto = require("crypto");

const UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.6 Safari/605.1.15";

function loadSignatureScript() {
  const filePath = path.join(__dirname, "douyin.js");
  const source = fs.readFileSync(filePath, "utf8");
  vm.runInThisContext(source, { filename: "douyin.js" });
  if (typeof generate_a_bogus !== "function") {
    throw new Error("generate_a_bogus 不存在");
  }
}

function loadFrontierSignScript() {
  const filePath = path.join(__dirname, "douyin_frontier_sign.js");
  const source = fs.readFileSync(filePath, "utf8");
  const noop = () => {};
  const sandbox = {
    window: null,
    document: {
      cookie: "",
      referrer: "https://live.douyin.com/",
      hidden: false,
      visibilityState: "visible",
      addEventListener: noop,
      removeEventListener: noop,
    },
    navigator: {
      userAgent: UA,
      language: "zh-CN",
      platform: "MacIntel",
      cookieEnabled: true,
    },
    location: {
      href: "https://live.douyin.com/",
      protocol: "https:",
      host: "live.douyin.com",
      hostname: "live.douyin.com",
      pathname: "/",
      search: "",
      hash: "",
    },
    screen: { width: 1920, height: 1080 },
    history: {},
    localStorage: { getItem() { return null; }, setItem() {}, removeItem() {} },
    sessionStorage: { getItem() { return null; }, setItem() {}, removeItem() {} },
    setTimeout,
    clearTimeout,
    setInterval,
    clearInterval,
    Request,
    Headers,
    URL,
    URLSearchParams,
    TextEncoder,
    TextDecoder,
    atob,
    btoa,
    crypto: globalThis.crypto,
    fetch,
    console: { log: noop, warn: noop, error: noop },
  };
  sandbox.window = sandbox;
  sandbox.self = sandbox;
  sandbox.globalThis = sandbox;
  vm.createContext(sandbox);
  vm.runInContext(source, sandbox, { filename: "douyin_frontier_sign.js" });
  const acrawler = sandbox.byted_acrawler || sandbox.window.byted_acrawler;
  if (!acrawler || typeof acrawler.frontierSign !== "function") {
    throw new Error("frontierSign 不存在");
  }
  return headers => acrawler.frontierSign(headers);
}

function md5Hex(input) {
  return crypto.createHash("md5").update(input).digest("hex");
}

function randomToken(length = 180) {
  const chars =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
  let text = "";
  const bytes = crypto.randomBytes(length);
  for (let index = 0; index < length; index += 1) {
    text += chars[bytes[index] % chars.length];
  }
  return text;
}

function parseSetCookie(headers) {
  if (typeof headers.getSetCookie === "function") {
    return headers.getSetCookie();
  }
  const single = headers.get("set-cookie");
  return single ? [single] : [];
}

async function getTtwid() {
  const response = await fetch("https://live.douyin.com/1", {
    headers: { "User-Agent": UA },
  });
  const cookies = parseSetCookie(response.headers);
  for (const cookie of cookies) {
    const match = cookie.match(/(?:^|[,;]\s*)ttwid=([^;]+)/);
    if (match) {
      return match[1];
    }
  }
  throw new Error("未取得 ttwid");
}

function buildEnterQuery(roomId) {
  return new URLSearchParams({
    aid: "6383",
    app_name: "douyin_web",
    live_id: "1",
    device_platform: "web",
    language: "zh-CN",
    enter_from: "page_refresh",
    cookie_enabled: "true",
    screen_width: "1920",
    screen_height: "1080",
    browser_language: "zh-CN",
    browser_platform: "MacIntel",
    browser_name: "Safari",
    browser_version: "18.6",
    web_rid: roomId,
    enter_source: "",
    is_need_double_stream: "false",
    insert_task_id: "",
    live_reason: "",
  });
}

async function signedFetch(url, referer) {
  const ttwid = await getTtwid();
  const msToken = randomToken();
  const parsed = new URL(url);
  parsed.searchParams.set("msToken", msToken);
  const rawQuery = parsed.searchParams.toString();
  const aBogus = generate_a_bogus(rawQuery, UA);
  parsed.searchParams.set("a_bogus", aBogus);

  const response = await fetch(parsed.toString(), {
    headers: {
      "User-Agent": UA,
      Referer: referer,
      Cookie: `ttwid=${ttwid}`,
    },
  });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }
  return response.json();
}

async function signedFetchWithCookie(url, referer, ttwid) {
  const msToken = randomToken();
  const parsed = new URL(url);
  parsed.searchParams.set("msToken", msToken);
  const rawQuery = parsed.searchParams.toString();
  const aBogus = generate_a_bogus(rawQuery, UA);
  parsed.searchParams.set("a_bogus", aBogus);

  const response = await fetch(parsed.toString(), {
    headers: {
      "User-Agent": UA,
      Referer: referer,
      Cookie: `ttwid=${ttwid}`,
    },
  });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status}`);
  }
  return response.json();
}

function pickFirstUrl(list) {
  return Array.isArray(list) && list.length > 0 ? list[0] : "";
}

function normalizeImageUrl(url) {
  if (!url) {
    return "";
  }
  if (url.startsWith("//")) {
    return `https:${url}`;
  }
  if (url.startsWith("http://")) {
    return `https://${url.slice("http://".length)}`;
  }
  return url;
}

function extractUrls(room) {
  const urls = [];
  try {
    const streamData = room.stream_url?.live_core_sdk_data?.pull_data?.stream_data;
    if (streamData) {
      const parsed = JSON.parse(streamData);
      for (const key of ["origin", "hd", "sd", "ld"]) {
        const flv = parsed?.data?.[key]?.main?.flv;
        if (flv && !urls.includes(flv)) {
          urls.push(flv);
        }
      }
    }
  } catch (_) {}

  if (urls.length === 0) {
    const fallback = room.stream_url?.flv_pull_url || {};
    for (const value of Object.values(fallback)) {
      if (typeof value === "string" && value && !urls.includes(value)) {
        urls.push(value);
      }
    }
  }

  return urls;
}

function parseResponse(roomId, payload) {
  const rooms = payload?.data?.data || [];
  const room = rooms[0] || {};
  const user = payload?.data?.user || {};
  const owner = room.owner || {};
  const isOnline = Number(room.status) === 2;
  const streamerName = owner.nickname || user.nickname || "";
  const avatarUrl = normalizeImageUrl(
    pickFirstUrl(owner.avatar_thumb?.url_list) ||
      pickFirstUrl(user.avatar_thumb?.url_list)
  );
  const screenshotUrl = normalizeImageUrl(pickFirstUrl(room.cover?.url_list));
  const urls = extractUrls(room);

  return {
    room_id: room.id_str || payload?.data?.enter_room_id || roomId,
    streamer_name: streamerName,
    room_name: room.title || "",
    avatar_url: avatarUrl,
    screenshot_url: isOnline ? screenshotUrl : "",
    is_online: isOnline,
    heat_text: isOnline ? String(room.user_count_str || "") : "",
    page_url: `https://live.douyin.com/${roomId}`,
    title: room.title || "",
    urls,
  };
}

function buildDanmakuSignatureInput(roomId) {
  return Buffer.from(
    "bGl2ZV9pZD0xLGFpZD02MzgzLHZlcnNpb25fY29kZT0xODA4MDAsd2ViY2FzdF9zZGtfdmVyc2lvbj0xLjMuMCxyb29tX2lkPQ==",
    "base64",
  ).toString("utf8")
    + roomId
    + Buffer.from(
      "LHN1Yl9yb29tX2lkPSxzdWJfY2hhbm5lbF9pZD0sZGlkX3J1bGU9Myx1c2VyX3VuaXF1ZV9pZD0sZGV2aWNlX3BsYXRmb3JtPXdlYixkZXZpY2VfdHlwZT0sYWM9LGlkZW50aXR5PWF1ZGllbmNl",
      "base64",
    ).toString("utf8");
}

function buildDanmakuWsUrl(roomId, signature) {
  const params = new URLSearchParams({
    app_name: "douyin_web",
    version_code: "180800",
    webcast_sdk_version: "1.3.0",
    update_version_code: "1.3.0",
    compress: "gzip",
    host: "https://live.douyin.com",
    aid: "6383",
    live_id: "1",
    did_rule: "3",
    debug: "true",
    endpoint: "live_pc",
    support_wrds: "1",
    im_path: "/webcast/im/fetch/",
    device_platform: "web",
    cookie_enabled: "true",
    browser_language: "en-US",
    browser_platform: "MacIntel",
    browser_online: "true",
    tz_name: "Asia/Shanghai",
    identity: "audience",
    heartbeatDuration: "10000",
    room_id: roomId,
    signature,
  });
  return `wss://webcast3-ws-web-hl.douyin.com/webcast/im/push/v2/?${params.toString()}`;
}

async function prepareDanmaku(roomId) {
  loadSignatureScript();
  const frontierSign = loadFrontierSignScript();
  const ttwid = await getTtwid();
  const payload = await signedFetchWithCookie(
    `https://live.douyin.com/webcast/room/web/enter/?${buildEnterQuery(roomId).toString()}`,
    `https://live.douyin.com/${roomId}`,
    ttwid,
  );
  const parsed = parseResponse(roomId, payload);
  const actualRoomId = parsed.room_id || roomId;
  const signed = frontierSign({ "X-MS-STUB": md5Hex(buildDanmakuSignatureInput(actualRoomId)) });
  const signature = Object.values(signed || {})[0];
  if (!signature) {
    throw new Error("生成抖音弹幕签名失败");
  }

  return {
    room_id: actualRoomId,
    cookie: `ttwid=${ttwid}`,
    user_agent: UA,
    referer: "https://live.douyin.com",
    ws_url: buildDanmakuWsUrl(actualRoomId, signature),
  };
}

async function main() {
  const danmakuMode = process.argv[2] === "--danmaku";
  const roomId = (danmakuMode ? process.argv[3] : process.argv[2] || "").trim();
  if (!roomId) {
    throw new Error("缺少抖音房间号");
  }

  if (danmakuMode) {
    process.stdout.write(JSON.stringify(await prepareDanmaku(roomId)));
    process.exit(0);
  }

  loadSignatureScript();
  const query = buildEnterQuery(roomId);
  const payload = await signedFetch(
    `https://live.douyin.com/webcast/room/web/enter/?${query.toString()}`,
    `https://live.douyin.com/${roomId}`
  );
  process.stdout.write(JSON.stringify(parseResponse(roomId, payload)));
  process.exit(0);
}

main().catch(error => {
  process.stderr.write(String(error?.message || error));
  process.exit(1);
});
