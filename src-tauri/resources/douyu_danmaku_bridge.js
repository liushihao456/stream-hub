#!/usr/bin/env node

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const zlib = require("node:zlib");

function emit(event) {
  process.stdout.write(`${JSON.stringify(event)}\n`);
}

function md5Hex(input) {
  return crypto.createHash("md5").update(input).digest("hex");
}

function inferPlatform(input) {
  const value = String(input || "").trim().toLowerCase();
  if (value === "bilibili_live" || value.includes("live.bilibili.com")) {
    return "bilibili_live";
  }
  if (value === "huya" || value.includes("huya.com")) {
    return "huya";
  }
  if (value === "douyin_live" || value.includes("live.douyin.com") || value.includes("v.douyin.com")) {
    return "douyin_live";
  }
  return "douyu";
}

function extractNumericId(input) {
  const value = String(input || "").trim();
  if (/^\d+$/.test(value)) {
    return value;
  }
  const withoutFragment = value.split("#")[0];
  const withoutQuery = withoutFragment.split("?")[0];
  const segment = withoutQuery.replace(/\/+$/, "").split("/").pop() || "";
  return /^\d+$/.test(segment) ? segment : "";
}

function packDouyuMessage(message) {
  const body = Buffer.concat([Buffer.from(message, "utf8"), Buffer.from([0])]);
  const length = body.length + 8;
  const header = Buffer.alloc(12);
  header.writeUInt32LE(length, 0);
  header.writeUInt32LE(length, 4);
  header.writeUInt16LE(689, 8);
  header.writeUInt16LE(0, 10);
  return Buffer.concat([header, body]);
}

function parseDouyuMessages(chunk, buffered) {
  const nextBuffered = Buffer.concat([buffered, chunk]);
  const messages = [];
  let cursor = 0;

  while (cursor + 12 <= nextBuffered.length) {
    const packetLength = nextBuffered.readUInt32LE(cursor);
    const totalLength = packetLength + 4;
    if (cursor + totalLength > nextBuffered.length || totalLength <= 12) {
      break;
    }

    const payload = nextBuffered
      .subarray(cursor + 12, cursor + totalLength)
      .toString("utf8")
      .replace(/\0+$/, "");

    if (payload) {
      messages.push(payload);
    }
    cursor += totalLength;
  }

  return {
    messages,
    buffered: nextBuffered.subarray(cursor),
  };
}

function decodeDouyuText(text) {
  return text.replace(/@S/g, "/").replace(/@A/g, "@");
}

function extractDouyuChatText(message) {
  if (!message.startsWith("type@=chatmsg")) {
    return "";
  }

  const field = message.split("/").find(item => item.startsWith("txt@="));
  return field ? decodeDouyuText(field.slice(5)).trim() : "";
}

function packBilibiliPacket(payload, operation) {
  const body = Buffer.isBuffer(payload) ? payload : Buffer.from(payload, "utf8");
  const packetLength = body.length + 16;
  const header = Buffer.alloc(16);
  header.writeUInt32BE(packetLength, 0);
  header.writeUInt16BE(16, 4);
  header.writeUInt16BE(1, 6);
  header.writeUInt32BE(operation, 8);
  header.writeUInt32BE(1, 12);
  return Buffer.concat([header, body]);
}

function decodeBilibiliTextPacket(body) {
  try {
    return JSON.parse(body.toString("utf8"));
  } catch {
    return null;
  }
}

function parseBilibiliPackets(buffer) {
  const messages = [];
  let offset = 0;

  while (offset + 16 <= buffer.length) {
    const packetLength = buffer.readUInt32BE(offset);
    const headerLength = buffer.readUInt16BE(offset + 4);
    const version = buffer.readUInt16BE(offset + 6);
    const operation = buffer.readUInt32BE(offset + 8);

    if (packetLength <= 0 || offset + packetLength > buffer.length) {
      break;
    }

    const body = buffer.subarray(offset + headerLength, offset + packetLength);

    if (operation === 5) {
      if (version === 2) {
        try {
          const inflated = zlib.inflateSync(body);
          messages.push(...parseBilibiliPackets(inflated));
        } catch {}
      } else if (version === 3) {
        try {
          const inflated = zlib.brotliDecompressSync(body);
          messages.push(...parseBilibiliPackets(inflated));
        } catch {}
      } else if (version === 0 || version === 1) {
        const parsed = decodeBilibiliTextPacket(body);
        if (parsed) {
          messages.push(parsed);
        }
      }
    } else if (operation === 8) {
      messages.push({ __type: "connected" });
    } else if (operation === 3) {
      messages.push({ __type: "heartbeat" });
    }

    offset += packetLength;
  }

  return messages;
}

function extractBilibiliChatText(message) {
  if (!message || typeof message !== "object") {
    return "";
  }

  const cmd = String(message.cmd || "");
  if (cmd.startsWith("DANMU_MSG")) {
    return typeof message.info?.[1] === "string" ? message.info[1].trim() : "";
  }

  if (cmd === "LIVE_INTERACTIVE_GAME") {
    return String(message.data?.msg || "").trim();
  }

  return "";
}

async function fetchJson(url, options = {}) {
  const response = await fetch(url, options);
  if (!response.ok) {
    throw new Error(`HTTP ${response.status} for ${url}`);
  }
  return response.json();
}

async function fetchText(url, options = {}) {
  const response = await fetch(url, options);
  if (!response.ok) {
    throw new Error(`HTTP ${response.status} for ${url}`);
  }
  return response.text();
}

const BILI_WBI_MIXIN_KEY_ENC_TAB = [
  46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49,
  33, 9, 42, 19, 29, 28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40,
  61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25, 54, 21, 56, 59, 6, 63, 57, 62, 11,
  36, 20, 34, 44, 52,
];

function getBiliMixinKey(imgKey, subKey) {
  const origin = `${imgKey}${subKey}`;
  return BILI_WBI_MIXIN_KEY_ENC_TAB.map(index => origin[index]).join("").slice(0, 32);
}

function signBiliParams(params, imgKey, subKey) {
  const mixinKey = getBiliMixinKey(imgKey, subKey);
  const payload = { ...params, wts: Math.floor(Date.now() / 1000).toString() };
  const items = Object.entries(payload).sort(([a], [b]) => a.localeCompare(b));
  const query = items
    .map(([key, value]) => `${key}=${String(value).split("").filter(ch => !"!'()*".includes(ch)).join("")}`)
    .join("&");
  payload.w_rid = md5Hex(`${query}${mixinKey}`);
  return payload;
}

async function connectDouyu(roomId) {
  let socket;
  try {
    socket = new WebSocket("wss://danmuproxy.douyu.com:8506");
  } catch (error) {
    emit({ type: "error", text: String(error) });
    process.exit(1);
  }

  let buffered = Buffer.alloc(0);
  let heartbeat = null;
  let connected = false;

  function cleanup() {
    if (heartbeat) {
      clearInterval(heartbeat);
      heartbeat = null;
    }
  }

  socket.binaryType = "arraybuffer";

  socket.onopen = () => {
    socket.send(packDouyuMessage(`type@=loginreq/roomid@=${roomId}/`));
    socket.send(packDouyuMessage(`type@=joingroup/rid@=${roomId}/gid@=-9999/`));
    heartbeat = setInterval(() => {
      socket.send(packDouyuMessage("type@=mrkl/"));
    }, 30000);
  };

  socket.onmessage = event => {
    const parsed = parseDouyuMessages(Buffer.from(event.data), buffered);
    buffered = parsed.buffered;

    for (const message of parsed.messages) {
      if (message.startsWith("type@=pingreq")) {
        socket.send(packDouyuMessage("type@=mrkl/"));
        continue;
      }

      if (message.startsWith("type@=loginres") && !connected) {
        connected = true;
        emit({ type: "status", text: "Stream Hub 弹幕已连接" });
        continue;
      }

      const text = extractDouyuChatText(message);
      if (text) {
        emit({ type: "chat", text });
      }
    }
  };

  socket.onerror = error => {
    emit({ type: "error", text: error?.message || "斗鱼弹幕连接失败" });
    cleanup();
    process.exit(1);
  };

  socket.onclose = event => {
    if (!connected) {
      emit({ type: "error", text: `斗鱼弹幕连接关闭 (${event.code})` });
    }
    cleanup();
    process.exit(0);
  };
}

async function connectBilibili(rawTarget) {
  const roomInput = extractNumericId(rawTarget);
  if (!roomInput) {
    throw new Error("缺少 B 站直播间房间号");
  }

  const ua =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";

  const roomInfo = await fetchJson(
    `https://api.live.bilibili.com/room/v1/Room/get_info?room_id=${encodeURIComponent(roomInput)}`,
    { headers: { "User-Agent": ua, Referer: `https://live.bilibili.com/${roomInput}` } },
  );
  const roomId = String(roomInfo?.data?.room_id || roomInput);

  const nav = await fetchJson("https://api.bilibili.com/x/web-interface/nav", {
    headers: { "User-Agent": ua, Referer: "https://www.bilibili.com/" },
  });
  const imgKey = String(nav?.data?.wbi_img?.img_url || "").split("/").pop()?.split(".")[0] || "";
  const subKey = String(nav?.data?.wbi_img?.sub_url || "").split("/").pop()?.split(".")[0] || "";
  const signedParams = signBiliParams(
    { id: roomId, type: "0", web_location: "444.8" },
    imgKey,
    subKey,
  );
  const dmInfoUrl = new URL("https://api.live.bilibili.com/xlive/web-room/v1/index/getDanmuInfo");
  Object.entries(signedParams).forEach(([key, value]) => dmInfoUrl.searchParams.set(key, String(value)));
  const danmuInfo = await fetchJson(dmInfoUrl.toString(), {
    headers: { "User-Agent": ua, Referer: `https://live.bilibili.com/${roomId}` },
  });
  const token = String(danmuInfo?.data?.token || "");
  const uid = Number(nav?.data?.mid || 0);

  let socket;
  try {
    socket = new WebSocket("wss://broadcastlv.chat.bilibili.com:443/sub");
  } catch (error) {
    emit({ type: "error", text: String(error) });
    process.exit(1);
  }

  let heartbeat = null;
  let connected = false;
  socket.binaryType = "arraybuffer";

  function cleanup() {
    if (heartbeat) {
      clearInterval(heartbeat);
      heartbeat = null;
    }
  }

  socket.onopen = () => {
    const authPayload = JSON.stringify({
      uid,
      roomid: Number(roomId),
      protover: 2,
      buvid: `${crypto.randomUUID()}${Math.floor(10000 + Math.random() * 80000)}infoc`,
      platform: "web",
      type: 2,
      key: token,
    });
    socket.send(packBilibiliPacket(authPayload, 7));
    heartbeat = setInterval(() => {
      socket.send(packBilibiliPacket(Buffer.alloc(0), 2));
    }, 30000);
  };

  socket.onmessage = event => {
    for (const message of parseBilibiliPackets(Buffer.from(event.data))) {
      if (message?.__type === "connected" && !connected) {
        connected = true;
        emit({ type: "status", text: "Stream Hub 弹幕已连接" });
        continue;
      }

      const text = extractBilibiliChatText(message);
      if (text) {
        emit({ type: "chat", text });
      }
    }
  };

  socket.onerror = error => {
    emit({ type: "error", text: error?.message || "B站弹幕连接失败" });
    cleanup();
    process.exit(1);
  };

  socket.onclose = event => {
    if (!connected) {
      emit({ type: "error", text: `B站弹幕连接关闭 (${event.code})` });
    }
    cleanup();
    process.exit(0);
  };
}

const HUYA_BLOCK_LIST = [
  "分享了直播间，房间号",
  "录制并分享了小视频",
  "进入直播间",
  "刚刚在打赏君活动中",
  "竟然抽出了",
  "车队召集令在此",
  "微信公众号“虎牙志愿者”",
];

let huyaScriptLoaded = false;

function loadHuyaScript() {
  if (huyaScriptLoaded) {
    return;
  }

  const source = fs.readFileSync(path.join(__dirname, "huya.js"), "utf8");
  vm.runInThisContext(source, { filename: "huya.js" });
  if (
    typeof sendRegisterGroups !== "function" ||
    typeof sendHeartBeat !== "function" ||
    typeof test !== "function"
  ) {
    throw new Error("虎牙弹幕编解码脚本加载失败");
  }
  huyaScriptLoaded = true;
}

function parseHuyaAnchorUidFromHtml(html) {
  const match = String(html).match(/var\s+TT_ROOM_DATA\s*=\s*(\{.*?\});/s);
  if (!match) {
    return "";
  }

  try {
    const data = JSON.parse(match[1]);
    return String(data.id || data.uid || "");
  } catch {
    return "";
  }
}

async function resolveHuyaAnchorUid(rawTarget) {
  const target = String(rawTarget || "").trim();
  const roomId = extractNumericId(target);
  const pageUrl = target.startsWith("http://") || target.startsWith("https://")
    ? target
    : `https://www.huya.com/${encodeURIComponent(roomId || target)}`;
  const headers = {
    "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15",
    Referer: "https://www.huya.com/",
  };

  try {
    const html = await fetchText(pageUrl, { headers });
    const uid = parseHuyaAnchorUidFromHtml(html);
    if (/^\d+$/.test(uid)) {
      return uid;
    }
  } catch {}

  if (!roomId) {
    return "";
  }

  try {
    const payload = await fetchJson(
      `https://mp.huya.com/cache.php?m=Live&do=profileRoom&roomid=${encodeURIComponent(roomId)}`,
      { headers },
    );
    const uid = String(payload?.data?.liveData?.uid || payload?.data?.profileInfo?.uid || "");
    return /^\d+$/.test(uid) ? uid : "";
  } catch {
    return "";
  }
}

async function connectHuya(rawTarget) {
  loadHuyaScript();

  const anchorUid = await resolveHuyaAnchorUid(rawTarget);
  if (!anchorUid) {
    throw new Error("无法识别虎牙主播 UID");
  }

  let socket;
  try {
    socket = new WebSocket("wss://wsapi.huya.com");
  } catch (error) {
    emit({ type: "error", text: String(error) });
    process.exit(1);
  }

  let heartbeat = null;
  let connected = false;
  socket.binaryType = "arraybuffer";

  function cleanup() {
    if (heartbeat) {
      clearInterval(heartbeat);
      heartbeat = null;
    }
  }

  socket.onopen = () => {
    socket.send(new Uint8Array(sendRegisterGroups([`live:${anchorUid}`, `chat:${anchorUid}`])));
    heartbeat = setInterval(() => {
      socket.send(new Uint8Array(sendHeartBeat()));
    }, 30000);
  };

  socket.onmessage = event => {
    let result = "";
    try {
      result = String(test(Array.from(Buffer.from(event.data))));
    } catch {
      return;
    }

    if (result === "EWebSocketCommandType.EWSCmdS2C_RegisterGroupRsp") {
      if (!connected) {
        connected = true;
        emit({ type: "status", text: "Stream Hub 弹幕已连接" });
      }
      return;
    }

    if (result.startsWith("EWebSocketCommandType")) {
      return;
    }

    let message;
    try {
      message = JSON.parse(result);
    } catch {
      return;
    }

    const text = String(message?.sMsg || "").trim();
    if (
      Number(message?.ePushType) === 5 &&
      Number(message?.iUri) === 1400 &&
      Number(message?.iProtocolType) === 2 &&
      text &&
      !HUYA_BLOCK_LIST.some(item => text.includes(item))
    ) {
      emit({ type: "chat", text });
    }
  };

  socket.onerror = error => {
    emit({ type: "error", text: error?.message || "虎牙弹幕连接失败" });
    cleanup();
    process.exit(1);
  };

  socket.onclose = event => {
    if (!connected) {
      emit({ type: "error", text: `虎牙弹幕连接关闭 (${event.code})` });
    }
    cleanup();
    process.exit(0);
  };
}

async function main() {
  const rawPlatform = (process.argv[2] || "").trim();
  const rawTarget = (process.argv[3] || process.argv[2] || "").trim();
  if (!rawTarget) {
    emit({ type: "error", text: "缺少房间号" });
    process.exit(1);
  }

  const platform = inferPlatform(rawPlatform || rawTarget);
  if (platform === "bilibili_live") {
    await connectBilibili(rawTarget);
  } else if (platform === "huya") {
    await connectHuya(rawTarget);
  } else if (platform === "douyin_live") {
    throw new Error("抖音弹幕由 Stream Hub 后端 WebSocket 处理");
  } else {
    await connectDouyu(extractNumericId(rawTarget));
  }
}

main().catch(error => {
  emit({ type: "error", text: String(error) });
  process.exit(1);
});
