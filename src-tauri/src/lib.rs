use axum::extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade};
use axum::body::Body;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::{HeaderValue as AxumHeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use base64::Engine;
use flate2::read::GzDecoder;
use futures_util::{SinkExt, StreamExt};
use regex::Regex;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, ORIGIN, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command as TokioCommand;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue as WsHeaderValue;
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;
use url::Url;
use uuid::Uuid;


#[cfg(target_os = "macos")]
use block2::RcBlock;
#[cfg(target_os = "macos")]
use objc2_foundation::{NSArray, NSHTTPCookie};
#[cfg(target_os = "macos")]
use objc2_web_kit::WKWebView;
#[cfg(target_os = "macos")]
use std::ptr::NonNull;

const DOUYU_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36";
const BILIBILI_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
const HUYA_USER_AGENT: &str =
    "Mozilla/5.0 (iPhone; CPU iPhone OS 14_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/14.0 Mobile/15E148 Safari/604.1";
const DOUYIN_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.6 Safari/605.1.15";
const PLATFORM_DOUYU: &str = "douyu";
const PLATFORM_BILIBILI_LIVE: &str = "bilibili_live";
const PLATFORM_HUYA: &str = "huya";
const PLATFORM_DOUYIN_LIVE: &str = "douyin_live";
const HUYA_SEARCH_STATUS_LIMIT: usize = 6;
const DOUYIN_SEARCH_VERIFY_LIMIT: usize = 5;

fn default_streamer_platform() -> String {
    PLATFORM_DOUYU.to_string()
}

fn default_danmaku_font_size() -> u32 {
    18
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Streamer {
    id: String,
    name: String,
    #[serde(default = "default_streamer_platform")]
    platform: String,
    target: String,
    avatar_url: Option<String>,
    is_online: Option<bool>,
    screenshot_url: Option<String>,
    heat_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    bilibili_cookie: String,
    #[serde(default = "default_danmaku_font_size")]
    danmaku_font_size: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            bilibili_cookie: String::new(),
            danmaku_font_size: default_danmaku_font_size(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolvedStreamer {
    name: String,
    platform: String,
    target: String,
    room_id: String,
    room_name: String,
    streamer_name: String,
    avatar_url: String,
    is_online: bool,
    screenshot_url: String,
    heat_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchStreamer {
    name: String,
    platform: String,
    target: String,
    room_id: String,
    room_name: String,
    avatar_url: String,
    is_online: bool,
    screenshot_url: String,
    heat_text: String,
}

#[derive(Debug, Clone, Default)]
struct ExtractResult {
    platform: String,
    room_id: String,
    streamer_name: String,
    room_name: String,
    avatar_url: String,
    is_online: bool,
    screenshot_url: String,
    heat_text: String,
    page_url: String,
    title: String,
    urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FrontendPlayInfo {
    platform: String,
    room_id: String,
    streamer_name: String,
    room_name: String,
    avatar_url: String,
    is_online: bool,
    screenshot_url: String,
    heat_text: String,
    page_url: String,
    title: String,
    urls: Vec<String>,
    proxy_urls: Vec<String>,
}

#[derive(Debug, Clone)]
struct StreamProxySession {
    url: String,
    headers: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
struct RoomInfo {
    room_id: String,
    is_living: bool,
    streamer_name: String,
    room_name: String,
    avatar_url: String,
}

#[derive(Debug, Clone, Default)]
struct RoomMeta {
    screenshot_url: String,
    heat_text: String,
}

#[derive(Debug, Clone, Default)]
struct RoomSnapshot {
    room_id: String,
    streamer_name: String,
    room_name: String,
    avatar_url: String,
    is_online: bool,
    screenshot_url: String,
    heat_text: String,
}

#[derive(Debug, Clone, Serialize)]
struct DanmakuCommentPayload {
    text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DanmakuEventPayload {
    method: String,
    kind: String,
    timestamp_ms: u64,
    dms: Vec<DanmakuCommentPayload>,
}

struct DanmakuServerState {
    started: AtomicBool,
    port: u16,
    stream_sessions: Arc<Mutex<HashMap<String, StreamProxySession>>>,
}

#[derive(Debug, Clone, Deserialize)]
struct DanmakuBridgeEvent {
    #[serde(rename = "type")]
    event_type: String,
    text: String,
}

#[derive(Debug, Clone, Deserialize)]
struct SearchApiUserResponse {
    data: SearchApiUserData,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct SearchApiUserData {
    #[serde(default, rename = "relateUser")]
    relate_user: Vec<SearchApiUserItem>,
}

#[derive(Debug, Clone, Deserialize)]
struct SearchApiUserItem {
    #[serde(rename = "anchorInfo")]
    anchor_info: SearchApiAnchorInfo,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct SearchApiAnchorInfo {
    #[serde(default, rename = "rid", deserialize_with = "deserialize_value_to_string")]
    room_id: String,
    #[serde(default, rename = "nickName")]
    nick_name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    avatar: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliLiveSearchResponse {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: BilibiliLiveSearchData,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct BilibiliLiveSearchData {
    #[serde(default)]
    result: Vec<BilibiliLiveSearchItem>,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliLiveSearchItem {
    #[serde(default, deserialize_with = "deserialize_value_to_string")]
    roomid: String,
    #[serde(default)]
    uname: String,
    #[serde(default)]
    live_status: i64,
    #[serde(default)]
    uface: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliRoomInfoResponse {
    data: BilibiliRoomInfoData,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliRoomInfoData {
    #[serde(deserialize_with = "deserialize_value_to_string")]
    room_id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    user_cover: String,
    #[serde(default)]
    keyframe: String,
    #[serde(default, deserialize_with = "deserialize_value_to_string")]
    online: String,
    live_status: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliAnchorInfoResponse {
    data: BilibiliAnchorInfoData,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliAnchorInfoData {
    info: BilibiliAnchorInfo,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliAnchorInfo {
    #[serde(default)]
    uname: String,
    #[serde(default)]
    face: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliRoomPlayInfoResponse {
    data: BilibiliRoomPlayInfoData,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliRoomPlayInfoData {
    encrypted: bool,
    #[serde(default)]
    pwd_verified: bool,
    playurl_info: BilibiliPlayurlInfo,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliPlayurlInfo {
    playurl: BilibiliPlayurl,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliPlayurl {
    #[serde(default)]
    stream: Vec<BilibiliPlayStream>,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliPlayStream {
    #[serde(default, rename = "protocol_name")]
    protocol_name: String,
    #[serde(default, rename = "format")]
    formats: Vec<BilibiliPlayFormat>,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliPlayFormat {
    #[serde(default, rename = "format_name")]
    format_name: String,
    #[serde(default, rename = "codec")]
    codecs: Vec<BilibiliPlayCodec>,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliPlayCodec {
    #[serde(default, rename = "codec_name")]
    codec_name: String,
    #[serde(default, rename = "current_qn")]
    current_qn: i64,
    #[serde(default, rename = "accept_qn")]
    accept_qn: Vec<i64>,
    #[serde(default, rename = "base_url")]
    base_url: String,
    #[serde(default, rename = "url_info")]
    url_info: Vec<BilibiliUrlInfo>,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliUrlInfo {
    #[serde(default)]
    host: String,
    #[serde(default)]
    extra: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliOldPlayUrlResponse {
    data: BilibiliOldPlayUrlData,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliOldPlayUrlData {
    #[serde(default)]
    durl: Vec<BilibiliOldDurl>,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliOldDurl {
    #[serde(default)]
    url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliRoomBaseInfoResponse {
    data: BilibiliRoomBaseInfoData,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliRoomBaseInfoData {
    #[serde(default, rename = "by_room_ids")]
    by_room_ids: HashMap<String, BilibiliRoomBaseInfo>,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliRoomBaseInfo {
    #[serde(deserialize_with = "deserialize_value_to_string")]
    room_id: String,
    #[serde(default, deserialize_with = "deserialize_value_to_string")]
    short_id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    uname: String,
    #[serde(default)]
    cover: String,
    #[serde(default)]
    live_url: String,
    #[serde(default, deserialize_with = "deserialize_value_to_string")]
    online: String,
    live_status: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct HuyaProfileRoomResponse {
    data: HuyaProfileRoomData,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct HuyaProfileRoomData {
    #[serde(default, rename = "liveStatus")]
    live_status: String,
    #[serde(default, rename = "realLiveStatus")]
    real_live_status: String,
    #[serde(default, rename = "liveData")]
    live_data: HuyaLiveData,
    #[serde(default)]
    stream: Option<HuyaStreamData>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct HuyaLiveData {
    #[serde(default, rename = "roomName")]
    room_name: String,
    #[serde(default)]
    introduction: String,
    #[serde(default)]
    nick: String,
    #[serde(default, rename = "avatar180")]
    avatar_180: String,
    #[serde(default)]
    screenshot: String,
    #[serde(
        default,
        rename = "totalCount",
        deserialize_with = "deserialize_value_to_string"
    )]
    total_count: String,
    #[serde(
        default,
        rename = "activityCount",
        deserialize_with = "deserialize_value_to_string"
    )]
    activity_count: String,
    #[serde(
        default,
        rename = "profileRoom",
        deserialize_with = "deserialize_value_to_string"
    )]
    profile_room: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct HuyaStreamData {
    #[serde(default, rename = "baseSteamInfoList")]
    base_stream_info_list: Vec<HuyaStreamInfo>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct HuyaStreamInfo {
    #[serde(default, rename = "sStreamName")]
    s_stream_name: String,
    #[serde(default, rename = "sFlvUrl")]
    s_flv_url: String,
    #[serde(default, rename = "sFlvUrlSuffix")]
    s_flv_url_suffix: String,
    #[serde(default, rename = "sFlvAntiCode")]
    s_flv_anti_code: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DouyinHelperResponse {
    room_id: String,
    streamer_name: String,
    room_name: String,
    avatar_url: String,
    screenshot_url: String,
    is_online: bool,
    heat_text: String,
    page_url: String,
    title: String,
    urls: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct DouyinDanmakuResponse {
    cookie: String,
    user_agent: String,
    referer: String,
    ws_url: String,
}

#[derive(Debug, Clone, Default)]
struct DouyinDanmakuBatch {
    comments: Vec<String>,
    need_ack: bool,
    ack_payload: Vec<u8>,
    log_id: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct DanmakuWsQuery {
    #[serde(default, rename = "roomId")]
    room_id: String,
    #[serde(default)]
    platform: String,
}

#[derive(Clone)]
struct DanmakuHttpState {
    node_bin: String,
    bridge_script: PathBuf,
    douyin_helper_script: PathBuf,
    stream_sessions: Arc<Mutex<HashMap<String, StreamProxySession>>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlatformIcons {
    bilibili: String,
    douyu: String,
    huya: String,
    douyin: String,
}

fn douyu_client() -> Result<Client, String> {
    Client::builder()
        .cookie_store(true)
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|err| err.to_string())
}

fn bilibili_client() -> Result<Client, String> {
    Client::builder()
        .cookie_store(true)
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|err| err.to_string())
}

fn huya_client() -> Result<Client, String> {
    Client::builder()
        .cookie_store(true)
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|err| err.to_string())
}

fn douyin_client() -> Result<Client, String> {
    Client::builder()
        .cookie_store(true)
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|err| err.to_string())
}

fn normalize_platform(platform: &str) -> &'static str {
    match platform.trim().to_lowercase().as_str() {
        PLATFORM_BILIBILI_LIVE => PLATFORM_BILIBILI_LIVE,
        PLATFORM_HUYA => PLATFORM_HUYA,
        PLATFORM_DOUYIN_LIVE => PLATFORM_DOUYIN_LIVE,
        _ => PLATFORM_DOUYU,
    }
}

fn infer_platform_from_target(target: &str) -> &'static str {
    let trimmed = target.trim().to_lowercase();
    if trimmed.contains("live.bilibili.com") {
        PLATFORM_BILIBILI_LIVE
    } else if trimmed.contains("huya.com") {
        PLATFORM_HUYA
    } else if trimmed.contains("live.douyin.com") || trimmed.contains("v.douyin.com") {
        PLATFORM_DOUYIN_LIVE
    } else {
        PLATFORM_DOUYU
    }
}

fn ensure_streamer_platform(streamer: &mut Streamer) {
    streamer.platform = normalize_platform(if streamer.platform.trim().is_empty() {
        infer_platform_from_target(&streamer.target)
    } else {
        &streamer.platform
    })
    .to_string();
}

fn strip_html_tags(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    output
}

fn build_bilibili_live_url(room_id: &str) -> String {
    format!("https://live.bilibili.com/{}", room_id.trim())
}

fn build_huya_live_url(room_id: &str) -> String {
    format!("https://www.huya.com/{}", room_id.trim())
}

fn build_douyin_live_url(room_id: &str) -> String {
    format!("https://live.douyin.com/{}", room_id.trim())
}

fn extract_bilibili_room_id(target: &str) -> Option<String> {
    let trimmed = target.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|char| char.is_ascii_digit()) {
        return Some(trimmed.to_string());
    }

    let without_fragment = trimmed.split('#').next().unwrap_or(trimmed);
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);
    let segment = without_query
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();

    if !segment.is_empty() && segment.chars().all(|char| char.is_ascii_digit()) {
        Some(segment.to_string())
    } else {
        None
    }
}

fn extract_huya_room_id(target: &str) -> Option<String> {
    let trimmed = target.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|char| char.is_ascii_digit()) {
        return Some(trimmed.to_string());
    }

    let without_fragment = trimmed.split('#').next().unwrap_or(trimmed);
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);
    let segment = without_query
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();

    if !segment.is_empty() && segment.chars().all(|char| char.is_ascii_digit()) {
        Some(segment.to_string())
    } else {
        None
    }
}

fn extract_douyin_room_id(target: &str) -> Option<String> {
    let trimmed = target.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|char| char.is_ascii_digit()) {
        return Some(trimmed.to_string());
    }

    let without_fragment = trimmed.split('#').next().unwrap_or(trimmed);
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);
    let segments = without_query
        .trim_end_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    for segment in segments.iter().rev() {
        if !segment.is_empty() && segment.chars().all(|char| char.is_ascii_digit()) {
            return Some((*segment).to_string());
        }
    }

    None
}

fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn danmaku_text_event(kind: &str, text: impl Into<String>) -> Result<String, String> {
    let event = DanmakuEventPayload {
        method: "sendDM".to_string(),
        kind: kind.to_string(),
        timestamp_ms: current_timestamp_ms(),
        dms: vec![DanmakuCommentPayload { text: text.into() }],
    };
    serde_json::to_string(&event).map_err(|err| err.to_string())
}

fn deserialize_value_to_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(value_to_string(&value))
}

fn md5_hex(input: &str) -> String {
    format!("{:x}", md5::compute(input))
}

fn normalize_room_input(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://www.douyu.com/{trimmed}")
    }
}

fn extract_room_id_from_target(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|char| char.is_ascii_digit()) {
        return Some(trimmed.to_string());
    }

    let without_fragment = trimmed.split('#').next().unwrap_or(trimmed);
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);
    let segment = without_query
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();

    if !segment.is_empty() && segment.chars().all(|char| char.is_ascii_digit()) {
        Some(segment.to_string())
    } else {
        None
    }
}

fn fetch_text(url: &str, referer: Option<&str>) -> Result<String, String> {
    let client = douyu_client()?;
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(DOUYU_USER_AGENT).map_err(|err| err.to_string())?,
    );
    if let Some(value) = referer {
        headers.insert(
            REFERER,
            HeaderValue::from_str(value).map_err(|err| err.to_string())?,
        );
    }

    let response = client
        .get(url)
        .headers(headers)
        .send()
        .map_err(|err| err.to_string())?;

    if !response.status().is_success() {
        return Err(format!("HTTP {} for {}", response.status(), url));
    }

    response.text().map_err(|err| err.to_string())
}

fn fetch_text_with_headers(
    client: &Client,
    url: &str,
    user_agent: &str,
    referer: Option<&str>,
    origin: Option<&str>,
) -> Result<String, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(user_agent).map_err(|err| err.to_string())?,
    );
    if let Some(value) = referer {
        headers.insert(
            REFERER,
            HeaderValue::from_str(value).map_err(|err| err.to_string())?,
        );
    }
    if let Some(value) = origin {
        headers.insert(
            ORIGIN,
            HeaderValue::from_str(value).map_err(|err| err.to_string())?,
        );
    }

    let response = client
        .get(url)
        .headers(headers)
        .send()
        .map_err(|err| err.to_string())?;

    if !response.status().is_success() {
        return Err(format!("HTTP {} for {}", response.status(), url));
    }

    response.text().map_err(|err| err.to_string())
}

fn fetch_json_with_headers(
    client: &Client,
    url: &str,
    user_agent: &str,
    referer: Option<&str>,
    origin: Option<&str>,
) -> Result<Value, String> {
    let text = fetch_text_with_headers(client, url, user_agent, referer, origin)?;
    serde_json::from_str(&text).map_err(|err| err.to_string())
}

fn fetch_json(url: &str, referer: Option<&str>) -> Result<Value, String> {
    let text = fetch_text(url, referer)?;
    serde_json::from_str(&text).map_err(|err| err.to_string())
}

fn fetch_search_json(url: &str, keyword: &str) -> Result<Value, String> {
    let client = douyu_client()?;
    let mut bootstrap_headers = HeaderMap::new();
    bootstrap_headers.insert(
        USER_AGENT,
        HeaderValue::from_str(DOUYU_USER_AGENT).map_err(|err| err.to_string())?,
    );
    bootstrap_headers.insert(
        REFERER,
        HeaderValue::from_static("https://www.douyu.com/search/"),
    );
    bootstrap_headers.insert(ORIGIN, HeaderValue::from_static("https://www.douyu.com"));
    bootstrap_headers.insert(
        "accept",
        HeaderValue::from_static("application/json, text/plain, */*"),
    );
    bootstrap_headers.insert(
        "x-requested-with",
        HeaderValue::from_static("XMLHttpRequest"),
    );

    let _ = client
        .get("https://www.douyu.com/")
        .headers(bootstrap_headers.clone())
        .send();
    let _ = client
        .get(format!(
            "https://www.douyu.com/search?kw={}",
            urlencoding::encode(keyword)
        ))
        .headers(bootstrap_headers.clone())
        .send();

    let response = client
        .get(url)
        .headers(bootstrap_headers)
        .send()
        .map_err(|err| err.to_string())?;

    if !response.status().is_success() {
        return Err(format!("HTTP {} for {}", response.status(), url));
    }

    response.json().map_err(|err| err.to_string())
}

fn fetch_bilibili_json(url: &str, referer: &str, raw_cookie: &str) -> Result<Value, String> {
    let client = bilibili_client()?;
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(BILIBILI_USER_AGENT).map_err(|err| err.to_string())?,
    );
    headers.insert(
        REFERER,
        HeaderValue::from_str(referer).map_err(|err| err.to_string())?,
    );
    headers.insert(
        ORIGIN,
        HeaderValue::from_static("https://live.bilibili.com"),
    );
    if let Some(cookie_header) = build_bilibili_cookie_header(raw_cookie)? {
        headers.insert(reqwest::header::COOKIE, cookie_header);
    }

    let _ = client
        .get("https://www.bilibili.com")
        .headers(headers.clone())
        .send();

    let response = client
        .get(url)
        .headers(headers)
        .send()
        .map_err(|err| err.to_string())?;

    if !response.status().is_success() {
        return Err(format!("HTTP {} for {}", response.status(), url));
    }

    response.json().map_err(|err| err.to_string())
}

fn build_bilibili_cookie_header(raw_cookie: &str) -> Result<Option<HeaderValue>, String> {
    let mut cookies = raw_cookie
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if !cookies
        .iter()
        .any(|part| part.split('=').next().unwrap_or_default().trim() == "CURRENT_QUALITY")
    {
        cookies.push("CURRENT_QUALITY=125".to_string());
    }

    if cookies.is_empty() {
        return Ok(None);
    }

    let header = cookies.join("; ");
    let value = HeaderValue::from_str(&header).map_err(|err| err.to_string())?;
    Ok(Some(value))
}

pub(crate) fn build_bilibili_cookie_string(raw_cookie: &str) -> Result<Option<String>, String> {
    Ok(build_bilibili_cookie_header(raw_cookie)?
        .and_then(|value| value.to_str().ok().map(ToOwned::to_owned)))
}

fn fetch_bilibili_search(keyword: &str) -> Result<Vec<BilibiliLiveSearchItem>, String> {
    let client = bilibili_client()?;
    let attempts = [
        (
            "https://api.bilibili.com/x/web-interface/wbi/search/type",
            "https://search.bilibili.com/",
            None,
        ),
        (
            "https://api.bilibili.com/x/web-interface/wbi/search/type",
            "https://www.bilibili.com/",
            None,
        ),
        (
            "https://api.bilibili.com/x/web-interface/search/type",
            "https://www.bilibili.com/",
            None,
        ),
        (
            "https://api.bilibili.com/x/web-interface/search/type",
            "https://www.bilibili.com/",
            Some("https://search.bilibili.com"),
        ),
        (
            "https://api.bilibili.com/x/web-interface/search/type",
            "https://search.bilibili.com/",
            None,
        ),
    ];
    let mut last_error = String::new();

    for (url, referer, origin) in attempts {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(BILIBILI_USER_AGENT).map_err(|err| err.to_string())?,
        );
        headers.insert(
            REFERER,
            HeaderValue::from_str(referer).map_err(|err| err.to_string())?,
        );
        if let Some(origin) = origin {
            headers.insert(
                ORIGIN,
                HeaderValue::from_str(origin).map_err(|err| err.to_string())?,
            );
        }
        headers.insert(
            "accept",
            HeaderValue::from_static("application/json, text/plain, */*"),
        );

        let response = client
            .get(url)
            .headers(headers)
            .query(&[
                ("search_type", "live_user"),
                ("keyword", keyword),
                ("order", "online"),
                ("page", "1"),
                ("page_size", "20"),
            ])
            .send()
            .map_err(|err| err.to_string());
        let response = match response {
            Ok(response) => response,
            Err(err) => {
                last_error = err;
                continue;
            }
        };
        let status = response.status();
        if !status.is_success() {
            last_error = format!("HTTP {status} for {url}");
            continue;
        }
        let parsed = response.json::<BilibiliLiveSearchResponse>();
        let parsed = match parsed {
            Ok(parsed) => parsed,
            Err(err) => {
                last_error = err.to_string();
                continue;
            }
        };
        if parsed.code != 0 {
            last_error = format!("Bilibili search error {}: {}", parsed.code, parsed.message);
            continue;
        }
        return Ok(parsed.data.result);
    }

    Err(if last_error.is_empty() {
        "Bilibili search failed".to_string()
    } else {
        last_error
    })
}

fn fetch_bilibili_public_json(url: &str, referer: &str) -> Result<Value, String> {
    let client = bilibili_client()?;
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(BILIBILI_USER_AGENT).map_err(|err| err.to_string())?,
    );
    headers.insert(
        REFERER,
        HeaderValue::from_str(referer).map_err(|err| err.to_string())?,
    );
    headers.insert(
        ORIGIN,
        HeaderValue::from_static("https://live.bilibili.com"),
    );

    let response = client
        .get(url)
        .headers(headers)
        .send()
        .map_err(|err| err.to_string())?;

    if !response.status().is_success() {
        return Err(format!("HTTP {} for {}", response.status(), url));
    }

    response.json().map_err(|err| err.to_string())
}

fn post_form_json(url: &str, referer: &str, body: &[(String, String)]) -> Result<Value, String> {
    let client = douyu_client()?;
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(DOUYU_USER_AGENT).map_err(|err| err.to_string())?,
    );
    headers.insert(
        REFERER,
        HeaderValue::from_str(referer).map_err(|err| err.to_string())?,
    );
    headers.insert(ORIGIN, HeaderValue::from_static("https://www.douyu.com"));
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded; charset=UTF-8"),
    );

    let response = client
        .post(url)
        .headers(headers)
        .form(body)
        .send()
        .map_err(|err| err.to_string())?;

    if !response.status().is_success() {
        return Err(format!("HTTP {} for {}", response.status(), url));
    }

    response.json().map_err(|err| err.to_string())
}

fn extract_room_info_json(html: &str) -> Option<String> {
    let marker = "\\\"roomInfo\\\"";
    let marker_index = html.find(marker)?;
    let suffix = &html[marker_index + marker.len()..];
    let open_offset = suffix.find('{')?;
    let start = marker_index + marker.len() + open_offset;
    let bytes = html.as_bytes();
    let mut depth = 0_i32;

    for (index, byte) in bytes.iter().enumerate().skip(start) {
        match *byte as char {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(html[start..=index].replace("\\\"", "\""));
                }
            }
            _ => {}
        }
    }

    None
}

fn value_to_string(value: &Value) -> String {
    value
        .as_str()
        .map(ToString::to_string)
        .or_else(|| value.as_i64().map(|v| v.to_string()))
        .or_else(|| value.as_u64().map(|v| v.to_string()))
        .unwrap_or_default()
}

fn extract_room_info(html: &str) -> Result<RoomInfo, String> {
    let room_info_json = extract_room_info_json(html)
        .ok_or_else(|| "Failed to extract roomInfo JSON".to_string())?;
    let parsed: Value = serde_json::from_str(&room_info_json).map_err(|err| err.to_string())?;
    let room = &parsed["room"];
    let room_id = value_to_string(&room["room_id"]);
    if room_id.trim().is_empty() {
        return Err("Failed to find room_id in roomInfo JSON".into());
    }

    let avatar_big = value_to_string(&room["avatar"]["big"]);
    let avatar_mid = value_to_string(&room["avatar"]["middle"]);

    Ok(RoomInfo {
        room_id,
        is_living: is_room_online(room),
        streamer_name: value_to_string(&room["nickname"]),
        room_name: value_to_string(&room["room_name"]),
        avatar_url: if avatar_big.is_empty() {
            avatar_mid
        } else {
            avatar_big
        },
    })
}

fn is_room_online(room: &Value) -> bool {
    let status_live = value_to_string(&room["status"]) == "1";
    match room["show_status"].as_i64() {
        Some(1) => status_live,
        Some(_) => false,
        None => status_live,
    }
}

fn get_room_meta(room_id: &str) -> Result<RoomMeta, String> {
    let json = fetch_json(&format!("https://www.douyu.com/betard/{room_id}"), None)?;
    let room = &json["room"];
    let room_pic = value_to_string(&room["room_pic"]);
    let show_details = value_to_string(&room["show_details"]);
    let heat = value_to_string(&room["room_biz_all"]["hot"]);

    Ok(RoomMeta {
        screenshot_url: if room_pic.is_empty() {
            show_details
        } else {
            room_pic
        },
        heat_text: heat,
    })
}

fn get_room_snapshot(room_id: &str) -> Result<RoomSnapshot, String> {
    let json = fetch_json(&format!("https://www.douyu.com/betard/{room_id}"), None)?;
    let room = &json["room"];
    let avatar_big = value_to_string(&room["avatar"]["big"]);
    let avatar_mid = value_to_string(&room["avatar"]["middle"]);

    Ok(RoomSnapshot {
        room_id: room_id.to_string(),
        streamer_name: value_to_string(&room["nickname"]),
        room_name: value_to_string(&room["room_name"]),
        avatar_url: if avatar_big.is_empty() {
            avatar_mid
        } else {
            avatar_big
        },
        is_online: is_room_online(room),
        screenshot_url: if is_room_online(room) {
            value_to_string(&room["room_pic"])
        } else {
            String::new()
        },
        heat_text: if is_room_online(room) {
            value_to_string(&room["room_biz_all"]["hot"])
        } else {
            String::new()
        },
    })
}

fn fetch_douyu_room_state(target: &str) -> Result<ExtractResult, String> {
    if let Some(room_id) = extract_room_id_from_target(target) {
        let snapshot = get_room_snapshot(&room_id)?;
        return Ok(ExtractResult {
            platform: PLATFORM_DOUYU.to_string(),
            room_id: snapshot.room_id,
            streamer_name: snapshot.streamer_name,
            room_name: snapshot.room_name,
            avatar_url: snapshot.avatar_url,
            is_online: snapshot.is_online,
            screenshot_url: if snapshot.is_online {
                snapshot.screenshot_url
            } else {
                String::new()
            },
            heat_text: if snapshot.is_online {
                snapshot.heat_text
            } else {
                String::new()
            },
            page_url: normalize_room_input(target),
            title: String::new(),
            urls: Vec::new(),
        });
    }

    let page_url = normalize_room_input(target);
    let html = fetch_text(&page_url, Some("https://www.douyu.com/"))?;
    let room_info = extract_room_info(&html)?;

    if !room_info.is_living {
        return Ok(ExtractResult {
            platform: PLATFORM_DOUYU.to_string(),
            room_id: room_info.room_id,
            streamer_name: room_info.streamer_name,
            room_name: room_info.room_name,
            avatar_url: room_info.avatar_url,
            is_online: false,
            screenshot_url: String::new(),
            heat_text: String::new(),
            page_url,
            title: String::new(),
            urls: Vec::new(),
        });
    }

    let room_meta = get_room_meta(&room_info.room_id).unwrap_or_default();

    Ok(ExtractResult {
        platform: PLATFORM_DOUYU.to_string(),
        room_id: room_info.room_id,
        streamer_name: room_info.streamer_name.clone(),
        room_name: room_info.room_name.clone(),
        avatar_url: room_info.avatar_url,
        is_online: true,
        screenshot_url: room_meta.screenshot_url,
        heat_text: room_meta.heat_text,
        page_url,
        title: room_info.room_name,
        urls: Vec::new(),
    })
}

fn get_encryption(did: &str) -> Result<Value, String> {
    let url = format!("https://www.douyu.com/wgapi/livenc/liveweb/websec/getEncryption?did={did}");
    let json = fetch_json(&url, None)?;
    if json["error"].as_i64().unwrap_or(-1) != 0 {
        return Err(format!(
            "Douyu encryption API error: {}",
            json["error"].as_i64().unwrap_or(-1)
        ));
    }
    Ok(json["data"].clone())
}

fn build_auth(enc: &Value, room_id: &str, timestamp: i64) -> String {
    let mut value = value_to_string(&enc["rand_str"]);
    let enc_time = enc["enc_time"].as_i64().unwrap_or_default();
    let key = value_to_string(&enc["key"]);
    for _ in 0..enc_time {
        value = md5_hex(&format!("{value}{key}"));
    }
    let suffix = if enc["is_special"].as_i64().unwrap_or_default() == 1 {
        String::new()
    } else {
        format!("{room_id}{timestamp}")
    };
    md5_hex(&format!("{value}{key}{suffix}"))
}

fn build_primary_url(play_data: &Value) -> String {
    let rtmp_url = value_to_string(&play_data["rtmp_url"]);
    let rtmp_live = value_to_string(&play_data["rtmp_live"]);
    if rtmp_url.is_empty() || rtmp_live.is_empty() {
        String::new()
    } else {
        format!("{rtmp_url}/{rtmp_live}")
    }
}

fn build_xs_info(play_data: &Value) -> Option<(String, String)> {
    let meta = &play_data["p2pMeta"];
    if meta.is_null() {
        return None;
    }

    let rtmp_live = value_to_string(&play_data["rtmp_live"]);
    let xp2p_domain = value_to_string(&meta["xp2p_domain"]);
    let delay = value_to_string(&meta["xp2p_txDelay"]);
    let tx_secret = value_to_string(&meta["xp2p_txSecret"]);
    let tx_time = value_to_string(&meta["xp2p_txTime"]);
    if rtmp_live.is_empty() || xp2p_domain.is_empty() {
        return None;
    }

    let mut xs_parts: Vec<String> = rtmp_live
        .replace("flv", "xs")
        .split('&')
        .map(str::to_string)
        .collect();
    xs_parts.push(format!("delay={delay}"));
    xs_parts.push(format!("txSecret={tx_secret}"));
    xs_parts.push(format!("txTime={tx_time}"));
    xs_parts.push(format!("uuid={}", Uuid::new_v4()));

    let live_prefix = rtmp_live.split('.').next().unwrap_or_default();
    Some((
        format!("{xp2p_domain}/live/{}", xs_parts.join("&")),
        format!("https://{xp2p_domain}/{live_prefix}.xs"),
    ))
}

fn get_backup_urls(play_data: &Value) -> Vec<String> {
    let Some((xs_path, cdn_url)) = build_xs_info(play_data) else {
        return Vec::new();
    };

    let Ok(json) = fetch_json(&cdn_url, None) else {
        return Vec::new();
    };

    let mut domains = Vec::new();
    for key in ["sug", "bak"] {
        if let Some(items) = json[key].as_array() {
            for item in items {
                if let Some(domain) = item.as_str() {
                    domains.push(domain.to_string());
                }
            }
        }
    }

    domains
        .into_iter()
        .map(|domain| format!("https://{domain}/{xs_path}"))
        .collect()
}

fn extract_douyu_play_info(target: &str) -> Result<ExtractResult, String> {
    let mut state = fetch_douyu_room_state(target)?;
    if !state.is_online {
        return Ok(state);
    }

    let did = md5_hex(&Uuid::new_v4().to_string());
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_secs() as i64;
    let enc = get_encryption(&did)?;
    let auth = build_auth(&enc, &state.room_id, timestamp);

    let body = vec![
        ("enc_data".to_string(), value_to_string(&enc["enc_data"])),
        ("tt".to_string(), timestamp.to_string()),
        ("did".to_string(), did),
        ("auth".to_string(), auth),
        ("cdn".to_string(), String::new()),
        ("rate".to_string(), "0".to_string()),
        ("hevc".to_string(), "1".to_string()),
        ("fa".to_string(), "0".to_string()),
        ("ive".to_string(), "0".to_string()),
    ];

    let url = format!(
        "https://www.douyu.com/lapi/live/getH5PlayV1/{}",
        state.room_id
    );
    let play_response = post_form_json(&url, &state.page_url, &body)?;
    if play_response["error"].as_i64().unwrap_or(-1) != 0 || play_response["data"].is_null() {
        return Err(format!(
            "Douyu play API error: {}",
            play_response["error"].as_i64().unwrap_or(-1)
        ));
    }

    let play_data = &play_response["data"];
    let primary_url = build_primary_url(play_data);
    if primary_url.is_empty() {
        return Err("Failed to build Douyu play url".into());
    }

    let mut urls = get_backup_urls(play_data);
    urls.push(primary_url);
    state.title = value_to_string(&play_data["room_name"]);
    if state.title.is_empty() {
        state.title = state.room_name.clone();
    }
    state.urls = urls;
    Ok(state)
}

fn normalize_huya_image_url(url: &str) -> String {
    normalize_remote_image_url(url)
}

fn extract_huya_room_id_from_html(html: &str) -> Option<String> {
    let patterns = [
        r#""profileRoom"\s*:\s*"?(?P<id>\d+)"?"#,
        r#"profileRoom\s*:\s*"?(?P<id>\d+)"?"#,
    ];
    for pattern in patterns {
        let regex = Regex::new(pattern).ok()?;
        if let Some(captures) = regex.captures(html) {
            if let Some(id) = captures.name("id") {
                return Some(id.as_str().to_string());
            }
        }
    }
    None
}

fn resolve_huya_room_id(target: &str) -> Result<String, String> {
    if let Some(room_id) = extract_huya_room_id(target) {
        return Ok(room_id);
    }

    let page_url = if target.trim().starts_with("http://") || target.trim().starts_with("https://")
    {
        target.trim().to_string()
    } else {
        format!("https://www.huya.com/{}", target.trim())
    };
    let client = huya_client()?;
    let html = fetch_text_with_headers(
        &client,
        &page_url,
        HUYA_USER_AGENT,
        Some("https://www.huya.com/"),
        None,
    )?;
    extract_huya_room_id_from_html(&html).ok_or_else(|| "无法识别虎牙房间号".to_string())
}

fn fetch_huya_profile_room(room_id: &str) -> Result<HuyaProfileRoomResponse, String> {
    let url = format!("https://mp.huya.com/cache.php?m=Live&do=profileRoom&roomid={room_id}");
    let client = huya_client()?;
    serde_json::from_value(fetch_json_with_headers(
        &client,
        &url,
        HUYA_USER_AGENT,
        Some(&build_huya_live_url(room_id)),
        None,
    )?)
    .map_err(|err| err.to_string())
}

fn huya_title(data: &HuyaLiveData) -> String {
    if data.room_name.trim().is_empty() {
        data.introduction.trim().to_string()
    } else {
        data.room_name.trim().to_string()
    }
}

fn is_huya_room_online(data: &HuyaProfileRoomData) -> bool {
    let live_status = data.live_status.trim().to_ascii_uppercase();
    let real_live_status = data.real_live_status.trim().to_ascii_uppercase();
    live_status == "ON" || real_live_status == "ON"
}

fn huya_heat_text(data: &HuyaLiveData) -> String {
    let total_count = data.total_count.trim();
    if !total_count.is_empty() {
        return total_count.to_string();
    }

    data.activity_count.trim().to_string()
}

fn fetch_huya_room_state(target: &str) -> Result<ExtractResult, String> {
    let room_id = resolve_huya_room_id(target)?;
    let response = fetch_huya_profile_room(&room_id)?;
    let is_online = is_huya_room_online(&response.data);
    let live_data = response.data.live_data;

    Ok(ExtractResult {
        platform: PLATFORM_HUYA.to_string(),
        room_id: if live_data.profile_room.trim().is_empty() {
            room_id.clone()
        } else {
            live_data.profile_room.clone()
        },
        streamer_name: live_data.nick.trim().to_string(),
        room_name: huya_title(&live_data),
        avatar_url: normalize_huya_image_url(&live_data.avatar_180),
        is_online,
        screenshot_url: if is_online {
            normalize_huya_image_url(&live_data.screenshot)
        } else {
            String::new()
        },
        heat_text: if is_online {
            huya_heat_text(&live_data)
        } else {
            String::new()
        },
        page_url: build_huya_live_url(&room_id),
        title: huya_title(&live_data),
        urls: Vec::new(),
    })
}

fn huya_turn_str(value: u64, radix: u32, width: usize) -> String {
    let mut text = match radix {
        2 => format!("{value:b}"),
        16 => format!("{value:x}"),
        _ => value.to_string(),
    };
    while text.len() < width {
        text.insert(0, '0');
    }
    text
}

fn huya_rot_uid(uid: u64) -> Option<u64> {
    let text = huya_turn_str(uid, 2, 64);
    let left = &text[..32];
    let right = &text[32..];
    let shift = 8;
    let rotated = format!("{}{}", &right[shift..], &right[..shift]);
    u64::from_str_radix(&format!("{left}{rotated}"), 2).ok()
}

fn huya_decode_base64_to_string(value: &str) -> Option<String> {
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(value)
        .ok()?;
    String::from_utf8(decoded).ok()
}

fn huya_parse_anti_code(anti_code: &str) -> HashMap<String, String> {
    anti_code
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn huya_ws_secret(
    anti_codes: &HashMap<String, String>,
    converted_uid: u64,
    seq_id: u64,
    stream_name: &str,
) -> Option<String> {
    let fm = anti_codes.get("fm")?;
    let ws_time = anti_codes.get("wsTime")?;
    let ctype = anti_codes.get("ctype")?;
    let t = anti_codes
        .get("t")
        .cloned()
        .unwrap_or_else(|| "100".to_string());
    let mut template = huya_decode_base64_to_string(urlencoding::decode(fm).ok()?.as_ref())?;
    let suffix = md5_hex(&format!("{seq_id}|{ctype}|{t}"));
    template = template.replace("$0", &converted_uid.to_string());
    template = template.replace("$1", stream_name);
    template = template.replace("$2", &suffix);
    template = template.replace("$3", ws_time);
    Some(md5_hex(&template))
}

fn huya_format_url(uid: u64, stream: &HuyaStreamInfo) -> Option<String> {
    if stream.s_stream_name.trim().is_empty()
        || stream.s_flv_url.trim().is_empty()
        || stream.s_flv_url_suffix.trim().is_empty()
        || stream.s_flv_anti_code.trim().is_empty()
    {
        return None;
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    let sdk_sid = now;
    let seq_id = uid + now;
    let mut anti_codes = huya_parse_anti_code(&stream.s_flv_anti_code);
    let converted_uid = huya_rot_uid(uid)?;
    let ws_secret = huya_ws_secret(&anti_codes, converted_uid, seq_id, &stream.s_stream_name)?;

    anti_codes.insert("u".to_string(), converted_uid.to_string());
    anti_codes.insert("wsSecret".to_string(), ws_secret);
    anti_codes.insert("seqid".to_string(), seq_id.to_string());
    anti_codes.insert("sdk_sid".to_string(), sdk_sid.to_string());
    anti_codes.insert("ratio".to_string(), "0".to_string());

    let example = "https://tx.flv.huya.com/huyalive/1099531627955-1099531627955-85900114719145984-2199063379366-10057-A-0-1.flv?wsSecret=42a9adedc7011adc1dbc20628eaa503f&wsTime=67b6c60d&seqid=1742352165582&ctype=huya_live&ver=1&fs=bgct&ratio=2000&dMod=mseh-8&sdkPcdn=1_1&u=1451203978&t=100&sv=2407051433&sdk_sid=1740031240996&a_block=0&sf=1";
    let example_query = Url::parse(example)
        .ok()?
        .query_pairs()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect::<Vec<_>>();

    let query = example_query
        .into_iter()
        .filter_map(|(key, value)| {
            anti_codes
                .get(&key)
                .cloned()
                .or(Some(value))
                .map(|final_value| format!("{key}={final_value}"))
        })
        .collect::<Vec<_>>()
        .join("&");

    Some(format!(
        "{}/{}.{}?{}",
        normalize_remote_image_url(&stream.s_flv_url),
        stream.s_stream_name,
        stream.s_flv_url_suffix,
        query
    ))
}

fn extract_huya_play_info(target: &str) -> Result<ExtractResult, String> {
    let mut state = fetch_huya_room_state(target)?;
    if !state.is_online {
        return Ok(state);
    }

    let room_id = state.room_id.clone();
    let response = fetch_huya_profile_room(&room_id)?;
    let stream_data = response
        .data
        .stream
        .ok_or_else(|| "未获取到虎牙直播流信息".to_string())?;

    let uid_seed = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| err.to_string())?
        .as_millis() as u64)
        % 4_294_967_295;

    let mut streams = stream_data.base_stream_info_list;
    streams.sort_by_key(|item| item.s_flv_url.contains("txdirect.flv.huya.com"));
    let urls = streams
        .iter()
        .filter_map(|stream| huya_format_url(uid_seed, stream))
        .collect::<Vec<_>>();

    if urls.is_empty() {
        return Err("未获取到可用的虎牙播放线路".to_string());
    }

    state.urls = urls;
    Ok(state)
}

fn get_bilibili_room_info(target: &str, raw_cookie: &str) -> Result<BilibiliRoomInfoData, String> {
    let room_id =
        extract_bilibili_room_id(target).ok_or_else(|| "无法识别 B 站直播间房间号".to_string())?;
    let url = format!("https://api.live.bilibili.com/room/v1/Room/get_info?room_id={room_id}");
    let response: BilibiliRoomInfoResponse = serde_json::from_value(fetch_bilibili_json(
        &url,
        &build_bilibili_live_url(&room_id),
        raw_cookie,
    )?)
    .map_err(|err| err.to_string())?;
    Ok(response.data)
}

fn get_bilibili_anchor_info(room_id: &str, raw_cookie: &str) -> Result<BilibiliAnchorInfo, String> {
    let url = format!(
        "https://api.live.bilibili.com/live_user/v1/UserInfo/get_anchor_in_room?roomid={room_id}"
    );
    let response: BilibiliAnchorInfoResponse = serde_json::from_value(fetch_bilibili_json(
        &url,
        &build_bilibili_live_url(room_id),
        raw_cookie,
    )?)
    .map_err(|err| err.to_string())?;
    Ok(response.data.info)
}

fn normalize_remote_image_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Some(rest) = trimmed.strip_prefix("http://") {
        return format!("https://{rest}");
    }
    if let Some(rest) = trimmed.strip_prefix("//") {
        return format!("https://{rest}");
    }
    trimmed.to_string()
}

fn resolve_bilibili_avatar(
    room_id: &str,
    current_avatar: &str,
    fallback_avatar: &str,
    raw_cookie: &str,
) -> String {
    if !current_avatar.trim().is_empty() {
        return normalize_remote_image_url(current_avatar);
    }

    let anchor_avatar = get_bilibili_anchor_info(room_id, raw_cookie)
        .map(|anchor| anchor.face.trim().to_string())
        .unwrap_or_default();
    if !anchor_avatar.is_empty() {
        return normalize_remote_image_url(&anchor_avatar);
    }

    normalize_remote_image_url(fallback_avatar)
}

fn bilibili_cover(room_info: &BilibiliRoomInfoData) -> String {
    if !room_info.user_cover.trim().is_empty() {
        normalize_remote_image_url(&room_info.user_cover)
    } else {
        normalize_remote_image_url(&room_info.keyframe)
    }
}

fn fetch_bilibili_room_state(target: &str, raw_cookie: &str) -> Result<ExtractResult, String> {
    let room_info = get_bilibili_room_info(target, raw_cookie)?;
    let room_id = room_info.room_id.clone();
    let anchor = get_bilibili_anchor_info(&room_id, raw_cookie).unwrap_or(BilibiliAnchorInfo {
        uname: String::new(),
        face: String::new(),
    });
    let is_online = room_info.live_status == 1;
    let room_url = build_bilibili_live_url(&room_id);

    Ok(ExtractResult {
        platform: PLATFORM_BILIBILI_LIVE.to_string(),
        room_id: room_id.clone(),
        streamer_name: if anchor.uname.trim().is_empty() {
            room_info.title.clone()
        } else {
            anchor.uname
        },
        room_name: room_info.title.clone(),
        avatar_url: normalize_remote_image_url(&anchor.face),
        is_online,
        screenshot_url: if is_online {
            bilibili_cover(&room_info)
        } else {
            String::new()
        },
        heat_text: if is_online {
            room_info.online.clone()
        } else {
            String::new()
        },
        page_url: room_url.clone(),
        title: room_info.title,
        urls: Vec::new(),
    })
}

fn bilibili_codec_urls(codec: &BilibiliPlayCodec) -> Vec<String> {
    codec
        .url_info
        .iter()
        .filter(|info| !info.host.trim().is_empty())
        .map(|info| format!("{}{}{}", info.host, codec.base_url, info.extra))
        .collect()
}

fn bilibili_cdn_level(url: &str) -> i32 {
    let Ok(parsed) = Url::parse(url) else {
        return 2;
    };
    let Some(host) = parsed.host_str() else {
        return 2;
    };

    if host.contains(".mcdn.bilivideo.cn") {
        2
    } else if host.contains(".szbdyd.com") {
        3
    } else if host.contains("bilivideo.com") && host.starts_with("up") {
        0
    } else {
        1
    }
}

fn reorder_bilibili_urls(urls: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for url in urls {
        if !deduped.contains(&url) {
            deduped.push(url);
        }
    }

    deduped.sort_by_key(|url| bilibili_cdn_level(url));
    deduped
}

fn select_bilibili_codec<'a>(
    playurl: &'a BilibiliPlayurl,
    protocol_name: &str,
    format_name: &str,
    codec_name: &str,
) -> Option<&'a BilibiliPlayCodec> {
    playurl
        .stream
        .iter()
        .find(|stream| stream.protocol_name == protocol_name)?
        .formats
        .iter()
        .find(|format| format.format_name == format_name)?
        .codecs
        .iter()
        .find(|codec| codec.codec_name == codec_name)
}

fn select_bilibili_urls(playurl: &BilibiliPlayurl) -> Vec<String> {
    let codec_preferences = [
        ("http_hls", "fmp4", "avc"),
        ("http_hls", "ts", "avc"),
        ("http_stream", "flv", "avc"),
        ("http_hls", "fmp4", "hevc"),
        ("http_hls", "ts", "hevc"),
        ("http_stream", "flv", "hevc"),
    ];
    let codec = codec_preferences
        .iter()
        .find_map(|(protocol, format, codec)| {
            select_bilibili_codec(playurl, protocol, format, codec)
        });

    codec
        .map(|codec| reorder_bilibili_urls(bilibili_codec_urls(codec)))
        .unwrap_or_default()
}

fn fetch_bilibili_room_play_info(
    room_id: &str,
    qn: i64,
    raw_cookie: &str,
) -> Result<Vec<String>, String> {
    let url = format!(
        "https://api.live.bilibili.com/xlive/web-room/v2/index/getRoomPlayInfo?room_id={room_id}&protocol=0,1&format=0,1,2&codec=0,1&qn={qn}&platform=web&ptype=8&dolby=5"
    );
    let response: BilibiliRoomPlayInfoResponse = serde_json::from_value(fetch_bilibili_json(
        &url,
        &build_bilibili_live_url(room_id),
        raw_cookie,
    )?)
    .map_err(|err| err.to_string())?;

    if response.data.encrypted && !response.data.pwd_verified {
        return Err("该 B 站直播间需要密码验证".to_string());
    }

    let urls = select_bilibili_urls(&response.data.playurl_info.playurl);
    if urls.is_empty() {
        return Err("未获取到 B 站直播播放线路".to_string());
    }
    Ok(urls)
}

fn fetch_bilibili_old_play_url(
    room_id: &str,
    qn: i64,
    raw_cookie: &str,
) -> Result<Vec<String>, String> {
    let url = format!(
        "https://api.live.bilibili.com/room/v1/Room/playUrl?cid={room_id}&qn={qn}&platform=web"
    );
    let response: BilibiliOldPlayUrlResponse = serde_json::from_value(fetch_bilibili_json(
        &url,
        &build_bilibili_live_url(room_id),
        raw_cookie,
    )?)
    .map_err(|err| err.to_string())?;
    let urls = response
        .data
        .durl
        .into_iter()
        .filter_map(|item| {
            if item.url.trim().is_empty() {
                None
            } else {
                Some(item.url)
            }
        })
        .collect::<Vec<_>>();
    if urls.is_empty() {
        return Err("未获取到 B 站旧版播放线路".to_string());
    }
    Ok(urls)
}

fn extract_json_from_script(html: &str, prefix: &str, suffix: &str) -> Option<String> {
    let start = html.find(prefix)?;
    let remain = &html[start + prefix.len()..];
    let end = remain.find(suffix)?;
    Some(remain[..end].to_string())
}

fn fetch_bilibili_html_play_url(room_id: &str, raw_cookie: &str) -> Result<Vec<String>, String> {
    let room_url = build_bilibili_live_url(room_id);
    let html = {
        let client = bilibili_client()?;
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(BILIBILI_USER_AGENT).map_err(|err| err.to_string())?,
        );
        headers.insert(
            REFERER,
            HeaderValue::from_str(&room_url).map_err(|err| err.to_string())?,
        );
        if let Some(cookie_header) = build_bilibili_cookie_header(raw_cookie)? {
            headers.insert(reqwest::header::COOKIE, cookie_header);
        }
        client
            .get(&room_url)
            .headers(headers)
            .send()
            .map_err(|err| err.to_string())?
            .text()
            .map_err(|err| err.to_string())?
    };

    let json_text = extract_json_from_script(
        &html,
        "<script>window.__NEPTUNE_IS_MY_WAIFU__=",
        "</script>",
    )
    .ok_or_else(|| "未找到 B 站直播页面初始化数据".to_string())?;
    let json: Value = serde_json::from_str(&json_text).map_err(|err| err.to_string())?;
    let playurl = &json["roomInitRes"]["data"]["playurl_info"]["playurl"];
    let playurl: BilibiliPlayurl =
        serde_json::from_value(playurl.clone()).map_err(|err| err.to_string())?;
    let urls = select_bilibili_urls(&playurl);
    if urls.is_empty() {
        return Err("未从 B 站页面解析到播放线路".to_string());
    }
    Ok(urls)
}

fn extract_bilibili_play_info(target: &str, raw_cookie: &str) -> Result<ExtractResult, String> {
    let mut state = fetch_bilibili_room_state(target, raw_cookie)?;
    if !state.is_online {
        return Ok(state);
    }
    let room_id = state.room_id.clone();

    let urls = fetch_bilibili_room_play_info(&room_id, 30000, raw_cookie)
        .or_else(|_| fetch_bilibili_old_play_url(&room_id, 30000, raw_cookie))
        .or_else(|_| fetch_bilibili_html_play_url(&room_id, raw_cookie))?;
    state.urls = urls;
    if state.title.trim().is_empty() {
        state.title = state.room_name.clone();
    }
    Ok(state)
}

fn build_douyin_search_engine_urls(keyword: &str) -> Vec<String> {
    let encoded = urlencoding::encode(keyword);
    vec![
        format!(
            "https://www.so.com/s?q={}",
            urlencoding::encode(&format!("site:live.douyin.com {keyword} 抖音 直播"))
        ),
        format!(
            "https://www.sogou.com/web?query={}",
            urlencoding::encode(&format!("site:live.douyin.com {keyword} 抖音 直播"))
        ),
        format!("https://www.sogou.com/web?query={encoded}%20抖音%20直播"),
    ]
}

fn extract_room_ids_from_text(text: &str, host: &str) -> Vec<String> {
    let pattern = format!(r"{}/(\d{{6,24}})", regex::escape(host));
    let Ok(regex) = Regex::new(&pattern) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for capture in regex.captures_iter(text) {
        if let Some(room_id) = capture.get(1) {
            let room_id = room_id.as_str().to_string();
            if !ids.contains(&room_id) {
                ids.push(room_id);
            }
        }
    }
    ids
}

fn search_douyin_candidate_room_ids(keyword: &str) -> Vec<String> {
    let handles = build_douyin_search_engine_urls(keyword)
        .into_iter()
        .map(|url| {
            std::thread::spawn(move || {
                let client = douyin_client()?;
                let text = fetch_text_with_headers(
                    &client,
                    &url,
                    DOUYIN_USER_AGENT,
                    Some("https://www.douyin.com/"),
                    None,
                )?;
                Ok::<Vec<String>, String>(extract_room_ids_from_text(&text, "live.douyin.com"))
            })
        })
        .collect::<Vec<_>>();

    let mut room_ids = Vec::new();
    for handle in handles {
        let Ok(Ok(ids)) = handle.join() else {
            continue;
        };
        for room_id in ids {
            if !room_ids.contains(&room_id) {
                room_ids.push(room_id);
            }
            if room_ids.len() >= DOUYIN_SEARCH_VERIFY_LIMIT {
                return room_ids;
            }
        }
    }
    room_ids
}

fn fetch_douyin_room_state_with_app(
    app: Option<&AppHandle>,
    target: &str,
) -> Result<ExtractResult, String> {
    let room_id =
        extract_douyin_room_id(target).ok_or_else(|| "无法识别抖音直播间房间号".to_string())?;
    let response = run_douyin_helper(app, &room_id)?;
    Ok(ExtractResult {
        platform: PLATFORM_DOUYIN_LIVE.to_string(),
        room_id: if response.room_id.trim().is_empty() {
            room_id.clone()
        } else {
            response.room_id.clone()
        },
        streamer_name: response.streamer_name.trim().to_string(),
        room_name: response.room_name.trim().to_string(),
        avatar_url: normalize_remote_image_url(&response.avatar_url),
        is_online: response.is_online,
        screenshot_url: if response.is_online {
            normalize_remote_image_url(&response.screenshot_url)
        } else {
            String::new()
        },
        heat_text: if response.is_online {
            response.heat_text.trim().to_string()
        } else {
            String::new()
        },
        page_url: if response.page_url.trim().is_empty() {
            build_douyin_live_url(&room_id)
        } else {
            build_douyin_live_url(&room_id)
        },
        title: if response.title.trim().is_empty() {
            response.room_name.trim().to_string()
        } else {
            response.title.trim().to_string()
        },
        urls: response
            .urls
            .into_iter()
            .map(|url| normalize_remote_image_url(&url))
            .collect(),
    })
}

fn fetch_room_state_for_platform_with_app(
    app: Option<&AppHandle>,
    platform: &str,
    target: &str,
) -> Result<ExtractResult, String> {
    match normalize_platform(platform) {
        PLATFORM_BILIBILI_LIVE => fetch_bilibili_room_state(target, ""),
        PLATFORM_HUYA => fetch_huya_room_state(target),
        PLATFORM_DOUYIN_LIVE => fetch_douyin_room_state_with_app(app, target),
        _ => fetch_douyu_room_state(target),
    }
}

fn extract_play_info_for_platform_with_app(
    app: Option<&AppHandle>,
    platform: &str,
    target: &str,
    bilibili_cookie: &str,
) -> Result<ExtractResult, String> {
    match normalize_platform(platform) {
        PLATFORM_BILIBILI_LIVE => extract_bilibili_play_info(target, bilibili_cookie),
        PLATFORM_HUYA => extract_huya_play_info(target),
        PLATFORM_DOUYIN_LIVE => {
            let state = fetch_douyin_room_state_with_app(app, target)?;
            if !state.is_online {
                return Ok(state);
            }
            if state.urls.is_empty() {
                return Err("未获取到抖音直播播放线路".to_string());
            }
            Ok(state)
        }
        _ => extract_douyu_play_info(target),
    }
}

fn settings_file(app: &AppHandle) -> Result<PathBuf, String> {
    app_data_file(app, "settings.json")
}

fn load_settings_inner(app: &AppHandle) -> Result<Settings, String> {
    let path = settings_file(app)?;
    let mut settings: Settings = read_json_or_default(&path)?;
    settings.danmaku_font_size = normalize_danmaku_font_size(settings.danmaku_font_size);
    Ok(settings)
}

fn save_settings_inner(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_file(app)?;
    let mut normalized = settings.clone();
    normalized.danmaku_font_size = normalize_danmaku_font_size(normalized.danmaku_font_size);
    write_json(&path, &normalized)
}

fn normalize_danmaku_font_size(value: u32) -> u32 {
    value.clamp(9, 28)
}

fn has_bilibili_login(cookie: &str) -> bool {
    cookie
        .split(';')
        .map(str::trim)
        .any(|part| part.starts_with("SESSDATA=") && part.len() > "SESSDATA=".len())
}

#[cfg(target_os = "macos")]
fn extract_bilibili_cookie_from_window<R: tauri::Runtime>(
    window: &WebviewWindow<R>,
) -> Result<Option<String>, String> {
    let (tx, rx) = mpsc::channel();
    window
        .with_webview(move |webview| unsafe {
            let view: &WKWebView = &*webview.inner().cast();
            let store = view.configuration().websiteDataStore().httpCookieStore();
            let tx = tx.clone();
            let block = RcBlock::new(move |cookies_ptr: NonNull<NSArray<NSHTTPCookie>>| {
                let cookies = cookies_ptr.as_ref();
                let mut parts = Vec::new();
                for index in 0..cookies.len() {
                    let cookie = cookies.objectAtIndex(index);
                    let domain = cookie.domain().to_string();
                    if !domain.contains("bilibili.com") {
                        continue;
                    }
                    let name = cookie.name().to_string();
                    let value = cookie.value().to_string();
                    if name.is_empty() || value.is_empty() {
                        continue;
                    }
                    parts.push(format!("{name}={value}"));
                }
                let joined = parts.join("; ");
                let _ = tx.send(joined);
            });
            store.getAllCookies(&block);
        })
        .map_err(|err| err.to_string())?;

    let cookie = rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "读取 B站 登录 Cookie 超时".to_string())?;
    if has_bilibili_login(&cookie) {
        Ok(Some(cookie))
    } else {
        Ok(None)
    }
}

#[cfg(not(target_os = "macos"))]
fn extract_bilibili_cookie_from_window<R: tauri::Runtime>(
    _window: &WebviewWindow<R>,
) -> Result<Option<String>, String> {
    Ok(None)
}

fn save_bilibili_cookie_and_notify(app: &AppHandle, cookie: String) -> Result<Settings, String> {
    let mut settings = load_settings_inner(app)?;
    settings.bilibili_cookie = cookie;
    save_settings_inner(app, &settings)?;
    app.emit("bilibili-login-updated", &settings)
        .map_err(|err| err.to_string())?;
    Ok(settings)
}

fn clear_bilibili_cookie_and_notify(app: &AppHandle) -> Result<Settings, String> {
    let mut settings = load_settings_inner(app)?;
    settings.bilibili_cookie.clear();
    save_settings_inner(app, &settings)?;
    app.emit("bilibili-login-updated", &settings)
        .map_err(|err| err.to_string())?;
    Ok(settings)
}

fn maybe_capture_bilibili_login<R: tauri::Runtime>(app: AppHandle, window: WebviewWindow<R>) {
    match extract_bilibili_cookie_from_window(&window) {
        Ok(Some(cookie)) => {
            if save_bilibili_cookie_and_notify(&app, cookie).is_ok() {
                let _ = window.close();
            }
        }
        Ok(None) => {}
        Err(err) => {
            let _ = app.emit("bilibili-login-error", err);
        }
    }
}

fn bilibili_login_url() -> &'static str {
    "https://passport.bilibili.com/login"
}

fn spawn_bilibili_login_watcher(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        for _ in 0..180 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let Some(window) = app.get_webview_window("bilibili-login") else {
                break;
            };

            match extract_bilibili_cookie_from_window(&window) {
                Ok(Some(cookie)) => {
                    if save_bilibili_cookie_and_notify(&app, cookie).is_ok() {
                        let _ = window.close();
                    }
                    break;
                }
                Ok(None) => {}
                Err(err) => {
                    let _ = app.emit("bilibili-login-error", err);
                }
            }
        }
    });
}

fn register_stream_proxy_sessions(
    app: &AppHandle,
    settings: &Settings,
    data: &ExtractResult,
    port: u16,
) -> Result<Vec<String>, String> {
    let state = app.state::<Arc<DanmakuServerState>>();
    let headers = stream_proxy_headers_for_platform(&data.platform, settings)?;
    let mut sessions = state
        .stream_sessions
        .lock()
        .map_err(|_| "stream proxy 状态锁已损坏".to_string())?;

    let mut proxy_urls = Vec::with_capacity(data.urls.len());
    for url in &data.urls {
        let session_id = Uuid::new_v4().to_string();
        eprintln!("[stream-hub:proxy] register session {} for {}", session_id, url);
        sessions.insert(
            session_id.clone(),
            StreamProxySession {
                url: url.clone(),
                headers: headers.clone(),
            },
        );
        proxy_urls.push(format!("http://127.0.0.1:{port}/stream-proxy/{session_id}"));
    }

    eprintln!("[stream-hub:proxy] total sessions after register: {}", sessions.len());
    Ok(proxy_urls)
}

fn stream_proxy_headers_for_platform(
    platform: &str,
    settings: &Settings,
) -> Result<Vec<(String, String)>, String> {
    let mut headers = Vec::new();
    match platform {
        PLATFORM_BILIBILI_LIVE => {
            headers.push(("user-agent".to_string(), BILIBILI_USER_AGENT.to_string()));
            headers.push(("referer".to_string(), "https://live.bilibili.com/".to_string()));
            if let Some(cookie) = build_bilibili_cookie_string(&settings.bilibili_cookie)? {
                headers.push(("cookie".to_string(), cookie));
            }
        }
        PLATFORM_HUYA => {
            headers.push(("user-agent".to_string(), HUYA_USER_AGENT.to_string()));
            headers.push(("referer".to_string(), "https://www.huya.com/".to_string()));
        }
        PLATFORM_DOUYIN_LIVE => {
            headers.push(("user-agent".to_string(), DOUYIN_USER_AGENT.to_string()));
            headers.push(("referer".to_string(), "https://live.douyin.com/".to_string()));
        }
        _ => {
            headers.push(("user-agent".to_string(), DOUYU_USER_AGENT.to_string()));
            headers.push(("referer".to_string(), "https://www.douyu.com/".to_string()));
        }
    }
    headers.push(("accept".to_string(), "*/*".to_string()));
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::{
        huya_heat_text, is_huya_room_online, normalize_danmaku_font_size, select_bilibili_urls,
        BilibiliPlayCodec, BilibiliPlayFormat, BilibiliPlayStream, BilibiliPlayurl,
        BilibiliUrlInfo, HuyaProfileRoomResponse,
    };

    fn bilibili_codec(
        protocol: &str,
        format: &str,
        codec: &str,
        base_url: &str,
    ) -> BilibiliPlayStream {
        BilibiliPlayStream {
            protocol_name: protocol.to_string(),
            formats: vec![BilibiliPlayFormat {
                format_name: format.to_string(),
                codecs: vec![BilibiliPlayCodec {
                    codec_name: codec.to_string(),
                    current_qn: 250,
                    accept_qn: vec![250],
                    base_url: base_url.to_string(),
                    url_info: vec![BilibiliUrlInfo {
                        host: "https://example.bilivideo.com".to_string(),
                        extra: "?token=test".to_string(),
                    }],
                }],
            }],
        }
    }

    #[test]
    fn normalize_danmaku_font_size_clamps_to_supported_range() {
        assert_eq!(normalize_danmaku_font_size(6), 9);
        assert_eq!(normalize_danmaku_font_size(18), 18);
        assert_eq!(normalize_danmaku_font_size(72), 28);
    }

    #[test]
    fn parses_huya_nested_live_status() {
        let response: HuyaProfileRoomResponse = serde_json::from_value(serde_json::json!({
            "data": {
                "liveStatus": "ON",
                "realLiveStatus": "ON",
                "liveData": {
                    "nick": "测试主播",
                    "profileRoom": 8826
                }
            }
        }))
        .expect("huya response should deserialize");

        assert!(is_huya_room_online(&response.data));
    }

    #[test]
    fn parses_huya_heat_from_total_count() {
        let response: HuyaProfileRoomResponse = serde_json::from_value(serde_json::json!({
            "data": {
                "liveData": {
                    "totalCount": 237519,
                    "activityCount": 4757927
                }
            }
        }))
        .expect("huya response should deserialize");

        assert_eq!(huya_heat_text(&response.data.live_data), "237519");
    }

    #[test]
    fn falls_back_huya_heat_to_activity_count() {
        let response: HuyaProfileRoomResponse = serde_json::from_value(serde_json::json!({
            "data": {
                "liveData": {
                    "activityCount": "4757927"
                }
            }
        }))
        .expect("huya response should deserialize");

        assert_eq!(huya_heat_text(&response.data.live_data), "4757927");
    }

    #[test]
    fn bilibili_play_urls_prefer_hls_fmp4_over_flv() {
        let playurl = BilibiliPlayurl {
            stream: vec![
                bilibili_codec("http_stream", "flv", "avc", "/live/test.flv"),
                bilibili_codec("http_hls", "fmp4", "avc", "/live/test/index.m3u8"),
            ],
        };

        let urls = select_bilibili_urls(&playurl);

        assert_eq!(
            urls.first().map(String::as_str),
            Some("https://example.bilivideo.com/live/test/index.m3u8?token=test")
        );
    }

    #[test]
    fn bilibili_play_urls_fall_back_to_flv_when_hls_missing() {
        let playurl = BilibiliPlayurl {
            stream: vec![bilibili_codec(
                "http_stream",
                "flv",
                "avc",
                "/live/test.flv",
            )],
        };

        let urls = select_bilibili_urls(&playurl);

        assert_eq!(
            urls.first().map(String::as_str),
            Some("https://example.bilivideo.com/live/test.flv?token=test")
        );
    }
}

fn detect_node_binary() -> Result<String, String> {
    let candidates = ["node", "/opt/homebrew/bin/node", "/usr/local/bin/node"];

    for candidate in candidates {
        let mut cmd = Command::new(candidate);
        cmd.arg("--version");
        if let Ok(status) = cmd.status() {
            if status.success() {
                return Ok(candidate.to_string());
            }
        }
    }

    Err("未找到可用的 Node.js，弹幕桥接无法启动。".to_string())
}

fn locate_danmaku_bridge_script(app: &AppHandle) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();

    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("douyu_danmaku_bridge.js"));
    }

    candidates
        .push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/douyu_danmaku_bridge.js"));

    if let Ok(current_dir) = env::current_dir() {
        candidates.push(current_dir.join("src-tauri/resources/douyu_danmaku_bridge.js"));
        candidates.push(current_dir.join("resources/douyu_danmaku_bridge.js"));
    }

    candidates
        .into_iter()
        .find(|path| path.exists() && path.is_file())
        .ok_or_else(|| "未找到弹幕桥接脚本。".to_string())
}

fn locate_resource_file(app: &AppHandle, file_name: &str) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();

    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("resources").join(file_name));
        candidates.push(resource_dir.join(file_name));
    }

    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join(file_name),
    );

    if let Ok(current_dir) = env::current_dir() {
        candidates.push(current_dir.join("src-tauri/resources").join(file_name));
        candidates.push(current_dir.join("resources").join(file_name));
    }

    candidates
        .into_iter()
        .find(|path| path.exists() && path.is_file())
        .ok_or_else(|| format!("未找到资源文件：{file_name}"))
}

fn resource_file_to_data_url(app: &AppHandle, file_name: &str) -> Result<String, String> {
    let path = locate_resource_file(app, file_name)?;
    let bytes = fs::read(path).map_err(|err| err.to_string())?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Ok(format!("data:image/x-icon;base64,{encoded}"))
}

fn locate_douyin_helper_script(app: &AppHandle) -> Result<PathBuf, String> {
    locate_resource_file(app, "douyin_live_helper.js")
}

fn run_douyin_helper(
    app: Option<&AppHandle>,
    room_id: &str,
) -> Result<DouyinHelperResponse, String> {
    let node_bin = detect_node_binary()?;
    let script = if let Some(app) = app {
        locate_douyin_helper_script(app)?
    } else {
        let mut candidates = Vec::new();
        candidates.push(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/douyin_live_helper.js"),
        );
        if let Ok(current_dir) = env::current_dir() {
            candidates.push(current_dir.join("src-tauri/resources/douyin_live_helper.js"));
            candidates.push(current_dir.join("resources/douyin_live_helper.js"));
        }
        candidates
            .into_iter()
            .find(|path| path.exists() && path.is_file())
            .ok_or_else(|| "未找到抖音直播解析脚本。".to_string())?
    };

    let output = Command::new(node_bin)
        .arg(script)
        .arg(room_id)
        .output()
        .map_err(|err| err.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "抖音直播解析失败".to_string()
        } else {
            stderr
        });
    }

    let stdout = String::from_utf8(output.stdout).map_err(|err| err.to_string())?;
    serde_json::from_str(stdout.trim()).map_err(|err| err.to_string())
}

async fn prepare_douyin_danmaku(
    state: &DanmakuHttpState,
    room_id: &str,
) -> Result<DouyinDanmakuResponse, String> {
    let output = TokioCommand::new(&state.node_bin)
        .arg(&state.douyin_helper_script)
        .arg("--danmaku")
        .arg(room_id)
        .output()
        .await
        .map_err(|err| err.to_string())?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "抖音弹幕签名失败".to_string()
        } else {
            stderr
        });
    }

    let stdout = String::from_utf8(output.stdout).map_err(|err| err.to_string())?;
    serde_json::from_str(stdout.trim()).map_err(|err| err.to_string())
}

fn read_proto_varint(data: &[u8], cursor: &mut usize) -> Result<u64, String> {
    let mut result = 0u64;
    let mut shift = 0u32;
    while *cursor < data.len() {
        let byte = data[*cursor];
        *cursor += 1;
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift >= 64 {
            return Err("protobuf varint 过长".to_string());
        }
    }
    Err("protobuf varint 不完整".to_string())
}

fn read_proto_bytes<'a>(data: &'a [u8], cursor: &mut usize) -> Result<&'a [u8], String> {
    let length = read_proto_varint(data, cursor)? as usize;
    if *cursor + length > data.len() {
        return Err("protobuf bytes 不完整".to_string());
    }
    let bytes = &data[*cursor..*cursor + length];
    *cursor += length;
    Ok(bytes)
}

fn skip_proto_field(data: &[u8], cursor: &mut usize, wire_type: u64) -> Result<(), String> {
    match wire_type {
        0 => {
            let _ = read_proto_varint(data, cursor)?;
        }
        1 => {
            if *cursor + 8 > data.len() {
                return Err("protobuf fixed64 不完整".to_string());
            }
            *cursor += 8;
        }
        2 => {
            let _ = read_proto_bytes(data, cursor)?;
        }
        5 => {
            if *cursor + 4 > data.len() {
                return Err("protobuf fixed32 不完整".to_string());
            }
            *cursor += 4;
        }
        _ => return Err(format!("不支持的 protobuf wire type: {wire_type}")),
    }
    Ok(())
}

fn decode_proto_string(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap_or_default()
}

fn decode_douyin_outer_frame(data: &[u8]) -> Result<(u64, Vec<u8>), String> {
    let mut cursor = 0usize;
    let mut log_id = 0u64;
    let mut payload = Vec::new();

    while cursor < data.len() {
        let key = read_proto_varint(data, &mut cursor)?;
        let field = key >> 3;
        let wire_type = key & 0x07;
        match (field, wire_type) {
            (3, 0) => log_id = read_proto_varint(data, &mut cursor)?,
            (8, 2) => payload = read_proto_bytes(data, &mut cursor)?.to_vec(),
            _ => skip_proto_field(data, &mut cursor, wire_type)?,
        }
    }

    Ok((log_id, payload))
}

fn decode_douyin_message(data: &[u8]) -> Result<(String, Vec<u8>), String> {
    let mut cursor = 0usize;
    let mut method = String::new();
    let mut payload = Vec::new();

    while cursor < data.len() {
        let key = read_proto_varint(data, &mut cursor)?;
        let field = key >> 3;
        let wire_type = key & 0x07;
        match (field, wire_type) {
            (1, 2) => method = decode_proto_string(read_proto_bytes(data, &mut cursor)?),
            (2, 2) => payload = read_proto_bytes(data, &mut cursor)?.to_vec(),
            _ => skip_proto_field(data, &mut cursor, wire_type)?,
        }
    }

    Ok((method, payload))
}

fn decode_douyin_chat_content(data: &[u8]) -> Result<String, String> {
    let mut cursor = 0usize;

    while cursor < data.len() {
        let key = read_proto_varint(data, &mut cursor)?;
        let field = key >> 3;
        let wire_type = key & 0x07;
        match (field, wire_type) {
            (3, 2) => return Ok(decode_proto_string(read_proto_bytes(data, &mut cursor)?)),
            _ => skip_proto_field(data, &mut cursor, wire_type)?,
        }
    }

    Ok(String::new())
}

fn decode_douyin_response(data: &[u8], log_id: u64) -> Result<DouyinDanmakuBatch, String> {
    let mut cursor = 0usize;
    let mut comments = Vec::new();
    let mut internal_ext = String::new();
    let mut need_ack = false;

    while cursor < data.len() {
        let key = read_proto_varint(data, &mut cursor)?;
        let field = key >> 3;
        let wire_type = key & 0x07;
        match (field, wire_type) {
            (1, 2) => {
                let (method, payload) =
                    decode_douyin_message(read_proto_bytes(data, &mut cursor)?)?;
                if method == "WebcastChatMessage" {
                    let text = decode_douyin_chat_content(&payload)?.trim().to_string();
                    if !text.is_empty() {
                        comments.push(text);
                    }
                }
            }
            (5, 2) => internal_ext = decode_proto_string(read_proto_bytes(data, &mut cursor)?),
            (9, 0) => need_ack = read_proto_varint(data, &mut cursor)? != 0,
            _ => skip_proto_field(data, &mut cursor, wire_type)?,
        }
    }

    Ok(DouyinDanmakuBatch {
        comments,
        need_ack,
        ack_payload: internal_ext.into_bytes(),
        log_id,
    })
}

fn decode_douyin_danmaku_batch(data: &[u8]) -> Result<DouyinDanmakuBatch, String> {
    let (log_id, payload) = decode_douyin_outer_frame(data)?;
    if payload.is_empty() {
        return Ok(DouyinDanmakuBatch::default());
    }

    let mut decoder = GzDecoder::new(payload.as_slice());
    let mut decoded = Vec::new();
    if decoder.read_to_end(&mut decoded).is_err() {
        decoded = payload;
    }

    decode_douyin_response(&decoded, log_id)
}

fn write_proto_varint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 0x80 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

fn write_proto_bytes_field(field: u64, bytes: &[u8], output: &mut Vec<u8>) {
    write_proto_varint((field << 3) | 2, output);
    write_proto_varint(bytes.len() as u64, output);
    output.extend_from_slice(bytes);
}

fn write_proto_string_field(field: u64, value: &str, output: &mut Vec<u8>) {
    write_proto_bytes_field(field, value.as_bytes(), output);
}

fn write_proto_varint_field(field: u64, value: u64, output: &mut Vec<u8>) {
    write_proto_varint(field << 3, output);
    write_proto_varint(value, output);
}

fn encode_douyin_push_frame(payload_type: &str, log_id: u64, payload: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    if log_id > 0 {
        write_proto_varint_field(2, log_id, &mut output);
    }
    write_proto_string_field(7, payload_type, &mut output);
    if !payload.is_empty() {
        write_proto_bytes_field(8, payload, &mut output);
    }
    output
}

fn ensure_danmaku_server_running(app: &AppHandle) -> Result<u16, String> {
    let state = app.state::<Arc<DanmakuServerState>>();
    if state.started.swap(true, Ordering::SeqCst) {
        return Ok(state.port);
    }

    let port = state.port;
    let http_state = DanmakuHttpState {
        node_bin: detect_node_binary()?,
        bridge_script: locate_danmaku_bridge_script(app)?,
        douyin_helper_script: locate_douyin_helper_script(app)?,
        stream_sessions: Arc::clone(&state.stream_sessions),
    };
    let state_ref = Arc::clone(&state.inner().clone());
    tauri::async_runtime::spawn(async move {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => run_danmaku_server(listener, http_state).await,
            Err(_) => {
                state_ref.started.store(false, Ordering::SeqCst);
            }
        }
    });
    Ok(port)
}

async fn run_danmaku_server(listener: TcpListener, state: DanmakuHttpState) {
    let app = Router::new()
        .route("/danmaku-websocket", get(handle_danmaku_websocket))
        .route("/stream-proxy/:session_id", get(handle_stream_proxy))
        .route("/stream-proxy/:session_id/*path", get(handle_stream_proxy_segment))
        /* fallback 404 handler - log unmatched paths */
        .fallback(|req: axum::http::Request<axum::body::Body>| async move {
            let path = req.uri().path().to_string();
            let method = req.method().to_string();
            eprintln!("[stream-hub:proxy] unmatched {} {}", method, path);
            let mut headers = HeaderMap::new();
            headers.insert("access-control-allow-origin", AxumHeaderValue::from_static("*"));
            (StatusCode::NOT_FOUND, headers, format!("no route for {} {}", method, path))
        })
        .with_state(state);
    let _ = axum::serve(listener, app).await;
}

async fn handle_stream_proxy(
    State(state): State<DanmakuHttpState>,
    AxumPath(session_id): AxumPath<String>,
) -> impl IntoResponse {
    proxy_stream(&state, &session_id, "").await
}

async fn handle_stream_proxy_segment(
    State(state): State<DanmakuHttpState>,
    AxumPath((session_id, path)): AxumPath<(String, String)>,
) -> impl IntoResponse {
    proxy_stream(&state, &session_id, &path).await
}

async fn proxy_stream(
    state: &DanmakuHttpState,
    session_id: &str,
    path: &str,
) -> impl IntoResponse {
    fn cors_headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("access-control-allow-origin", AxumHeaderValue::from_static("*"));
        h
    }

    fn build_client() -> reqwest::Client {
        reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .expect("reqwest::Client::builder().build()")
    }

    fn base_url_of(url: &str) -> String {
        let without_qs = url.split('?').next().unwrap_or(url);
        if let Some(last_slash) = without_qs.rfind('/') {
            without_qs[..=last_slash].to_string()
        } else {
            format!("{}/", without_qs)
        }
    }

    let session = match state.stream_sessions.lock() {
        Ok(sessions) => {
            eprintln!("[stream-hub:proxy] lookup session {} total={}", session_id, sessions.len());
            match sessions.get(session_id).cloned() {
                Some(s) => s,
                None => return (StatusCode::NOT_FOUND, cors_headers(), Body::from("session not found")),
            }
        }
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, cors_headers(), Body::from("lock error")),
    };

    let target_url = if path.is_empty() {
        session.url.clone()
    } else if path.contains(".m3u8") {
        session.url.clone()
    } else {
        let base = base_url_of(&session.url);
        let seg = path.trim_start_matches('/');
        format!("{}{}", base, seg)
    };

    let client = build_client();
    let mut request = client.get(&target_url);
    for (name, value) in session.headers {
        request = request.header(name, value);
    }
    let response = match request.send().await {
        Ok(r) => r,
        Err(err) => {
            eprintln!("[stream-hub:proxy] upstream request failed: {:?}", err);
            return (StatusCode::BAD_GATEWAY, cors_headers(), Body::from(err.to_string()))
        }
    };
    let status = response.status();
    if !status.is_success() {
        eprintln!("[stream-hub:proxy] upstream returned status {} for {}", status, target_url);
        return (
            StatusCode::BAD_GATEWAY,
            cors_headers(),
            Body::from(format!("上游直播流返回状态码：{status}")),
        );
    }

    let mut headers = cors_headers();
    headers.insert("cache-control", AxumHeaderValue::from_static("no-store"));
    if let Some(content_type) = response.headers().get(CONTENT_TYPE) {
        if let Ok(value) = AxumHeaderValue::from_str(content_type.to_str().unwrap_or_default()) {
            headers.insert("content-type", value);
        }
    }

    (StatusCode::OK, headers, Body::from_stream(response.bytes_stream()))
}



async fn handle_danmaku_websocket(
    State(state): State<DanmakuHttpState>,
    ws: WebSocketUpgrade,
    Query(query): Query<DanmakuWsQuery>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        let _ = proxy_live_danmaku(socket, query.platform, query.room_id, state).await;
    })
}

fn parse_danmaku_target_from_client_message(message: &str) -> Option<(String, String)> {
    let payload = message;
    let platform = infer_platform_from_target(payload).to_string();
    let room_id = match platform.as_str() {
        PLATFORM_BILIBILI_LIVE => extract_bilibili_room_id(payload),
        PLATFORM_HUYA => extract_huya_room_id(payload),
        PLATFORM_DOUYIN_LIVE => extract_douyin_room_id(payload),
        _ => extract_room_id_from_target(payload),
    }?;
    Some((platform, room_id))
}

async fn proxy_douyin_danmaku(
    mut client_stream: WebSocket,
    room_id: String,
    state: DanmakuHttpState,
) -> Result<(), String> {
    let session = prepare_douyin_danmaku(&state, &room_id).await?;
    if session.ws_url.trim().is_empty() {
        return Err("抖音弹幕 WebSocket 地址为空".to_string());
    }

    let mut request = session
        .ws_url
        .as_str()
        .into_client_request()
        .map_err(|err| err.to_string())?;
    request.headers_mut().insert(
        "Cookie",
        WsHeaderValue::from_str(&session.cookie).map_err(|err| err.to_string())?,
    );
    request.headers_mut().insert(
        "User-Agent",
        WsHeaderValue::from_str(&session.user_agent).map_err(|err| err.to_string())?,
    );
    request.headers_mut().insert(
        "referer",
        WsHeaderValue::from_str(if session.referer.trim().is_empty() {
            "https://live.douyin.com"
        } else {
            session.referer.trim()
        })
        .map_err(|err| err.to_string())?,
    );

    let (ws_stream, _) = connect_async(request)
        .await
        .map_err(|err| err.to_string())?;
    let (mut write, mut read) = ws_stream.split();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));

    client_stream
        .send(AxumMessage::Text(danmaku_text_event(
            "status",
            "Stream Hub 弹幕已连接",
        )?))
        .await
        .map_err(|err| err.to_string())?;

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                let payload = encode_douyin_push_frame("hb", 0, &[]);
                if write.send(TungsteniteMessage::Ping(payload)).await.is_err() {
                    break;
                }
            }
            message = read.next() => {
                let Some(message) = message else {
                    break;
                };
                match message.map_err(|err| err.to_string())? {
                    TungsteniteMessage::Binary(data) => {
                        let batch = decode_douyin_danmaku_batch(&data)?;
                        for text in batch.comments {
                            if client_stream
                                .send(AxumMessage::Text(danmaku_text_event("chat", text)?))
                                .await
                                .is_err()
                            {
                                let _ = write.close().await;
                                return Ok(());
                            }
                        }

                        if batch.need_ack {
                            let ack = encode_douyin_push_frame("ack", batch.log_id, &batch.ack_payload);
                            let _ = write.send(TungsteniteMessage::Binary(ack)).await;
                        }
                    }
                    TungsteniteMessage::Ping(payload) => {
                        let _ = write.send(TungsteniteMessage::Pong(payload)).await;
                    }
                    TungsteniteMessage::Close(_) => break,
                    _ => {}
                }
            }
        }
    }

    Ok(())
}

async fn proxy_live_danmaku(
    mut client_stream: WebSocket,
    initial_platform: String,
    initial_room_id: String,
    state: DanmakuHttpState,
) -> Result<(), String> {
    client_stream
        .send(AxumMessage::Text(danmaku_text_event(
            "status",
            "Stream Hub 本地弹幕服务已连接",
        )?))
        .await
        .map_err(|err| err.to_string())?;

    let (platform, room_id) = if !initial_room_id.trim().is_empty() {
        (
            normalize_platform(&initial_platform).to_string(),
            initial_room_id,
        )
    } else {
        let mut parsed_room_id = String::new();
        let mut parsed_platform = PLATFORM_DOUYU.to_string();
        while let Some(Ok(message)) = client_stream.next().await {
            match message {
                AxumMessage::Text(text) => {
                    if let Some((platform, room_id)) =
                        parse_danmaku_target_from_client_message(&text)
                    {
                        parsed_platform = platform;
                        parsed_room_id = room_id;
                        break;
                    }
                }
                AxumMessage::Close(_) => return Ok(()),
                _ => {}
            }
        }
        (parsed_platform, parsed_room_id)
    };

    if room_id.is_empty() {
        let _ = client_stream
            .send(AxumMessage::Text(danmaku_text_event(
                "error",
                "Stream Hub 未能解析房间号",
            )?))
            .await;
        return Err("无法确定弹幕房间号".to_string());
    }

    if platform == PLATFORM_DOUYIN_LIVE {
        return proxy_douyin_danmaku(client_stream, room_id, state).await;
    }

    let mut child = TokioCommand::new(&state.node_bin)
        .arg(&state.bridge_script)
        .arg(&platform)
        .arg(&room_id)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| err.to_string())?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "弹幕桥接未输出 stdout。".to_string())?;
    let mut lines = BufReader::new(stdout).lines();

    while let Some(line) = lines.next_line().await.map_err(|err| err.to_string())? {
        let event: DanmakuBridgeEvent = match serde_json::from_str(&line) {
            Ok(event) => event,
            Err(_) => continue,
        };

        let outgoing = match event.event_type.as_str() {
            "status" => danmaku_text_event("status", event.text)?,
            "chat" => danmaku_text_event("chat", event.text)?,
            "error" => {
                danmaku_text_event("error", format!("Stream Hub 弹幕桥接失败: {}", event.text))?
            }
            _ => continue,
        };

        if client_stream
            .send(AxumMessage::Text(outgoing))
            .await
            .is_err()
        {
            let _ = child.kill().await;
            break;
        }
    }

    let _ = child.kill().await;
    Ok(())
}

fn search_douyu_streamers(keyword: &str) -> Result<Vec<SearchStreamer>, String> {
    let query = keyword.trim();
    if query.is_empty() {
        return Err("Missing keyword".into());
    }

    let url = format!(
        "https://www.douyu.com/japi/search/api/searchUser?kw={}&page=1&pageSize=30",
        urlencoding::encode(query)
    );
    let response: SearchApiUserResponse =
        serde_json::from_value(fetch_search_json(&url, query)?).map_err(|err| err.to_string())?;

    let mut results = Vec::new();
    for item in response.data.relate_user {
        let anchor = item.anchor_info;
        let room_id = anchor.room_id;
        let name = anchor.nick_name;
        if room_id.is_empty() || name.is_empty() {
            continue;
        }

        let result = SearchStreamer {
            name,
            platform: PLATFORM_DOUYU.to_string(),
            target: room_id.clone(),
            room_id,
            room_name: anchor.description,
            avatar_url: anchor.avatar,
            is_online: false,
            screenshot_url: String::new(),
            heat_text: String::new(),
        };

        results.push(result);
    }

    let room_ids = results
        .iter()
        .map(|result| result.room_id.clone())
        .collect::<Vec<_>>();
    let room_snapshots = fetch_douyu_room_snapshots_parallel(&room_ids);
    for result in results.iter_mut() {
        if let Some(snapshot) = room_snapshots.get(&result.room_id) {
            if !snapshot.streamer_name.trim().is_empty() {
                result.name = snapshot.streamer_name.clone();
            }
            if !snapshot.room_name.trim().is_empty() {
                result.room_name = snapshot.room_name.clone();
            }
            if !snapshot.avatar_url.trim().is_empty() {
                result.avatar_url = snapshot.avatar_url.clone();
            }
            result.is_online = snapshot.is_online;
            result.screenshot_url = snapshot.screenshot_url.clone();
            result.heat_text = snapshot.heat_text.clone();
        }
    }

    Ok(results)
}

fn fetch_douyu_room_snapshots_parallel(room_ids: &[String]) -> HashMap<String, RoomSnapshot> {
    let handles = room_ids
        .iter()
        .filter(|room_id| !room_id.trim().is_empty())
        .cloned()
        .map(|room_id| {
            std::thread::spawn(move || {
                let snapshot = get_room_snapshot(&room_id).ok()?;
                Some((room_id, snapshot))
            })
        })
        .collect::<Vec<_>>();

    handles
        .into_iter()
        .filter_map(|handle| handle.join().ok().flatten())
        .collect()
}

fn search_bilibili_streamers(keyword: &str) -> Result<Vec<SearchStreamer>, String> {
    let query = keyword.trim();
    if query.is_empty() {
        return Err("Missing keyword".into());
    }

    let items = fetch_bilibili_search(query)?;
    let room_ids = items
        .iter()
        .map(|item| item.roomid.clone())
        .filter(|room_id| !room_id.trim().is_empty())
        .collect::<Vec<_>>();
    let room_infos = fetch_bilibili_room_base_infos(&room_ids).unwrap_or_default();

    let mut results = Vec::new();
    for item in items {
        if item.roomid.trim().is_empty() || item.uname.trim().is_empty() {
            continue;
        }
        let room_id = item.roomid.trim().to_string();
        let avatar_url = normalize_remote_image_url(&item.uface);
        let room_info = room_infos
            .get(&room_id)
            .or_else(|| room_infos.values().find(|info| info.short_id == room_id));
        let is_online = room_info
            .map(|info| info.live_status == 1)
            .unwrap_or(item.live_status == 1);
        results.push(SearchStreamer {
            name: strip_html_tags(&item.uname).trim().to_string(),
            platform: PLATFORM_BILIBILI_LIVE.to_string(),
            target: build_bilibili_live_url(&room_id),
            room_id,
            room_name: room_info.map(|info| info.title.clone()).unwrap_or_default(),
            avatar_url,
            is_online,
            screenshot_url: if is_online {
                room_info
                    .map(|info| normalize_remote_image_url(&info.cover))
                    .unwrap_or_default()
            } else {
                String::new()
            },
            heat_text: if is_online {
                room_info
                    .map(|info| info.online.clone())
                    .unwrap_or_default()
            } else {
                String::new()
            },
        });
    }
    Ok(results)
}

fn search_huya_streamers(keyword: &str) -> Result<Vec<SearchStreamer>, String> {
    let query = keyword.trim();
    if query.is_empty() {
        return Err("Missing keyword".into());
    }

    let search_url = format!(
        "https://www.huya.com/search?hsk={}",
        urlencoding::encode(query)
    );
    let client = huya_client()?;
    let html = fetch_text_with_headers(
        &client,
        &search_url,
        DOUYU_USER_AGENT,
        Some("https://www.huya.com/"),
        None,
    )?;

    let item_regex = Regex::new(
        r#"(?s)<li title="房间号：(?P<room_id>\d+)" class="host-item">(?P<body>.*?)</li>"#,
    )
    .map_err(|err| err.to_string())?;
    let avatar_regex =
        Regex::new(r#"<img src="(?P<avatar>[^"]+)""#).map_err(|err| err.to_string())?;
    let nick_regex =
        Regex::new(r#"<div class="nick">(?P<nick>.*?)</div>"#).map_err(|err| err.to_string())?;
    let desc_regex =
        Regex::new(r#"<div class="desc">(?P<desc>.*?)</div>"#).map_err(|err| err.to_string())?;

    let mut candidates = Vec::new();
    for captures in item_regex.captures_iter(&html).take(12) {
        let room_id = captures
            .name("room_id")
            .map(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let body = captures
            .name("body")
            .map(|value| value.as_str())
            .unwrap_or_default();
        if room_id.is_empty() {
            continue;
        }

        let name = nick_regex
            .captures(body)
            .and_then(|capture| capture.name("nick"))
            .map(|value| strip_html_tags(value.as_str()).trim().to_string())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }

        let avatar_url = avatar_regex
            .captures(body)
            .and_then(|capture| capture.name("avatar"))
            .map(|value| normalize_huya_image_url(value.as_str()))
            .unwrap_or_default();
        let description = desc_regex
            .captures(body)
            .and_then(|capture| capture.name("desc"))
            .map(|value| strip_html_tags(value.as_str()).trim().to_string())
            .unwrap_or_default();

        candidates.push((room_id, name, avatar_url, description));
    }

    let state_handles = candidates
        .iter()
        .take(HUYA_SEARCH_STATUS_LIMIT)
        .map(|(room_id, _, _, _)| {
            let room_id = room_id.clone();
            std::thread::spawn(move || fetch_huya_room_state(&room_id).ok())
        })
        .collect::<Vec<_>>();
    let states = state_handles
        .into_iter()
        .map(|handle| handle.join().unwrap_or(None))
        .collect::<Vec<_>>();

    let mut results = Vec::new();
    for (index, (room_id, name, avatar_url, description)) in candidates.into_iter().enumerate() {
        let state = states.get(index).cloned().unwrap_or(None);
        results.push(SearchStreamer {
            name: state
                .as_ref()
                .map(|value| {
                    if value.streamer_name.trim().is_empty() {
                        name.clone()
                    } else {
                        value.streamer_name.clone()
                    }
                })
                .unwrap_or_else(|| name.clone()),
            platform: PLATFORM_HUYA.to_string(),
            target: build_huya_live_url(&room_id),
            room_id: room_id.clone(),
            room_name: state
                .as_ref()
                .map(|value| value.room_name.clone())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(description),
            avatar_url: state
                .as_ref()
                .map(|value| value.avatar_url.clone())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(avatar_url),
            is_online: state.as_ref().map(|value| value.is_online).unwrap_or(false),
            screenshot_url: state
                .as_ref()
                .map(|value| value.screenshot_url.clone())
                .unwrap_or_default(),
            heat_text: state
                .as_ref()
                .map(|value| value.heat_text.clone())
                .unwrap_or_default(),
        });
    }

    Ok(results)
}

fn search_douyin_streamers_with_app(
    app: Option<&AppHandle>,
    keyword: &str,
) -> Result<Vec<SearchStreamer>, String> {
    let query = keyword.trim();
    if query.is_empty() {
        return Err("Missing keyword".into());
    }

    let app = app.cloned();
    let state_handles = search_douyin_candidate_room_ids(query)
        .into_iter()
        .take(DOUYIN_SEARCH_VERIFY_LIMIT)
        .map(|room_id| {
            let app = app.clone();
            std::thread::spawn(move || {
                fetch_douyin_room_state_with_app(app.as_ref(), &build_douyin_live_url(&room_id))
                    .ok()
            })
        })
        .collect::<Vec<_>>();

    let mut results = Vec::new();
    for handle in state_handles {
        let Ok(Some(state)) = handle.join() else {
            continue;
        };
        let matches_keyword = state.streamer_name.contains(query)
            || state.room_name.contains(query)
            || state.title.contains(query);
        if !matches_keyword {
            continue;
        }
        if results
            .iter()
            .any(|streamer: &SearchStreamer| streamer.room_id == state.room_id)
        {
            continue;
        }
        results.push(SearchStreamer {
            name: if state.streamer_name.trim().is_empty() {
                state.room_name.clone()
            } else {
                state.streamer_name.clone()
            },
            platform: PLATFORM_DOUYIN_LIVE.to_string(),
            target: build_douyin_live_url(&state.room_id),
            room_id: state.room_id.clone(),
            room_name: state.room_name.clone(),
            avatar_url: state.avatar_url.clone(),
            is_online: state.is_online,
            screenshot_url: state.screenshot_url.clone(),
            heat_text: state.heat_text.clone(),
        });
        if results.len() >= 10 {
            break;
        }
    }

    Ok(results)
}

fn search_streamers_inner(
    app: Option<AppHandle>,
    keyword: &str,
) -> Result<Vec<SearchStreamer>, String> {
    let query = keyword.trim();
    if query.is_empty() {
        return Err("Missing keyword".into());
    }

    let douyu_query = query.to_string();
    let bilibili_query = query.to_string();
    let huya_query = query.to_string();
    let douyin_query = query.to_string();
    let douyin_app = app.clone();

    let douyu_handle =
        std::thread::spawn(move || search_douyu_streamers(&douyu_query).unwrap_or_default());
    let bilibili_handle =
        std::thread::spawn(move || search_bilibili_streamers(&bilibili_query).unwrap_or_default());
    let huya_handle =
        std::thread::spawn(move || search_huya_streamers(&huya_query).unwrap_or_default());
    let douyin_handle = std::thread::spawn(move || {
        search_douyin_streamers_with_app(douyin_app.as_ref(), &douyin_query).unwrap_or_default()
    });

    let mut results = douyu_handle.join().unwrap_or_default();
    results.extend(bilibili_handle.join().unwrap_or_default());
    results.extend(huya_handle.join().unwrap_or_default());
    results.extend(douyin_handle.join().unwrap_or_default());
    Ok(results)
}

fn search_streamers_by_platform_inner(
    app: Option<AppHandle>,
    platform: &str,
    keyword: &str,
) -> Result<Vec<SearchStreamer>, String> {
    let query = keyword.trim();
    if query.is_empty() {
        return Err("Missing keyword".into());
    }

    match normalize_platform(platform) {
        PLATFORM_DOUYU => search_douyu_streamers(query),
        PLATFORM_BILIBILI_LIVE => search_bilibili_streamers(query),
        PLATFORM_HUYA => search_huya_streamers(query),
        PLATFORM_DOUYIN_LIVE => search_douyin_streamers_with_app(app.as_ref(), query),
        _ => search_douyu_streamers(query),
    }
}

fn fetch_bilibili_room_base_infos(
    room_ids: &[String],
) -> Result<HashMap<String, BilibiliRoomBaseInfo>, String> {
    let ids = room_ids
        .iter()
        .filter(|room_id| !room_id.trim().is_empty())
        .map(|room_id| format!("room_ids={}", urlencoding::encode(room_id)))
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }

    let url = format!(
        "https://api.live.bilibili.com/xlive/web-room/v1/index/getRoomBaseInfo?{}&req_biz=web_room_componet",
        ids.join("&")
    );
    let response: BilibiliRoomBaseInfoResponse = serde_json::from_value(
        fetch_bilibili_public_json(&url, "https://live.bilibili.com/")?,
    )
    .map_err(|err| err.to_string())?;
    Ok(response.data.by_room_ids)
}

fn app_data_file(app: &AppHandle, file_name: &str) -> Result<PathBuf, String> {
    let app_dir = app.path().app_data_dir().map_err(|err| err.to_string())?;
    fs::create_dir_all(&app_dir).map_err(|err| err.to_string())?;
    Ok(app_dir.join(file_name))
}

fn read_json_or_default<T>(path: &Path) -> Result<T, String>
where
    T: for<'de> Deserialize<'de> + Default,
{
    if !path.exists() {
        return Ok(T::default());
    }

    let raw = fs::read_to_string(path).map_err(|err| err.to_string())?;
    serde_json::from_str(&raw).map_err(|err| err.to_string())
}

fn write_json<T>(path: &Path, value: &T) -> Result<(), String>
where
    T: Serialize,
{
    let content = serde_json::to_string_pretty(value).map_err(|err| err.to_string())?;
    fs::write(path, content).map_err(|err| err.to_string())
}

#[tauri::command]
fn load_streamers(app: AppHandle) -> Result<Vec<Streamer>, String> {
    let path = app_data_file(&app, "streamers.json")?;
    let mut streamers: Vec<Streamer> = read_json_or_default(&path)?;
    streamers.iter_mut().for_each(ensure_streamer_platform);
    Ok(streamers)
}

#[tauri::command]
fn save_streamers(app: AppHandle, streamers: Vec<Streamer>) -> Result<Vec<Streamer>, String> {
    let path = app_data_file(&app, "streamers.json")?;
    let mut streamers = streamers;
    streamers.iter_mut().for_each(ensure_streamer_platform);
    write_json(&path, &streamers)?;
    Ok(streamers)
}

#[tauri::command]
fn load_settings(app: AppHandle) -> Result<Settings, String> {
    load_settings_inner(&app)
}

#[tauri::command]
fn save_settings(app: AppHandle, settings: Settings) -> Result<Settings, String> {
    save_settings_inner(&app, &settings)?;
    Ok(settings)
}

#[tauri::command]
fn open_bilibili_login(app: AppHandle) -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        return Err("当前仅在 macOS 上支持内置 B站 登录面板".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let login_url = Url::parse(bilibili_login_url()).map_err(|err| err.to_string())?;
        if let Some(window) = app.get_webview_window("bilibili-login") {
            let _ = window.show();
            let _ = window.set_focus();
            let _ = window.navigate(login_url.clone());
            spawn_bilibili_login_watcher(app.clone());
            return Ok(());
        }

        let app_handle = app.clone();
        WebviewWindowBuilder::new(&app, "bilibili-login", WebviewUrl::External(login_url))
            .title("B站 登录")
            .inner_size(980.0, 760.0)
            .resizable(true)
            .center()
            .on_page_load(move |window, _payload| {
                maybe_capture_bilibili_login(app_handle.clone(), window);
            })
            .build()
            .map_err(|err| err.to_string())?;
        spawn_bilibili_login_watcher(app);
        Ok(())
    }
}

#[tauri::command]
fn refresh_bilibili_login(app: AppHandle) -> Result<Settings, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        return Err("当前仅在 macOS 上支持内置 B站 登录面板".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let window = app
            .get_webview_window("bilibili-login")
            .ok_or_else(|| "请先打开 B站 登录面板".to_string())?;
        if let Some(cookie) = extract_bilibili_cookie_from_window(&window)? {
            let settings = save_bilibili_cookie_and_notify(&app, cookie)?;
            let _ = window.close();
            Ok(settings)
        } else {
            Err("当前登录窗口里还没有检测到有效的 B站 登录态".to_string())
        }
    }
}

#[tauri::command]
fn clear_bilibili_login(app: AppHandle) -> Result<Settings, String> {
    clear_bilibili_cookie_and_notify(&app)
}

#[tauri::command]
fn load_platform_icons(app: AppHandle) -> Result<PlatformIcons, String> {
    Ok(PlatformIcons {
        bilibili: resource_file_to_data_url(&app, "bilibili_icon.ico")?,
        douyu: resource_file_to_data_url(&app, "douyu_icon.ico")?,
        huya: resource_file_to_data_url(&app, "huya_icon.ico")?,
        douyin: resource_file_to_data_url(&app, "douyin_icon.ico")?,
    })
}

#[tauri::command]
fn resolve_streamer(app: AppHandle, target: String) -> Result<ResolvedStreamer, String> {
    let parsed = fetch_room_state_for_platform_with_app(
        Some(&app),
        infer_platform_from_target(target.trim()),
        target.trim(),
    )?;
    let fallback_name = if !parsed.streamer_name.trim().is_empty() {
        parsed.streamer_name.trim().to_string()
    } else if !parsed.room_name.trim().is_empty() {
        parsed.room_name.trim().to_string()
    } else if !parsed.room_id.trim().is_empty() {
        parsed.room_id.trim().to_string()
    } else {
        target.trim().to_string()
    };

    Ok(ResolvedStreamer {
        name: fallback_name,
        platform: parsed.platform.clone(),
        target: if parsed.platform == PLATFORM_BILIBILI_LIVE {
            build_bilibili_live_url(&parsed.room_id)
        } else if parsed.platform == PLATFORM_HUYA {
            build_huya_live_url(&parsed.room_id)
        } else if parsed.platform == PLATFORM_DOUYIN_LIVE {
            build_douyin_live_url(&parsed.room_id)
        } else {
            parsed.room_id.clone()
        },
        room_id: parsed.room_id,
        room_name: parsed.room_name,
        streamer_name: parsed.streamer_name,
        avatar_url: parsed.avatar_url,
        is_online: parsed.is_online,
        screenshot_url: parsed.screenshot_url,
        heat_text: parsed.heat_text,
    })
}

#[tauri::command]
async fn search_streamers(app: AppHandle, keyword: String) -> Result<Vec<SearchStreamer>, String> {
    let keyword = keyword.trim().to_string();
    tauri::async_runtime::spawn_blocking(move || search_streamers_inner(Some(app), &keyword))
        .await
        .map_err(|err| err.to_string())?
}

#[tauri::command]
async fn search_streamers_by_platform(
    app: AppHandle,
    platform: String,
    keyword: String,
) -> Result<Vec<SearchStreamer>, String> {
    let keyword = keyword.trim().to_string();
    tauri::async_runtime::spawn_blocking(move || {
        search_streamers_by_platform_inner(Some(app), &platform, &keyword)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
async fn sync_streamers_status(
    app: AppHandle,
    streamers: Vec<Streamer>,
) -> Result<Vec<Streamer>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut streamers = streamers;
        streamers.iter_mut().for_each(ensure_streamer_platform);

        let bili_room_ids = streamers
            .iter()
            .filter(|streamer| streamer.platform == PLATFORM_BILIBILI_LIVE)
            .filter_map(|streamer| extract_bilibili_room_id(&streamer.target))
            .collect::<Vec<_>>();
        let bili_infos = fetch_bilibili_room_base_infos(&bili_room_ids).unwrap_or_default();

        let updated: Vec<Streamer> = streamers
            .into_iter()
            .map(|mut streamer| {
                if streamer.platform == PLATFORM_BILIBILI_LIVE {
                    let room_id = extract_bilibili_room_id(&streamer.target).unwrap_or_default();
                    let current_avatar = streamer.avatar_url.clone().unwrap_or_default();
                    let info = bili_infos
                        .get(&room_id)
                        .or_else(|| bili_infos.values().find(|item| item.short_id == room_id));

                    if let Some(info) = info {
                        let is_online = info.live_status == 1;
                        streamer.target = if info.live_url.trim().is_empty() {
                            build_bilibili_live_url(&info.room_id)
                        } else {
                            info.live_url.clone()
                        };
                        streamer.is_online = Some(is_online);
                        if !info.uname.trim().is_empty() {
                            streamer.name = info.uname.clone();
                        }
                        let avatar_url = resolve_bilibili_avatar(&room_id, &current_avatar, "", "");
                        if !avatar_url.is_empty() {
                            streamer.avatar_url = Some(avatar_url);
                        }
                        if is_online {
                            streamer.screenshot_url = if info.cover.trim().is_empty() {
                                None
                            } else {
                                Some(normalize_remote_image_url(&info.cover))
                            };
                            streamer.heat_text = if info.online.trim().is_empty() {
                                None
                            } else {
                                Some(info.online.clone())
                            };
                        } else {
                            streamer.screenshot_url = None;
                            streamer.heat_text = None;
                        }
                    } else {
                        streamer.is_online = Some(false);
                        let avatar_url = resolve_bilibili_avatar(&room_id, &current_avatar, "", "");
                        if !avatar_url.is_empty() {
                            streamer.avatar_url = Some(avatar_url);
                        }
                        streamer.screenshot_url = None;
                        streamer.heat_text = None;
                    }
                    return streamer;
                }

                if streamer.platform == PLATFORM_HUYA {
                    match fetch_huya_room_state(&streamer.target) {
                        Ok(parsed) => {
                            streamer.target = build_huya_live_url(&parsed.room_id);
                            streamer.is_online = Some(parsed.is_online);
                            if !parsed.avatar_url.trim().is_empty() {
                                streamer.avatar_url = Some(parsed.avatar_url);
                            }
                            if !parsed.streamer_name.trim().is_empty() {
                                streamer.name = parsed.streamer_name;
                            }
                            streamer.screenshot_url =
                                if parsed.is_online && !parsed.screenshot_url.trim().is_empty() {
                                    Some(parsed.screenshot_url)
                                } else {
                                    None
                                };
                            streamer.heat_text =
                                if parsed.is_online && !parsed.heat_text.trim().is_empty() {
                                    Some(parsed.heat_text)
                                } else {
                                    None
                                };
                        }
                        Err(_) => {
                            streamer.is_online = Some(false);
                            streamer.screenshot_url = None;
                            streamer.heat_text = None;
                        }
                    }
                    return streamer;
                }

                if streamer.platform == PLATFORM_DOUYIN_LIVE {
                    match fetch_douyin_room_state_with_app(Some(&app), &streamer.target) {
                        Ok(parsed) => {
                            streamer.target = build_douyin_live_url(&parsed.room_id);
                            streamer.is_online = Some(parsed.is_online);
                            if !parsed.avatar_url.trim().is_empty() {
                                streamer.avatar_url = Some(parsed.avatar_url);
                            }
                            if !parsed.streamer_name.trim().is_empty() {
                                streamer.name = parsed.streamer_name;
                            }
                            streamer.screenshot_url =
                                if parsed.is_online && !parsed.screenshot_url.trim().is_empty() {
                                    Some(parsed.screenshot_url)
                                } else {
                                    None
                                };
                            streamer.heat_text =
                                if parsed.is_online && !parsed.heat_text.trim().is_empty() {
                                    Some(parsed.heat_text)
                                } else {
                                    None
                                };
                        }
                        Err(_) => {
                            streamer.is_online = Some(false);
                            streamer.screenshot_url = None;
                            streamer.heat_text = None;
                        }
                    }
                    return streamer;
                }

                match fetch_douyu_room_state(&streamer.target) {
                    Ok(parsed) => {
                        streamer.is_online = Some(parsed.is_online);
                        if !parsed.avatar_url.trim().is_empty() {
                            streamer.avatar_url = Some(parsed.avatar_url);
                        }
                        if parsed.is_online && !parsed.screenshot_url.trim().is_empty() {
                            streamer.screenshot_url = Some(parsed.screenshot_url);
                        } else if !parsed.is_online {
                            streamer.screenshot_url = None;
                        }
                        streamer.heat_text =
                            if !parsed.is_online || parsed.heat_text.trim().is_empty() {
                                None
                            } else {
                                Some(parsed.heat_text)
                            };
                        if !parsed.streamer_name.trim().is_empty() {
                            streamer.name = parsed.streamer_name;
                        }
                    }
                    Err(_) => {
                        streamer.is_online = Some(false);
                        streamer.screenshot_url = None;
                        streamer.heat_text = None;
                    }
                }
                streamer
            })
            .collect();

        let path = app_data_file(&app, "streamers.json")?;
        write_json(&path, &updated)?;
        Ok(updated)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
async fn resolve_stream_playback(
    app: AppHandle,
    streamer: Streamer,
    settings: Settings,
) -> Result<FrontendPlayInfo, String> {
    let data = extract_stream_playback_data(app.clone(), streamer, settings.clone()).await?;
    let port = ensure_danmaku_server_running(&app)?;
    let proxy_urls = register_stream_proxy_sessions(&app, &settings, &data, port)?;

    Ok(FrontendPlayInfo {
        platform: data.platform,
        room_id: data.room_id,
        streamer_name: data.streamer_name,
        room_name: data.room_name,
        avatar_url: data.avatar_url,
        is_online: data.is_online,
        screenshot_url: data.screenshot_url,
        heat_text: data.heat_text,
        page_url: data.page_url,
        title: data.title,
        urls: data.urls,
        proxy_urls,
    })
}

async fn extract_stream_playback_data(
    app: AppHandle,
    streamer: Streamer,
    settings: Settings,
) -> Result<ExtractResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let platform = if streamer.platform.trim().is_empty() {
            infer_platform_from_target(&streamer.target).to_string()
        } else {
            normalize_platform(&streamer.platform).to_string()
        };
        let data = extract_play_info_for_platform_with_app(
            Some(&app),
            &platform,
            &streamer.target,
            &settings.bilibili_cookie,
        )?;
        if !data.is_online {
            return Err("主播当前未开播".into());
        }
        if data.urls.is_empty() {
            return Err("未获取到可播放的直播地址".into());
        }

        Ok::<_, String>(data)
    })
    .await
    .map_err(|err| err.to_string())?
}

#[tauri::command]
fn ensure_danmaku_server(app: AppHandle) -> Result<u16, String> {
    ensure_danmaku_server_running(&app)
}

pub fn run() {
    tauri::Builder::default()
        .manage(Arc::new(DanmakuServerState {
            started: AtomicBool::new(false),
            port: 19080,
            stream_sessions: Arc::new(Mutex::new(HashMap::new())),
        }))
        .invoke_handler(tauri::generate_handler![
            load_streamers,
            save_streamers,
            load_platform_icons,
            load_settings,
            save_settings,
            open_bilibili_login,
            refresh_bilibili_login,
            clear_bilibili_login,
            resolve_streamer,
            search_streamers,
            search_streamers_by_platform,
            sync_streamers_status,
            resolve_stream_playback,
            ensure_danmaku_server
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
