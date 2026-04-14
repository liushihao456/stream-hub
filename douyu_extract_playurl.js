const crypto = require('crypto');

const USER_AGENT =
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36';

function md5(input) {
  return crypto.createHash('md5').update(input).digest('hex');
}

function randomDid() {
  return md5(`${Math.random()}`);
}

function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

async function fetchText(url, init = {}) {
  const response = await fetch(url, {
    ...init,
    headers: {
      'user-agent': USER_AGENT,
      ...(init.headers || {}),
    },
  });

  if (!response.ok) {
    throw new Error(`HTTP ${response.status} for ${url}`);
  }

  return response.text();
}

async function fetchJson(url, init = {}) {
  const text = await fetchText(url, init);
  return JSON.parse(text);
}

function normalizeRoomInput(input) {
  if (/^https?:\/\//.test(input)) {
    return input;
  }
  return `https://www.douyu.com/${input}`;
}

function extractRoomInfoJSON(html) {
  const marker = '\\"roomInfo\\"';
  const markerIndex = html.indexOf(marker);
  if (markerIndex === -1) {
    return null;
  }

  const suffix = html.slice(markerIndex + marker.length);
  const openOffset = suffix.indexOf('{');
  if (openOffset === -1) {
    return null;
  }

  const start = markerIndex + marker.length + openOffset;
  let depth = 0;
  for (let i = start; i < html.length; i += 1) {
    const char = html[i];
    if (char === '{') {
      depth += 1;
    } else if (char === '}') {
      depth -= 1;
      if (depth === 0) {
        return html
          .slice(start, i + 1)
          .replace(/\\"/g, '"')
          .replace(/\\"/g, '"');
      }
    }
  }

  return null;
}

function extractRoomInfo(html) {
  const roomInfoJson = extractRoomInfoJSON(html);
  if (!roomInfoJson) {
    throw new Error('Failed to extract roomInfo JSON from Douyu page');
  }

  const parsed = JSON.parse(roomInfoJson);
  const roomId = parsed?.room?.room_id;
  const isLiving = parsed?.room?.show_status === 1;
  if (!roomId) {
    throw new Error('Failed to find room_id in roomInfo JSON');
  }

  return {
    roomId: String(roomId),
    isLiving: parsed?.room?.status === '1' || parsed?.room?.show_status === 1 || parsed?.room?.show_status === 2 || isLiving,
    streamerName: parsed?.room?.nickname || '',
    roomName: parsed?.room?.room_name || '',
    avatarUrl: parsed?.room?.avatar?.big || parsed?.room?.avatar?.middle || '',
  };
}

async function getEncryption(did) {
  const url = `https://www.douyu.com/wgapi/livenc/liveweb/websec/getEncryption?did=${encodeURIComponent(did)}`;
  const json = await fetchJson(url);
  if (json.error !== 0 || !json.data) {
    throw new Error(`Douyu encryption API error: ${json.error}`);
  }
  return json.data;
}

async function getRoomMeta(roomId) {
  try {
    const json = await fetchJson(`https://www.douyu.com/betard/${roomId}`);
    const room = json?.room || {};
    const rawHeat =
      room?.room_biz_all?.hot ??
      room?.hn ??
      room?.show_num ??
      '';
    return {
      screenshotUrl: room.room_pic || room.show_details || '',
      heatText: String(rawHeat ?? ''),
    };
  } catch (error) {
    return {
      screenshotUrl: '',
      heatText: '',
    };
  }
}

function buildAuth(enc, roomId, timestamp) {
  let value = enc.rand_str;
  for (let i = 0; i < enc.enc_time; i += 1) {
    value = md5(value + enc.key);
  }
  const suffix = enc.is_special === 1 ? '' : `${roomId}${timestamp}`;
  return md5(value + enc.key + suffix);
}

async function getDouyuPlayInfo(roomId, pageUrl) {
  const did = randomDid();
  const timestamp = Math.floor(Date.now() / 1000);
  const enc = await getEncryption(did);
  const auth = buildAuth(enc, roomId, timestamp);

  const form = new URLSearchParams({
    enc_data: enc.enc_data,
    tt: String(timestamp),
    did,
    auth,
    cdn: '',
    rate: '0',
    hevc: '1',
    fa: '0',
    ive: '0',
  });

  return fetchJson(`https://www.douyu.com/lapi/live/getH5PlayV1/${roomId}`, {
    method: 'POST',
    headers: {
      'content-type': 'application/x-www-form-urlencoded; charset=UTF-8',
      referer: pageUrl,
      origin: 'https://www.douyu.com',
    },
    body: form.toString(),
  });
}

function buildPrimaryUrl(playData) {
  return `${playData.rtmp_url}/${playData.rtmp_live}`;
}

function buildXsInfo(playData) {
  const meta = playData.p2pMeta;
  if (!meta) {
    return null;
  }

  const xsParts = playData.rtmp_live.replace(/flv/g, 'xs').split('&');
  xsParts.push(`delay=${meta.xp2p_txDelay}`);
  xsParts.push(`txSecret=${meta.xp2p_txSecret}`);
  xsParts.push(`txTime=${meta.xp2p_txTime}`);
  xsParts.push(`uuid=${crypto.randomUUID()}`);

  return {
    xsPath: `${meta.xp2p_domain}/live/${xsParts.join('&')}`,
    cdnUrl: `https://${meta.xp2p_domain}/${playData.rtmp_live.split('.')[0]}.xs`,
  };
}

async function getBackupUrls(playData) {
  const xsInfo = buildXsInfo(playData);
  if (!xsInfo) {
    return [];
  }

  try {
    const json = await fetchJson(xsInfo.cdnUrl);
    const domains = [...(json?.sug || []), ...(json?.bak || [])];
    return domains.map(domain => `https://${domain}/${xsInfo.xsPath}`);
  } catch (error) {
    return [];
  }
}

async function extractPlayInfo(input) {
  const pageUrl = normalizeRoomInput(input);
  const html = await fetchText(pageUrl, {
    headers: {
      referer: 'https://www.douyu.com/',
    },
  });
  const roomInfo = extractRoomInfo(html);
  if (!roomInfo.isLiving) {
    return {
      offline: true,
      isOnline: false,
      roomId: roomInfo.roomId,
      streamerName: roomInfo.streamerName,
      roomName: roomInfo.roomName,
      avatarUrl: roomInfo.avatarUrl,
      screenshotUrl: '',
      heatText: '',
      pageUrl,
      userAgent: USER_AGENT,
      log: '主播未开播',
    };
  }

  const roomMeta = await getRoomMeta(roomInfo.roomId);
  const playResponse = await getDouyuPlayInfo(roomInfo.roomId, pageUrl);
  if (playResponse.error !== 0 || !playResponse.data) {
    throw new Error(`Douyu play API error: ${playResponse.error}`);
  }

  const playData = playResponse.data;
  const primaryUrl = buildPrimaryUrl(playData);
  const backupUrls = await getBackupUrls(playData);

  return {
    isOnline: true,
    roomId: roomInfo.roomId,
    streamerName: roomInfo.streamerName,
    roomName: roomInfo.roomName,
    avatarUrl: roomInfo.avatarUrl,
    screenshotUrl: roomMeta.screenshotUrl,
    heatText: roomMeta.heatText,
    pageUrl,
    userAgent: USER_AGENT,
    title: playData.room_name || '',
    rate: playData.rate,
    multirates: playData.multirates || [],
    url: primaryUrl,
    urls: [primaryUrl, ...backupUrls],
    flvUrl: primaryUrl,
    p2pUrls: backupUrls,
  };
}

async function main() {
  const room = process.argv[2];
  if (!room) {
    console.error('Usage: node douyu_extract_playurl.js <room-id-or-url>');
    process.exit(1);
  }

  let lastError;
  for (let attempt = 0; attempt < 3; attempt += 1) {
    try {
      const result = await extractPlayInfo(room);
      console.log(JSON.stringify(result, null, 2));
      return;
    } catch (error) {
      lastError = error;
      if (attempt < 2) {
        await sleep(1000);
      }
    }
  }

  throw lastError;
}

main().catch(error => {
  console.error(error);
  process.exit(1);
});
