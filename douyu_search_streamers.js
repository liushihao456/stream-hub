const USER_AGENT =
  'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36';

async function fetchJson(url, init = {}) {
  const response = await fetch(url, {
    ...init,
    headers: {
      'user-agent': USER_AGENT,
      referer: 'https://www.douyu.com/search/',
      ...(init.headers || {}),
    },
  });

  if (!response.ok) {
    throw new Error(`HTTP ${response.status} for ${url}`);
  }

  return response.json();
}

function normalizeImage(url) {
  return typeof url === 'string' && url.trim() ? url.trim() : '';
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
      screenshotUrl: normalizeImage(room.room_pic || room.show_details || ''),
      heatText: rawHeat === '' ? '' : String(rawHeat),
    };
  } catch (error) {
    return {
      screenshotUrl: '',
      heatText: '',
    };
  }
}

async function searchStreamers(keyword) {
  const query = keyword.trim();
  if (!query) {
    throw new Error('Missing keyword');
  }

  const url = new URL('https://www.douyu.com/japi/search/api/searchUser');
  url.searchParams.set('kw', query);
  url.searchParams.set('page', '1');
  url.searchParams.set('pageSize', '30');

  const json = await fetchJson(url.toString());
  const users = json?.data?.relateUser || [];

  const results = users
    .map(item => item?.anchorInfo)
    .filter(anchor => anchor?.rid && anchor?.nickName)
    .map(anchor => ({
      name: String(anchor.nickName).trim(),
      target: String(anchor.rid).trim(),
      roomId: String(anchor.rid).trim(),
      roomName: String(anchor.description || '').trim(),
      avatarUrl: normalizeImage(anchor.avatar),
      screenshotUrl: normalizeImage(anchor.roomSrc),
      isOnline: Number(anchor.isLive) === 1,
      heatText: '',
    }));

  const enriched = await Promise.all(
    results.map(async streamer => {
      if (!streamer.isOnline) {
        return {
          ...streamer,
          heatText: '',
        };
      }

      const meta = await getRoomMeta(streamer.roomId);
      return {
        ...streamer,
        screenshotUrl: meta.screenshotUrl || streamer.screenshotUrl,
        heatText: meta.heatText || '',
      };
    }),
  );

  return enriched;
}

async function main() {
  const keyword = process.argv.slice(2).join(' ').trim();
  if (!keyword) {
    console.error('Usage: node douyu_search_streamers.js <keyword>');
    process.exit(1);
  }

  try {
    const results = await searchStreamers(keyword);
    process.stdout.write(JSON.stringify(results, null, 2));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}

main();
