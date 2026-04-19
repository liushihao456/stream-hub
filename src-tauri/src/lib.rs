use axum::extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderValue as AxumHeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use base64::Engine;
use block2::RcBlock;
use futures_util::StreamExt;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, ORIGIN, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::ptr::NonNull;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command as TokioCommand;
use url::Url;
use uuid::Uuid;

#[cfg(target_os = "macos")]
use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSArray, NSHTTPCookie, NSString};
#[cfg(target_os = "macos")]
use objc2_web_kit::WKWebView;

const DOUYU_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36";
const BILIBILI_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
const PLATFORM_DOUYU: &str = "douyu";
const PLATFORM_BILIBILI_LIVE: &str = "bilibili_live";

fn default_streamer_platform() -> String {
    PLATFORM_DOUYU.to_string()
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
    player: String,
    iina_path: String,
    mpv_path: String,
    bilibili_cookie: String,
    enable_iina_danmaku: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            player: default_player().to_string(),
            iina_path: String::new(),
            mpv_path: String::new(),
            bilibili_cookie: String::new(),
            enable_iina_danmaku: true,
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
struct DanmakuEventPayload {
    method: String,
    dms: Vec<DanmakuCommentPayload>,
}

struct DanmakuServerState {
    started: AtomicBool,
    port: u16,
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

#[derive(Debug, Clone, Deserialize)]
struct SearchApiAnchorInfo {
    #[serde(rename = "rid", deserialize_with = "deserialize_value_to_string")]
    room_id: String,
    #[serde(rename = "nickName")]
    nick_name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    avatar: String,
    #[serde(default, rename = "roomSrc")]
    room_src: String,
    #[serde(default, rename = "isLive")]
    is_live: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct BilibiliLiveSearchResponse {
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct IinaPlusArgs {
    raw_url: String,
    mpv_script: String,
    port: u16,
    urls: Vec<String>,
    r#type: i32,
    qualitys: Vec<String>,
    lines: Vec<String>,
    current_quality: usize,
    current_line: usize,
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
    dummy_media: Arc<Vec<u8>>,
    node_bin: String,
    bridge_script: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlatformIcons {
    bilibili: String,
    douyu: String,
}

const EMPTY_M4A_BYTES: &[u8] = include_bytes!("../resources/empty.m4a");

fn douyu_client() -> Result<Client, String> {
    Client::builder()
        .cookie_store(true)
        .build()
        .map_err(|err| err.to_string())
}

fn bilibili_client() -> Result<Client, String> {
    Client::builder()
        .cookie_store(true)
        .build()
        .map_err(|err| err.to_string())
}

fn default_player() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "iina"
    }

    #[cfg(not(target_os = "macos"))]
    {
        "mpv"
    }
}

fn normalize_platform(platform: &str) -> &'static str {
    match platform.trim().to_lowercase().as_str() {
        PLATFORM_BILIBILI_LIVE => PLATFORM_BILIBILI_LIVE,
        _ => PLATFORM_DOUYU,
    }
}

fn infer_platform_from_target(target: &str) -> &'static str {
    let trimmed = target.trim().to_lowercase();
    if trimmed.contains("live.bilibili.com") {
        PLATFORM_BILIBILI_LIVE
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

fn extract_bilibili_room_id(target: &str) -> Option<String> {
    let trimmed = target.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|char| char.is_ascii_digit()) {
        return Some(trimmed.to_string());
    }

    let without_fragment = trimmed.split('#').next().unwrap_or(trimmed);
    let without_query = without_fragment.split('?').next().unwrap_or(without_fragment);
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


fn danmaku_text_event(text: impl Into<String>) -> Result<String, String> {
    let event = DanmakuEventPayload {
        method: "sendDM".to_string(),
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
    let without_query = without_fragment.split('?').next().unwrap_or(without_fragment);
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
        headers.insert(REFERER, HeaderValue::from_str(value).map_err(|err| err.to_string())?);
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
    headers.insert(REFERER, HeaderValue::from_str(referer).map_err(|err| err.to_string())?);
    headers.insert(ORIGIN, HeaderValue::from_static("https://live.bilibili.com"));
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

fn fetch_bilibili_search(keyword: &str) -> Result<Vec<BilibiliLiveSearchItem>, String> {
    let client = bilibili_client()?;
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(BILIBILI_USER_AGENT).map_err(|err| err.to_string())?,
    );
    headers.insert(REFERER, HeaderValue::from_static("https://www.bilibili.com/"));
    let _ = client
        .get("https://www.bilibili.com")
        .headers(headers.clone())
        .send();

    let response: BilibiliLiveSearchResponse = client
        .get("https://api.bilibili.com/x/web-interface/search/type")
        .headers(headers)
        .query(&[
            ("search_type", "live_user"),
            ("keyword", keyword),
            ("order", "online"),
            ("page", "1"),
        ])
        .send()
        .map_err(|err| err.to_string())?
        .json()
        .map_err(|err| err.to_string())?;

    Ok(response.data.result)
}

fn post_form_json(url: &str, referer: &str, body: &[(String, String)]) -> Result<Value, String> {
    let client = douyu_client()?;
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(DOUYU_USER_AGENT).map_err(|err| err.to_string())?,
    );
    headers.insert(REFERER, HeaderValue::from_str(referer).map_err(|err| err.to_string())?);
    headers.insert(
        ORIGIN,
        HeaderValue::from_static("https://www.douyu.com"),
    );
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
    let room_info_json =
        extract_room_info_json(html).ok_or_else(|| "Failed to extract roomInfo JSON".to_string())?;
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
        avatar_url: if avatar_big.is_empty() { avatar_mid } else { avatar_big },
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
        avatar_url: if avatar_big.is_empty() { avatar_mid } else { avatar_big },
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
    let url = format!(
        "https://www.douyu.com/wgapi/livenc/liveweb/websec/getEncryption?did={did}"
    );
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

    let mut xs_parts: Vec<String> = rtmp_live.replace("flv", "xs").split('&').map(str::to_string).collect();
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

    let mut urls = vec![primary_url];
    urls.extend(get_backup_urls(play_data));
    state.title = value_to_string(&play_data["room_name"]);
    if state.title.is_empty() {
        state.title = state.room_name.clone();
    }
    state.urls = urls;
    Ok(state)
}

fn get_bilibili_room_info(target: &str, raw_cookie: &str) -> Result<BilibiliRoomInfoData, String> {
    let room_id = extract_bilibili_room_id(target).ok_or_else(|| "无法识别 B 站直播间房间号".to_string())?;
    let url = format!(
        "https://api.live.bilibili.com/room/v1/Room/get_info?room_id={room_id}"
    );
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
    codec.url_info
        .iter()
        .filter(|info| !info.host.trim().is_empty())
        .map(|info| format!("{}{}{}", info.host, codec.base_url, info.extra))
        .collect()
}

fn bilibili_codec_quality(codec: &BilibiliPlayCodec) -> i64 {
    let current = codec.current_qn;
    if current > 0 {
        current
    } else {
        codec.accept_qn.iter().copied().max().unwrap_or_default()
    }
}

fn bilibili_protocol_rank(protocol_name: &str) -> i32 {
    match protocol_name {
        "http_stream" => 2,
        "http_hls" => 1,
        _ => 0,
    }
}

fn bilibili_format_rank(format_name: &str) -> i32 {
    match format_name {
        "flv" => 2,
        "fmp4" => 1,
        _ => 0,
    }
}

fn bilibili_codec_rank(codec_name: &str) -> i32 {
    match codec_name {
        "avc" => 2,
        "hevc" => 1,
        _ => 0,
    }
}

fn select_bilibili_urls(playurl: &BilibiliPlayurl) -> Vec<String> {
    let mut best_score = None::<(i64, i32, i32, i32)>;
    let mut best_urls = Vec::new();

    for stream in &playurl.stream {
        let protocol_rank = bilibili_protocol_rank(&stream.protocol_name);
        for format in &stream.formats {
            let format_rank = bilibili_format_rank(&format.format_name);
            for codec in &format.codecs {
                let urls = bilibili_codec_urls(codec);
                if urls.is_empty() {
                    continue;
                }

                let score = (
                    bilibili_codec_quality(codec),
                    protocol_rank,
                    format_rank,
                    bilibili_codec_rank(&codec.codec_name),
                );

                if best_score.as_ref().map(|best| score > *best).unwrap_or(true) {
                    best_score = Some(score);
                    best_urls = urls;
                }
            }
        }
    }

    best_urls
}

fn fetch_bilibili_room_play_info(room_id: &str, qn: i64, raw_cookie: &str) -> Result<Vec<String>, String> {
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

fn fetch_bilibili_old_play_url(room_id: &str, qn: i64, raw_cookie: &str) -> Result<Vec<String>, String> {
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
        .filter_map(|item| if item.url.trim().is_empty() { None } else { Some(item.url) })
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
        headers.insert(REFERER, HeaderValue::from_str(&room_url).map_err(|err| err.to_string())?);
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

    let json_text = extract_json_from_script(&html, "<script>window.__NEPTUNE_IS_MY_WAIFU__=", "</script>")
        .ok_or_else(|| "未找到 B 站直播页面初始化数据".to_string())?;
    let json: Value = serde_json::from_str(&json_text).map_err(|err| err.to_string())?;
    let playurl = &json["roomInitRes"]["data"]["playurl_info"]["playurl"];
    let playurl: BilibiliPlayurl = serde_json::from_value(playurl.clone()).map_err(|err| err.to_string())?;
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

fn fetch_room_state_for_platform(platform: &str, target: &str) -> Result<ExtractResult, String> {
    match normalize_platform(platform) {
        PLATFORM_BILIBILI_LIVE => fetch_bilibili_room_state(target, ""),
        _ => fetch_douyu_room_state(target),
    }
}

fn fetch_room_state(target: &str) -> Result<ExtractResult, String> {
    fetch_room_state_for_platform(infer_platform_from_target(target), target)
}

fn extract_play_info_for_platform(
    platform: &str,
    target: &str,
    bilibili_cookie: &str,
) -> Result<ExtractResult, String> {
    match normalize_platform(platform) {
        PLATFORM_BILIBILI_LIVE => extract_bilibili_play_info(target, bilibili_cookie),
        _ => extract_douyu_play_info(target),
    }
}

fn settings_file(app: &AppHandle) -> Result<PathBuf, String> {
    app_data_file(app, "settings.json")
}

fn load_settings_inner(app: &AppHandle) -> Result<Settings, String> {
    let path = settings_file(app)?;
    read_json_or_default(&path)
}

fn save_settings_inner(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_file(app)?;
    write_json(&path, settings)
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

fn save_bilibili_cookie_and_notify(
    app: &AppHandle,
    cookie: String,
) -> Result<Settings, String> {
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

fn maybe_capture_bilibili_login<R: tauri::Runtime>(
    app: AppHandle,
    window: WebviewWindow<R>,
) {
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

fn write_playlist(title: &str, urls: &[String]) -> Result<PathBuf, String> {
    let safe_title = if title.trim().is_empty() {
        "Stream Hub Live".to_string()
    } else {
        title.trim().to_string()
    };

    let mut content = vec!["#EXTM3U".to_string()];
    for (index, url) in urls.iter().enumerate() {
        let name = if index == 0 {
            safe_title.clone()
        } else {
            format!("{safe_title} - Backup {index}")
        };
        content.push(format!("#EXTINF:-1,{name}"));
        content.push(url.clone());
    }

    let path = env::temp_dir().join(format!("stream-hub-{}.m3u", Uuid::new_v4()));
    fs::write(&path, content.join("\n") + "\n").map_err(|err| err.to_string())?;
    Ok(path)
}

fn normalize_player(settings: &Settings) -> String {
    let player = settings.player.trim().to_lowercase();
    if player.is_empty() {
        default_player().to_string()
    } else {
        player
    }
}

fn detect_mpv(settings: &Settings) -> Result<String, String> {
    if !settings.mpv_path.trim().is_empty() {
        return Ok(settings.mpv_path.trim().to_string());
    }

    #[cfg(target_os = "windows")]
    let candidates = vec!["mpv.exe".to_string(), "mpv".to_string()];

    #[cfg(target_os = "macos")]
    let candidates = vec![
        "mpv".to_string(),
        "/opt/homebrew/bin/mpv".to_string(),
        "/usr/local/bin/mpv".to_string(),
    ];

    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let candidates = vec!["mpv".to_string()];

    for program in candidates {
        let mut cmd = Command::new(&program);
        cmd.arg("--version");
        if let Ok(status) = cmd.status() {
            if status.success() {
                return Ok(program);
            }
        }
    }

    Err("未找到可用的 mpv，请在设置里手动填写 mpv 路径。".into())
}

fn normalize_iina_cli_candidate(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.ends_with("/Contents/MacOS/iina-cli") {
        trimmed.to_string()
    } else if trimmed.ends_with(".app") {
        format!("{trimmed}/Contents/MacOS/iina-cli")
    } else {
        trimmed.to_string()
    }
}

fn escape_mpv_script_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn build_mpv_script_opts(data: &ExtractResult, media_title: &str) -> String {
    let mut options = vec![
        ("force-media-title", media_title.to_string()),
        ("ytdl", "no".to_string()),
        ("stream-lavf-o", "reconnect_streamed=yes".to_string()),
    ];

    if data.platform == PLATFORM_BILIBILI_LIVE {
        options.push(("referrer", "https://live.bilibili.com/".to_string()));
    }

    options
        .iter()
        .map(|(key, value)| format!(r#"{key}="{}""#, escape_mpv_script_value(value)))
        .collect::<Vec<_>>()
        .join(",")
}

fn encode_hex(data: &[u8]) -> String {
    data.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn build_iina_plus_args(data: &ExtractResult, media_title: &str, port: u16) -> Result<String, String> {
    let payload = IinaPlusArgs {
        raw_url: data.page_url.clone(),
        mpv_script: build_mpv_script_opts(data, media_title),
        port,
        urls: data.urls.clone(),
        r#type: 0,
        qualitys: vec!["原画".to_string()],
        lines: data
            .urls
            .iter()
            .enumerate()
            .map(|(index, _)| format!("线路 {}", index + 1))
            .collect(),
        current_quality: 0,
        current_line: 0,
    };
    let json = serde_json::to_vec(&payload).map_err(|err| err.to_string())?;
    Ok(encode_hex(&json))
}

fn detect_iina_cli(settings: &Settings) -> Result<String, String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = settings;
        return Err("IINA 仅支持在 macOS 上使用。".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        if !settings.iina_path.trim().is_empty() {
            let normalized = normalize_iina_cli_candidate(&settings.iina_path);
            if Path::new(&normalized).exists() {
                return Ok(normalized);
            }
            return Err("未找到你填写的 IINA 路径。".to_string());
        }

        let candidates = [
            "/Applications/IINA.app/Contents/MacOS/iina-cli",
            "/Applications/IINA Nightly.app/Contents/MacOS/iina-cli",
            "/Applications/IINA.app",
            "/Applications/IINA Nightly.app",
        ];

        for candidate in candidates {
            let normalized = normalize_iina_cli_candidate(candidate);
            if Path::new(&normalized).exists() {
                return Ok(normalized);
            }
        }

        Err("未找到可用的 IINA，请在设置里手动填写 IINA.app 或 iina-cli 路径。".to_string())
    }
}

fn iina_process_name_from_cli(iina_cli: &str) -> String {
    if iina_cli.contains("Nightly") {
        "IINA Nightly".to_string()
    } else {
        "IINA".to_string()
    }
}

fn bring_iina_to_front(process_name: &str) {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = process_name;
    }

    #[cfg(target_os = "macos")]
    {
        let bundle_id = if process_name.contains("Nightly") {
            "com.colliderli.iina-nightly"
        } else {
            "com.colliderli.iina"
        }
        .to_string();

        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(350));
            let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(
                NSString::from_str(&bundle_id).as_ref(),
            );
            if let Some(app) = apps.firstObject() {
                let _ = app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
            }
        });
    }
}

fn iina_plugin_root() -> Result<PathBuf, String> {
    #[cfg(not(target_os = "macos"))]
    {
        return Err("IINA 插件仅支持在 macOS 上安装。".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        let home = env::var("HOME").map_err(|err| err.to_string())?;
        let path = PathBuf::from(home)
            .join("Library/Application Support/com.colliderli.iina/plugins");
        fs::create_dir_all(&path).map_err(|err| err.to_string())?;
        Ok(path)
    }
}

fn enable_iina_plugin_system() -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        let status = Command::new("defaults")
            .arg("write")
            .arg("com.colliderli.iina")
            .arg("iinaEnablePluginSystem")
            .arg("-bool")
            .arg("true")
            .status()
            .map_err(|err| format!("启用 IINA 插件系统失败：{err}"))?;

        if status.success() {
            Ok(())
        } else {
            Err("启用 IINA 插件系统失败。".to_string())
        }
    }
}

fn xjbeta_iina_plugin_dir() -> Result<PathBuf, String> {
    let plugin_dir = iina_plugin_root()?.join("com.xjbeta.danmaku.iinaplugin");
    if plugin_dir.exists() {
        Ok(plugin_dir)
    } else {
        Err("未检测到 IINA 弹幕插件 com.xjbeta.danmaku，请先安装 iina-plus 的弹幕插件。".to_string())
    }
}

fn detect_node_binary() -> Result<String, String> {
    let candidates = [
        "node",
        "/opt/homebrew/bin/node",
        "/usr/local/bin/node",
    ];

    for candidate in candidates {
        let mut cmd = Command::new(candidate);
        cmd.arg("--version");
        if let Ok(status) = cmd.status() {
            if status.success() {
                return Ok(candidate.to_string());
            }
        }
    }

    Err("未找到可用的 Node.js，斗鱼弹幕桥接无法启动。".to_string())
}

fn locate_danmaku_bridge_script(app: &AppHandle) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();

    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("douyu_danmaku_bridge.js"));
    }

    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/douyu_danmaku_bridge.js"));

    if let Ok(current_dir) = env::current_dir() {
        candidates.push(current_dir.join("src-tauri/resources/douyu_danmaku_bridge.js"));
        candidates.push(current_dir.join("resources/douyu_danmaku_bridge.js"));
    }

    candidates
        .into_iter()
        .find(|path| path.exists() && path.is_file())
        .ok_or_else(|| "未找到斗鱼弹幕桥接脚本。".to_string())
}

fn locate_resource_file(app: &AppHandle, file_name: &str) -> Result<PathBuf, String> {
    let mut candidates = Vec::new();

    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("resources").join(file_name));
        candidates.push(resource_dir.join(file_name));
    }

    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources").join(file_name));

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

fn enable_xjbeta_iina_plugin() -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        let _ = xjbeta_iina_plugin_dir()?;
        let plugin_key = "PluginEnabled.com.xjbeta.danmaku";
        let status = Command::new("defaults")
            .arg("write")
            .arg("com.colliderli.iina")
            .arg(plugin_key)
            .arg("-bool")
            .arg("true")
            .status()
            .map_err(|err| format!("启用 IINA 弹幕插件失败：{err}"))?;

        if !status.success() {
            return Err("启用 IINA 弹幕插件失败。".to_string());
        }

        let preferences_dir = iina_plugin_root()?.join(".preferences");
        fs::create_dir_all(&preferences_dir).map_err(|err| err.to_string())?;
        let pref_plist = preferences_dir.join("com.xjbeta.danmaku.plist");
        let parse_status = Command::new("defaults")
            .arg("write")
            .arg(pref_plist.as_os_str())
            .arg("enableIINAPLUSOptsParse")
            .arg("1")
            .status()
            .map_err(|err| format!("启用 IINA 弹幕参数解析失败：{err}"))?;

        if parse_status.success() {
            patch_xjbeta_plugin_visibility_behavior()?;
            Ok(())
        } else {
            Err("启用 IINA 弹幕参数解析失败。".to_string())
        }
    }
}

fn patch_xjbeta_plugin_visibility_behavior() -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        let plugin_file = xjbeta_iina_plugin_dir()?.join("DanmakuWeb/main.js");
        let content = fs::read_to_string(&plugin_file).map_err(|err| err.to_string())?;

        let old_block = r#"    document.addEventListener('visibilitychange', function () {
        if (document.visibilityState == 'visible') {
            console.log('visible');
            cm.start();
            cm.clear();
        } else {
            console.log('hidden');
            cm.stop();
            cm.clear();
        };
    });
"#;

        let new_block = r#"    document.addEventListener('visibilitychange', function () {
        if (document.visibilityState == 'visible') {
            console.log('visible');
            cm.start();
        } else {
            console.log('hidden');
            cm.stop();
        };
    });
"#;

        if content.contains(new_block) {
            return Ok(());
        }

        if !content.contains(old_block) {
            return Err("未找到可修补的 IINA 弹幕可见性逻辑。".to_string());
        }

        let patched = content.replacen(old_block, new_block, 1);
        fs::write(&plugin_file, patched).map_err(|err| err.to_string())?;
        Ok(())
    }
}

fn is_iina_running() -> bool {
    #[cfg(not(target_os = "macos"))]
    {
        false
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("pgrep")
            .arg("-x")
            .arg("IINA")
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
}

fn restart_iina_if_needed() -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        let status = Command::new("osascript")
            .arg("-e")
            .arg("tell application \"IINA\" to quit")
            .status()
            .map_err(|err| format!("重启 IINA 失败：{err}"))?;

        if !status.success() {
            return Err("重启 IINA 失败。".to_string());
        }

        std::thread::sleep(std::time::Duration::from_millis(900));
        Ok(())
    }
}

fn open_iina_playlist(
    app: &AppHandle,
    settings: &Settings,
    playlist_path: &Path,
    media_title: &str,
    data: &ExtractResult,
) -> Result<(), String> {
    let iina_cli = detect_iina_cli(settings)?;
    let iina_process_name = iina_process_name_from_cli(&iina_cli);
    let enable_danmaku = settings.enable_iina_danmaku
        && matches!(
            data.platform.as_str(),
            PLATFORM_DOUYU | PLATFORM_BILIBILI_LIVE
        );
    if enable_danmaku {
        enable_iina_plugin_system()?;
        enable_xjbeta_iina_plugin()?;
    }

    let mut command = Command::new(iina_cli);
    command.arg("--no-stdin");
    command.arg("--mpv-pause=no");
    command.arg("--mpv-force-window=immediate");

    if enable_danmaku {
        let danmaku_port = ensure_danmaku_server(app)?;
        let args_hex = build_iina_plus_args(data, media_title, danmaku_port)?;
        let checker = args_hex
            .chars()
            .rev()
            .take(25)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        command.arg(format!("--mpv-script-opts=iinaPlusArgs={args_hex}"));
        command.arg(format!(
            "http://127.0.0.1:{danmaku_port}/video.mp4?{checker}"
        ));
    } else {
        command.arg("--mpv-ytdl=no");
        command.arg("--mpv-stream-lavf-o=reconnect_streamed=yes");
        command.arg(format!("--mpv-force-media-title={media_title}"));
        if data.platform == PLATFORM_BILIBILI_LIVE {
            command.arg("--mpv-referrer=https://live.bilibili.com/");
        }
        command.arg(playlist_path);
    }

    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command.spawn().map_err(|err| err.to_string())?;
    bring_iina_to_front(&iina_process_name);
    Ok(())
}

fn open_mpv_playlist(
    playlist_path: &Path,
    media_title: &str,
    data: &ExtractResult,
    settings: &Settings,
) -> Result<(), String> {
    let mpv_bin = detect_mpv(settings)?;
    let mut command = Command::new(mpv_bin);
    command.arg("--ytdl=no");
    command.arg("--stream-lavf-o=reconnect_streamed=yes");
    command.arg(format!("--force-media-title={media_title}"));
    if data.platform == PLATFORM_BILIBILI_LIVE {
        command.arg("--referrer=https://live.bilibili.com/");
    }
    command.arg(playlist_path);
    command.stdin(Stdio::null());
    command.stdout(Stdio::null());
    command.stderr(Stdio::null());
    command.spawn().map_err(|err| err.to_string())?;
    Ok(())
}

fn ensure_danmaku_server(app: &AppHandle) -> Result<u16, String> {
    let state = app.state::<Arc<DanmakuServerState>>();
    if state.started.swap(true, Ordering::SeqCst) {
        return Ok(state.port);
    }

    let port = state.port;
    let http_state = DanmakuHttpState {
        dummy_media: Arc::new(EMPTY_M4A_BYTES.to_vec()),
        node_bin: detect_node_binary()?,
        bridge_script: locate_danmaku_bridge_script(app)?,
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
        .route("/video.mp4", get(handle_dummy_video))
        .route("/danmaku-websocket", get(handle_danmaku_websocket))
        .with_state(state);
    let _ = axum::serve(listener, app).await;
}

async fn handle_dummy_video(
    State(state): State<DanmakuHttpState>,
) -> impl IntoResponse {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("content-type", AxumHeaderValue::from_static("audio/mp4"));
    headers.insert("cache-control", AxumHeaderValue::from_static("no-store"));
    (StatusCode::OK, headers, (*state.dummy_media).clone())
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
    let payload = if let Some(raw) = message.strip_prefix("iinaWebDM://") {
        if let Some((prefix, url)) = raw.split_once('&') {
            if prefix.starts_with("v=") {
                url
            } else {
                raw
            }
        } else {
            raw
        }
    } else if let Some(raw) = message.strip_prefix("iinaDM://") {
        if let Some((prefix, url)) = raw.split_once('&') {
            if prefix.starts_with("v=") {
                url
            } else {
                raw
            }
        } else {
            raw
        }
    } else {
        message
    };

    let platform = infer_platform_from_target(payload).to_string();
    let room_id = if platform == PLATFORM_BILIBILI_LIVE {
        extract_bilibili_room_id(payload)
    } else {
        extract_room_id_from_target(payload)
    }?;
    Some((platform, room_id))
}

async fn proxy_live_danmaku(
    mut client_stream: WebSocket,
    initial_platform: String,
    initial_room_id: String,
    state: DanmakuHttpState,
) -> Result<(), String> {
    client_stream
        .send(AxumMessage::Text(
            danmaku_text_event("Stream Hub 本地弹幕服务已连接")?,
        ))
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
                    if let Some((platform, room_id)) = parse_danmaku_target_from_client_message(&text) {
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
            .send(AxumMessage::Text(
                danmaku_text_event("Stream Hub 未能解析房间号")?,
            ))
            .await;
        return Err("无法确定弹幕房间号".to_string());
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
            "status" => danmaku_text_event(event.text)?,
            "chat" => danmaku_text_event(event.text)?,
            "error" => danmaku_text_event(format!("Stream Hub 弹幕桥接失败: {}", event.text))?,
            _ => continue,
        };

        if client_stream.send(AxumMessage::Text(outgoing)).await.is_err() {
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
    let response: SearchApiUserResponse = serde_json::from_value(fetch_search_json(&url, query)?)
        .map_err(|err| err.to_string())?;

    let mut results = Vec::new();
    for item in response.data.relate_user {
        let anchor = item.anchor_info;
        let room_id = anchor.room_id;
        let name = anchor.nick_name;
        if room_id.is_empty() || name.is_empty() {
            continue;
        }

        let is_online = anchor.is_live == 1;
        let mut result = SearchStreamer {
            name,
            platform: PLATFORM_DOUYU.to_string(),
            target: room_id.clone(),
            room_id,
            room_name: anchor.description,
            avatar_url: anchor.avatar,
            is_online,
            screenshot_url: anchor.room_src,
            heat_text: String::new(),
        };

        if result.is_online {
            if let Ok(meta) = get_room_meta(&result.room_id) {
                if !meta.screenshot_url.trim().is_empty() {
                    result.screenshot_url = meta.screenshot_url;
                }
                result.heat_text = meta.heat_text;
            }
        }

        results.push(result);
    }

    Ok(results)
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
        let avatar_url = resolve_bilibili_avatar(&room_id, "", &item.uface, "");
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
                room_info.map(|info| info.online.clone()).unwrap_or_default()
            } else {
                String::new()
            },
        });
    }
    Ok(results)
}

fn search_streamers_inner(keyword: &str) -> Result<Vec<SearchStreamer>, String> {
    let query = keyword.trim();
    if query.is_empty() {
        return Err("Missing keyword".into());
    }

    let mut results = search_douyu_streamers(query).unwrap_or_default();
    results.extend(search_bilibili_streamers(query).unwrap_or_default());
    Ok(results)
}

fn fetch_bilibili_room_base_infos(room_ids: &[String]) -> Result<HashMap<String, BilibiliRoomBaseInfo>, String> {
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
    let response: BilibiliRoomBaseInfoResponse = serde_json::from_value(fetch_bilibili_json(
        &url,
        "https://live.bilibili.com/",
        "",
    )?)
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
        WebviewWindowBuilder::new(
            &app,
            "bilibili-login",
            WebviewUrl::External(login_url),
        )
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
    })
}

#[tauri::command]
fn install_iina_danmaku_plugin(app: AppHandle) -> Result<String, String> {
    let _ = app;
    enable_iina_plugin_system()?;
    enable_xjbeta_iina_plugin()?;
    if is_iina_running() {
        restart_iina_if_needed()?;
        Ok("已启用 IINA 弹幕插件并重启 IINA。".to_string())
    } else {
        Ok("已启用 IINA 弹幕插件。".to_string())
    }
}

#[tauri::command]
fn resolve_streamer(target: String) -> Result<ResolvedStreamer, String> {
    let parsed = fetch_room_state(target.trim())?;
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
fn search_streamers(keyword: String) -> Result<Vec<SearchStreamer>, String> {
    search_streamers_inner(keyword.trim())
}

#[tauri::command]
async fn sync_streamers_status(app: AppHandle, streamers: Vec<Streamer>) -> Result<Vec<Streamer>, String> {
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
                        streamer.heat_text = if !parsed.is_online || parsed.heat_text.trim().is_empty() {
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
fn play_streamer(app: AppHandle, streamer: Streamer, settings: Settings) -> Result<(), String> {
    let platform = if streamer.platform.trim().is_empty() {
        infer_platform_from_target(&streamer.target).to_string()
    } else {
        normalize_platform(&streamer.platform).to_string()
    };
    let data = extract_play_info_for_platform(&platform, &streamer.target, &settings.bilibili_cookie)?;
    if !data.is_online {
        return Err("主播当前未开播".into());
    }
    if data.urls.is_empty() {
        return Err("未获取到可播放的直播地址".into());
    }

    let playlist_path = write_playlist(&data.title, &data.urls)?;
    let media_title = if data.title.trim().is_empty() {
        "Stream Hub Live".to_string()
    } else {
        data.title.clone()
    };

    match normalize_player(&settings).as_str() {
        "iina" => open_iina_playlist(&app, &settings, &playlist_path, &media_title, &data),
        _ => open_mpv_playlist(&playlist_path, &media_title, &data, &settings),
    }
}

pub fn run() {
    tauri::Builder::default()
        .manage(Arc::new(DanmakuServerState {
            started: AtomicBool::new(false),
            port: 19080,
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
            install_iina_danmaku_plugin,
            resolve_streamer,
            search_streamers,
            sync_streamers_status,
            play_streamer
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
