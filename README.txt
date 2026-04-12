stream-hub

用途：从斗鱼直播网页抓取临时播放链接，并交给 mpv 播放。

文件说明：
1. douyu_extract_playurl.js
   用 Playwright 打开斗鱼直播间页面，从页面运行时日志里提取真实 .flv/.m3u8 播放地址。

2. douyu_to_mpv.py
   调用上面的 JS 提取脚本，并把拿到的链接交给 mpv 播放。

用法：
node douyu_extract_playurl.js 12817440
python3 douyu_to_mpv.py 12817440 --print-only
python3 douyu_to_mpv.py 12817440

也支持完整房间 URL：
python3 douyu_to_mpv.py https://www.douyu.com/12817440

注意：
1. 链接是临时签名链接，最好现抓现播。
2. 主播未开播时，脚本会返回 offline。
3. 依赖：node、playwright、python3、mpv。
