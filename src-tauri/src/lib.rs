use axum::extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderValue as AxumHeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::StreamExt;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, ORIGIN, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command as TokioCommand;
use uuid::Uuid;

#[cfg(target_os = "macos")]
use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
#[cfg(target_os = "macos")]
use objc2_foundation::NSString;

const DOUYU_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/135.0.0.0 Safari/537.36";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Streamer {
    id: String,
    name: String,
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
    enable_iina_danmaku: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            player: default_player().to_string(),
            iina_path: String::new(),
            mpv_path: String::new(),
            enable_iina_danmaku: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolvedStreamer {
    name: String,
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
}

#[derive(Clone)]
struct DanmakuHttpState {
    dummy_media: Arc<Vec<u8>>,
    node_bin: String,
    bridge_script: PathBuf,
}

const EMPTY_M4A_BYTES: &[u8] = include_bytes!("../resources/empty.m4a");

fn douyu_client() -> Result<Client, String> {
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

fn fetch_room_state(target: &str) -> Result<ExtractResult, String> {
    if let Some(room_id) = extract_room_id_from_target(target) {
        let snapshot = get_room_snapshot(&room_id)?;
        return Ok(ExtractResult {
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

fn extract_play_info(target: &str) -> Result<ExtractResult, String> {
    let mut state = fetch_room_state(target)?;
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

fn write_playlist(title: &str, urls: &[String]) -> Result<PathBuf, String> {
    let safe_title = if title.trim().is_empty() {
        "Douyu Live".to_string()
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

    let path = env::temp_dir().join("stream-hub-douyu.m3u");
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

fn build_mpv_script_opts(media_title: &str) -> String {
    let options = [
        ("force-media-title", media_title),
        ("ytdl", "no"),
        ("stream-lavf-o", "reconnect_streamed=yes"),
    ];

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
        mpv_script: build_mpv_script_opts(media_title),
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
    if settings.enable_iina_danmaku {
        enable_iina_plugin_system()?;
        enable_xjbeta_iina_plugin()?;
    }

    let mut command = Command::new(iina_cli);
    command.arg("--no-stdin");
    command.arg("--mpv-pause=no");
    command.arg("--mpv-force-window=immediate");

    if settings.enable_iina_danmaku {
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
    settings: &Settings,
) -> Result<(), String> {
    let mpv_bin = detect_mpv(settings)?;
    let mut command = Command::new(mpv_bin);
    command.arg("--ytdl=no");
    command.arg("--stream-lavf-o=reconnect_streamed=yes");
    command.arg(format!("--force-media-title={media_title}"));
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
        let _ = proxy_douyu_danmaku(socket, query.room_id, state).await;
    })
}

fn parse_room_id_from_client_message(message: &str) -> Option<String> {
    let payload = if let Some(raw) = message.strip_prefix("iinaDM://") {
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

    extract_room_id_from_target(payload)
}

async fn proxy_douyu_danmaku(
    mut client_stream: WebSocket,
    initial_room_id: String,
    state: DanmakuHttpState,
) -> Result<(), String> {
    client_stream
        .send(AxumMessage::Text(
            danmaku_text_event("Stream Hub 本地弹幕服务已连接")?,
        ))
        .await
        .map_err(|err| err.to_string())?;

    let room_id = if !initial_room_id.trim().is_empty() {
        initial_room_id
    } else {
        let mut parsed_room_id = String::new();
        while let Some(Ok(message)) = client_stream.next().await {
            match message {
                AxumMessage::Text(text) => {
                    if let Some(room_id) = parse_room_id_from_client_message(&text) {
                        parsed_room_id = room_id;
                        break;
                    }
                }
                AxumMessage::Close(_) => return Ok(()),
                _ => {}
            }
        }
        parsed_room_id
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
        .arg(&room_id)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| err.to_string())?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "斗鱼弹幕桥接未输出 stdout。".to_string())?;
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

fn search_streamers_inner(keyword: &str) -> Result<Vec<SearchStreamer>, String> {
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
    read_json_or_default(&path)
}

#[tauri::command]
fn save_streamers(app: AppHandle, streamers: Vec<Streamer>) -> Result<Vec<Streamer>, String> {
    let path = app_data_file(&app, "streamers.json")?;
    write_json(&path, &streamers)?;
    Ok(streamers)
}

#[tauri::command]
fn load_settings(app: AppHandle) -> Result<Settings, String> {
    let path = app_data_file(&app, "settings.json")?;
    read_json_or_default(&path)
}

#[tauri::command]
fn save_settings(app: AppHandle, settings: Settings) -> Result<Settings, String> {
    let path = app_data_file(&app, "settings.json")?;
    write_json(&path, &settings)?;
    Ok(settings)
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
        target: parsed.room_id.clone(),
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
        let updated: Vec<Streamer> = streamers
            .into_iter()
            .map(|mut streamer| {
                match fetch_room_state(&streamer.target) {
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
    let data = extract_play_info(&streamer.target)?;
    if !data.is_online {
        return Err("主播当前未开播".into());
    }
    if data.urls.is_empty() {
        return Err("未获取到可播放的直播地址".into());
    }

    let playlist_path = write_playlist(&data.title, &data.urls)?;
    let media_title = if data.title.trim().is_empty() {
        "Douyu Live".to_string()
    } else {
        data.title.clone()
    };

    match normalize_player(&settings).as_str() {
        "iina" => open_iina_playlist(&app, &settings, &playlist_path, &media_title, &data),
        _ => open_mpv_playlist(&playlist_path, &media_title, &settings),
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
            load_settings,
            save_settings,
            install_iina_danmaku_plugin,
            resolve_streamer,
            search_streamers,
            sync_streamers_status,
            play_streamer
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
