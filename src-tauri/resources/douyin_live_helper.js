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

async function main() {
  const roomId = (process.argv[2] || "").trim();
  if (!roomId) {
    throw new Error("缺少抖音房间号");
  }
  loadSignatureScript();
  const query = buildEnterQuery(roomId);
  const payload = await signedFetch(
    `https://live.douyin.com/webcast/room/web/enter/?${query.toString()}`,
    `https://live.douyin.com/${roomId}`
  );
  process.stdout.write(JSON.stringify(parseResponse(roomId, payload)));
}

main().catch(error => {
  process.stderr.write(String(error?.message || error));
  process.exit(1);
});
