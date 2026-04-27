const { overlay, mpv, event } = iina;
const { core } = iina;

let overlayReady = false;
let currentRoomID = "";
let currentEnabled = false;
let currentPort = 19080;

function parseScriptOptions() {
  const raw = mpv.getString("options/script-opts") || "";
  return raw.split(",").reduce((result, item) => {
    if (!item) {
      return result;
    }
    const [key, ...rest] = item.split("=");
    if (!key || rest.length === 0) {
      return result;
    }
    result[key.trim()] = rest.join("=").trim();
    return result;
  }, {});
}

function decodeHexJson(hex) {
  if (!hex || !/^[0-9a-f]+$/i.test(hex) || hex.length % 2 !== 0) {
    return null;
  }

  try {
    let text = "";
    for (let index = 0; index < hex.length; index += 2) {
      text += String.fromCharCode(Number.parseInt(hex.slice(index, index + 2), 16));
    }
    return JSON.parse(text);
  } catch {
    return null;
  }
}

function getIinaPlusArgs() {
  const options = parseScriptOptions();
  return decodeHexJson(options.iinaPlusArgs || "");
}

function inferPlatform(target) {
  const value = String(target || "").toLowerCase();
  if (value.includes("live.bilibili.com")) return "bilibili_live";
  if (value.includes("huya.com")) return "huya";
  if (value.includes("live.douyin.com") || value.includes("v.douyin.com")) return "douyin_live";
  return "douyu";
}

function extractNumericId(target) {
  const value = String(target || "").trim();
  if (/^\d+$/.test(value)) return value;
  const segment = value.split("#")[0].split("?")[0].replace(/\/+$/, "").split("/").pop() || "";
  return /^\d+$/.test(segment) ? segment : "";
}

function getDanmakuTarget() {
  const options = parseScriptOptions();
  const args = getIinaPlusArgs();
  if (options.streamhub_target) {
    return options.streamhub_target;
  }
  if (args && args.rawUrl) {
    return args.rawUrl;
  }

  const path = mpv.getString("path") || "";
  const match = path.match(/stream-hub-douyu-(\d+)\.m3u/i);
  return match ? match[1] : "";
}

function getRoomID() {
  return extractNumericId(getDanmakuTarget());
}

function getPlatform() {
  const options = parseScriptOptions();
  return options.streamhub_platform || inferPlatform(getDanmakuTarget());
}

function getDanmakuPort() {
  const options = parseScriptOptions();
  const args = getIinaPlusArgs();
  const raw = Number.parseInt(options.streamhub_port || String(args?.port || "19080"), 10);
  return Number.isFinite(raw) && raw > 0 ? raw : 19080;
}

function isDanmakuEnabled() {
  const options = parseScriptOptions();
  return Boolean(getIinaPlusArgs()) || (options.streamhub_enabled || "").toLowerCase() === "yes";
}

function syncOverlay() {
  currentRoomID = getRoomID();
  currentPort = getDanmakuPort();
  currentEnabled = isDanmakuEnabled() && currentRoomID !== "";

  if (!currentEnabled) {
    overlay.hide();
    if (overlayReady) {
      overlay.postMessage("stop", {});
    }
    return;
  }

  overlay.show();
  if (overlayReady) {
    overlay.postMessage("configure", { roomID: currentRoomID, platform: getPlatform(), port: currentPort });
  }
}

overlay.onMessage("ready", () => {
  overlayReady = true;
  core.osd("Stream Hub 弹幕插件已加载");
  syncOverlay();
});

overlay.loadFile("overlay.html");

event.on("mpv.file-loaded", () => {
  core.osd("Stream Hub 正在尝试连接弹幕");
  syncOverlay();
});

event.on("mpv.start-file", () => {
  syncOverlay();
});

event.on("mpv.end-file", () => {
  if (overlayReady) {
    overlay.postMessage("stop", {});
  }
  overlay.hide();
});
