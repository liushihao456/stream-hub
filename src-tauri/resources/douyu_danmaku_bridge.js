#!/usr/bin/env node

function emit(event) {
  process.stdout.write(`${JSON.stringify(event)}\n`);
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

function extractChatText(message) {
  if (!message.startsWith("type@=chatmsg")) {
    return "";
  }

  const field = message.split("/").find(item => item.startsWith("txt@="));
  return field ? decodeDouyuText(field.slice(5)).trim() : "";
}

async function main() {
  const roomId = (process.argv[2] || "").trim();
  if (!roomId) {
    emit({ type: "error", text: "缺少房间号" });
    process.exit(1);
  }

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

      const text = extractChatText(message);
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

main().catch(error => {
  emit({ type: "error", text: String(error) });
  process.exit(1);
});
