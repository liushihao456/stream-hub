# Stream Hub

Stream Hub 是一个基于 `Tauri + React` 的桌面直播聚合播放器，用来收藏、搜索并在应用内播放常看的直播间。

![Stream Hub 播放演示](docs/screenrecord.gif)

## 功能特性

- 支持收藏直播间，并按“已开播 / 未开播”分组展示。
- 支持搜索并添加斗鱼、B 站直播、虎牙、抖音直播主播。
- 使用网页播放器播放直播：`<video>` + MSE，支持 HTTP-FLV/HLS。
- 播放页支持弹幕、返回按钮、全屏按钮、Apple TV 风格 playback track。
- 通过应用内本地 stream proxy 请求直播流，处理 CORS、防盗链 Referer/User-Agent/Cookie 等限制。
- 启动时先加载本地收藏数据，主播在线状态在后台刷新，减少首屏等待。

## 安装

从 GitHub Releases 下载当前版本安装包：

- macOS：下载 `Stream.Hub_0.1.6_*.dmg`。
- Windows：下载 `Stream.Hub_0.1.6_x64-setup.exe` 或 `Stream.Hub_0.1.6_x64_en-US.msi`。

## 运行环境

- macOS 或 Windows
- Node.js
- Rust / Cargo

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
- 网页播放器主要支持 HTTP-FLV/HLS，依赖 WebView 的 MSE/解码能力；H.265/HEVC、特殊音频编码、长时间播放稳定性需要按平台验证。
- 仓库中仍保留部分早期斗鱼提流脚本，主要用于调试和历史兼容；桌面应用入口以 Tauri 为准。
