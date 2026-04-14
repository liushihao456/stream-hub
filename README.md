# stream-hub

通过斗鱼 H5 播放接口提取直播流，并交给 `mpv` 播放。

## 现状说明

斗鱼直播流地址不是固定写死在 HTML 里的，而是需要先拿房间信息，再调用斗鱼的 H5 播放接口获取带签名的直播地址。

这个项目当前采用的方案是：

1. 请求斗鱼直播间页面，提取真实房间号和开播状态
2. 调用斗鱼加密接口获取签名参数
3. 调用 `getH5PlayV1` 获取主播放地址和备用线路
4. 生成本地 `.m3u` 播放列表并交给 `mpv`

## 文件说明

### `douyu_extract_playurl.js`

直接调用斗鱼页面和 H5 播放接口，并输出如下 JSON：

- 直播中时：
  - `url`: 主播放地址
  - `urls`: 主播放地址和备用线路列表
  - `roomId`: 房间号
- 未开播时：
  - `offline: true`
  - `log`: 对应日志

### `douyu_to_mpv.py`

调用 `douyu_extract_playurl.js` 获取播放链接，然后：

- `--print-only`：只打印真实播放地址
- 不加参数：把主播放地址和备用线路写入本地 `.m3u`，再交给 `mpv` 播放

## 依赖

本项目当前依赖：

- `node`
- `python3`
- `mpv`

本项目当前不依赖额外 npm 包。

## 使用方法

进入项目目录：

```bash
cd ~/Projects/stream-hub
```

## 桌面 App

当前仓库已经包含一个基于 `Tauri + React` 的桌面应用骨架，支持：

- 本地保存主播列表
- 只输入房间号或链接时自动获取主播名
- 配置 `mpv` 路径
- 点击主播后调用现有斗鱼播放脚本

开发模式启动：

```bash
npm install
npm run tauri:dev
```

桌面应用的数据会保存在系统的应用数据目录里，而不是仓库文件中。

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

当前实现会生成一个本地 `.m3u` 播放列表，里面包含主线路和备用线路，并附带 `mpv` 的直播重连参数。

也支持完整 URL：

```bash
python3 douyu_to_mpv.py https://www.douyu.com/12817440
```

## 返回结果示例

### 直播中

```json
{
  "roomId": "12817440",
  "url": "https://xxx.douyucdn.cn/live/xxxx.flv?...",
  "urls": [
    "https://xxx.douyucdn.cn/live/xxxx.flv?...",
    "https://backup1.example/live/xxxx.xs?...",
    "https://backup2.example/live/xxxx.xs?..."
  ]
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

1. 播放地址依然是临时签名链接，不能长期缓存，最好现抓现播。
2. 当前方案依赖斗鱼 H5 接口和签名规则，如果斗鱼改接口，需要跟着调整。
3. 当前默认会把主线路和备用线路一起交给 `mpv`，比单独播放一条短命 URL 更稳。
4. 当前系统自带的 `yt-dlp` 对斗鱼直播页提取存在失效情况，因此本项目暂时不依赖 `yt-dlp`。

## 已验证

已在本机验证以下流程可行：

- 打开斗鱼直播间页面
- 提取 H5 播放地址
- 生成本地 `.m3u` 播放列表
- `mpv` 能正常打开并播放

## 后续可扩展方向

- 支持更多平台
- 支持主播名自动搜索房间号
- 增加命令行参数（例如 `--player`, `--json`, `--rate`）
- 做成统一 CLI 工具
