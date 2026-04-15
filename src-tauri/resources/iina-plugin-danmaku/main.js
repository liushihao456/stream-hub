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

function getRoomID() {
  const options = parseScriptOptions();
  if (options.streamhub_target) {
    return options.streamhub_target;
  }

  const path = mpv.getString("path") || "";
  const match = path.match(/stream-hub-douyu-(\d+)\.m3u/i);
  return match ? match[1] : "";
}

function getDanmakuPort() {
  const options = parseScriptOptions();
  const raw = Number.parseInt(options.streamhub_port || "19080", 10);
  return Number.isFinite(raw) && raw > 0 ? raw : 19080;
}

function isDanmakuEnabled() {
  const options = parseScriptOptions();
  return (options.streamhub_enabled || "").toLowerCase() === "yes";
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
    overlay.postMessage("configure", { roomID: currentRoomID, port: currentPort });
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
