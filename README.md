# stream-hub

从斗鱼直播网页提取临时播放链接，并交给 `mpv` 播放。

## 现状说明

斗鱼直播网页里的真实播放地址不是固定写死在 HTML 里的，而是页面运行后由前端逻辑动态拿到的临时签名流地址。

这个项目当前采用的方案是：

1. 用 Playwright 打开斗鱼直播间页面
2. 等页面播放器初始化
3. 从页面运行时日志 `window.__playLog.logs` 中提取真实 `.flv` / `.m3u8` 地址
4. 把该地址打印出来，或直接交给 `mpv` 播放

## 文件说明

### `douyu_extract_playurl.js`

使用 Playwright 打开斗鱼直播间页面，并输出如下 JSON：

- 直播中时：
  - `url`: 临时播放地址
  - `log`: 命中的日志行
- 未开播时：
  - `offline: true`
  - `log`: 对应日志

### `douyu_to_mpv.py`

调用 `douyu_extract_playurl.js` 获取播放链接，然后：

- `--print-only`：只打印真实播放地址
- 不加参数：直接调用 `mpv` 播放

## 依赖

本项目当前依赖：

- `node`
- `python3`
- `mpv`
- `playwright`

当前项目目录里已经安装过 `playwright`，如果你换机器或重装依赖，可执行：

```bash
npm install
```

如果 Playwright 浏览器缺失，再执行：

```bash
npx playwright install chromium
```

## 使用方法

进入项目目录：

```bash
cd ~/Projects/stream-hub
```

### 1) 提取斗鱼播放链接

```bash
node douyu_extract_playurl.js 12817440
```

也支持传完整直播间 URL：

```bash
node douyu_extract_playurl.js https://www.douyu.com/12817440
```

### 2) 只打印真实播放地址

```bash
python3 douyu_to_mpv.py 12817440 --print-only
```

### 3) 直接用 mpv 播放

```bash
python3 douyu_to_mpv.py 12817440
```

也支持完整 URL：

```bash
python3 douyu_to_mpv.py https://www.douyu.com/12817440
```

## 返回结果示例

### 直播中

```json
{
  "url": "https://xxx.douyucdn.cn/live/xxxx.flv?...",
  "log": "...stream url change..."
}
```

### 未开播

```json
{
  "offline": true,
  "log": "主播未开播"
}
```

## 注意事项

1. 播放地址是临时签名链接，不能长期缓存，最好现抓现播。
2. 斗鱼网页实现如果改动，`window.__playLog.logs` 方案未来可能失效。
3. 当前实测 `mpv` 可以直接播放抓到的 `.flv` 直播流。
4. 当前系统自带的 `yt-dlp` 对斗鱼直播页提取存在失效情况，因此本项目暂时不依赖 `yt-dlp`。

## 已验证

已在本机验证以下流程可行：

- 打开斗鱼直播间页面
- 提取临时 `.flv` 播放地址
- `ffprobe` 能识别流格式
- `mpv` 能正常打开并播放

## 后续可扩展方向

- 支持更多平台
- 支持主播名自动搜索房间号
- 增加命令行参数（例如 `--player`, `--json`, `--timeout`）
- 做成统一 CLI 工具
