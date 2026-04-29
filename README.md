# Stream Hub

Stream Hub 是一个基于 `Tauri + React + libmpv` 的桌面直播聚合播放器，用来收藏、搜索并在应用内播放常看的直播间。

![Stream Hub 主界面截图](docs/screenshot.png)

## 功能特性

- 支持收藏直播间，并按“已开播 / 未开播”分组展示。
- 支持搜索并添加斗鱼、B 站直播、虎牙、抖音直播主播。
- 内嵌 `libmpv` 播放器，点击主播卡片后进入独立播放页。
- 播放页支持弹幕、返回按钮、Apple TV 风格 playback track。
- 直播播放支持最长 5 分钟内存缓存，可在缓存窗口内回退观看。
- 启动时先加载本地收藏数据，主播在线状态在后台刷新，减少首屏等待。

## 运行环境

- macOS 或 Windows
- Node.js
- Rust / Cargo
- 系统可加载的 `libmpv`

macOS 可通过 Homebrew 安装 `libmpv`：

```bash
brew install mpv
```

Windows 需要确保 `mpv-2.dll` / `libmpv` 可被应用加载。

## 开发

安装依赖：

```bash
npm install
```

启动开发模式：

```bash
npm run tauri:dev
```

构建桌面应用：

```bash
npm run tauri:build
```

macOS 构建产物会生成在：

```bash
src-tauri/target/release/bundle/macos/Stream Hub.app
```

## 数据存储

主播收藏、设置、B 站登录态等数据保存在系统应用数据目录中，不会写入仓库目录。

## 支持平台

当前桌面应用支持：

- 斗鱼
- B 站直播
- 虎牙
- 抖音直播

## 说明

- 直播地址通常是临时签名链接，应用会在点击播放时实时提取。
- 弹幕功能依赖应用内本地弹幕服务。
- 直播回退能力基于 `libmpv` 的 demuxer cache；高码率直播在内存限制下可能不足完整 5 分钟。
- 仓库中仍保留部分早期斗鱼提流脚本，主要用于调试和历史兼容；桌面应用入口以 Tauri 为准。
