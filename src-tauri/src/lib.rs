use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, ORIGIN, REFERER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Settings {
    mpv_path: String,
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

fn douyu_client() -> Result<Client, String> {
    Client::builder()
        .build()
        .map_err(|err| err.to_string())
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
        is_living: value_to_string(&room["status"]) == "1"
            || matches!(room["show_status"].as_i64(), Some(1 | 2)),
        streamer_name: value_to_string(&room["nickname"]),
        room_name: value_to_string(&room["room_name"]),
        avatar_url: if avatar_big.is_empty() { avatar_mid } else { avatar_big },
    })
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
        is_online: value_to_string(&room["status"]) == "1"
            || matches!(room["show_status"].as_i64(), Some(1 | 2)),
        screenshot_url: value_to_string(&room["room_pic"]),
        heat_text: value_to_string(&room["room_biz_all"]["hot"]),
    })
}

fn get_search_live_state(keyword: &str, room_id: &str) -> Result<Option<bool>, String> {
    let url = format!(
        "https://www.douyu.com/japi/search/api/searchUser?kw={keyword}&page=1&pageSize=10"
    );
    let json = fetch_json(&url, Some("https://www.douyu.com/search/"))?;
    let users = json["data"]["relateUser"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    for item in users {
        let anchor = &item["anchorInfo"];
        if value_to_string(&anchor["rid"]) == room_id {
            return Ok(Some(anchor["isLive"].as_i64().unwrap_or_default() == 1));
        }
    }

    Ok(None)
}

fn fetch_room_state(target: &str) -> Result<ExtractResult, String> {
    if let Some(room_id) = extract_room_id_from_target(target) {
        let mut snapshot = get_room_snapshot(&room_id)?;
        if let Ok(Some(is_live)) = get_search_live_state(&room_id, &room_id) {
            snapshot.is_online = is_live;
        }
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

fn search_streamers_inner(keyword: &str) -> Result<Vec<SearchStreamer>, String> {
    let query = keyword.trim();
    if query.is_empty() {
        return Err("Missing keyword".into());
    }

    let url = format!(
        "https://www.douyu.com/japi/search/api/searchUser?kw={}&page=1&pageSize=30",
        query
    );
    let json = fetch_json(&url, Some("https://www.douyu.com/search/"))?;
    let users = json["data"]["relateUser"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let mut results = Vec::new();
    for item in users {
        let anchor = &item["anchorInfo"];
        let room_id = value_to_string(&anchor["rid"]);
        let name = value_to_string(&anchor["nickName"]);
        if room_id.is_empty() || name.is_empty() {
            continue;
        }

        let is_online = anchor["isLive"].as_i64().unwrap_or_default() == 1;
        let mut result = SearchStreamer {
            name,
            target: room_id.clone(),
            room_id,
            room_name: value_to_string(&anchor["description"]),
            avatar_url: value_to_string(&anchor["avatar"]),
            is_online,
            screenshot_url: value_to_string(&anchor["roomSrc"]),
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
fn sync_streamers_status(app: AppHandle, streamers: Vec<Streamer>) -> Result<Vec<Streamer>, String> {
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
}

#[tauri::command]
fn play_streamer(streamer: Streamer, settings: Settings) -> Result<(), String> {
    let data = extract_play_info(&streamer.target)?;
    if !data.is_online {
        return Err("主播当前未开播".into());
    }
    if data.urls.is_empty() {
        return Err("未获取到可播放的直播地址".into());
    }

    let playlist_path = write_playlist(&data.title, &data.urls)?;
    let mpv_bin = detect_mpv(&settings)?;
    let media_title = if data.title.trim().is_empty() {
        "Douyu Live".to_string()
    } else {
        data.title
    };

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

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            load_streamers,
            save_streamers,
            load_settings,
            save_settings,
            resolve_streamer,
            search_streamers,
            sync_streamers_status,
            play_streamer
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
