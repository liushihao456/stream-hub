import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import Hls from "hls.js";
import mpegts from "mpegts.js";

const tabs = [
	{ id: "favorites", label: "收藏" },
	{ id: "search", label: "搜索" },
	{ id: "settings", label: "设置" },
];

const searchPlatforms = ["douyu", "bilibili_live", "huya", "douyin_live"];
const PLAYBACK_BACK_HIDE_DELAY_MS = 2000;
const DEFAULT_DANMAKU_FONT_SIZE = 18;
const DANMAKU_TOP_PADDING = 24;
const DANMAKU_BOTTOM_PADDING = 28;
const DANMAKU_SPEED_PX_PER_SECOND = 120;
const DANMAKU_EXIT_PADDING_PX = 64;
const DANMAKU_ROW_GAP_PX = 96;
const DANMAKU_MAX_FLUSH_PER_FRAME = 6;
const DANMAKU_MAX_PENDING_TEXTS = 120;
const IS_MAC_PLATFORM =
	typeof navigator !== "undefined" && /Mac/.test(navigator.platform);
function parseDanmakuPayload(raw) {
	try {
		const payload = JSON.parse(raw);
		const kind = typeof payload?.kind === "string" ? payload.kind : "chat";
		const texts = Array.isArray(payload?.dms)
			? payload.dms
					.map((item) =>
						typeof item?.text === "string" ? item.text.trim() : "",
					)
					.filter(Boolean)
			: [];
		return { kind, texts };
	} catch {
		const text = String(raw).trim();
		return {
			kind: "chat",
			texts: text ? [text] : [],
		};
	}
}

function PlatformIcon({ platform, iconUrls }) {
	const iconMap = {
		douyu: { label: "斗鱼", url: iconUrls.douyu, className: "douyu" },
		bilibili_live: {
			label: "B站",
			url: iconUrls.bilibili,
			className: "bilibili",
		},
		huya: { label: "虎牙", url: iconUrls.huya, className: "huya" },
		douyin_live: { label: "抖音", url: iconUrls.douyin, className: "douyin" },
	};
	const config = iconMap[platform] || iconMap.douyu;

	return (
		<span
			className={`platform-icon ${config.className}`}
			aria-label={config.label}
			title={config.label}
		>
			{config.url ? (
				<img
					className="platform-icon-image"
					src={config.url}
					alt=""
					aria-hidden="true"
				/>
			) : (
				<span className="platform-icon-fallback" aria-hidden="true">
					{config.label}
				</span>
			)}
		</span>
	);
}

function RefreshIcon() {
	return (
		<svg viewBox="0 0 24 24" aria-hidden="true" className="refresh-icon">
			<path
				d="M20 12a8 8 0 1 1-2.34-5.66"
				fill="none"
				stroke="currentColor"
				strokeWidth="1.8"
				strokeLinecap="round"
			/>
			<path
				d="M20 4v4h-4"
				fill="none"
				stroke="currentColor"
				strokeWidth="1.8"
				strokeLinecap="round"
				strokeLinejoin="round"
			/>
		</svg>
	);
}

function LeftChevronIcon() {
	return (
		<svg viewBox="0 0 24 24" aria-hidden="true" className="playback-back-icon">
			<path
				d="M14.5 5.5 8 12l6.5 6.5"
				fill="none"
				stroke="currentColor"
				strokeWidth="1.9"
				strokeLinecap="round"
				strokeLinejoin="round"
			/>
		</svg>
	);
}

function FullscreenIcon({ active }) {
	return (
		<svg
			viewBox="0 0 24 24"
			aria-hidden="true"
			className="playback-fullscreen-icon"
		>
			{active ? (
				<>
					<path
						d="M9.5 4.75v4.75H4.75"
						fill="none"
						stroke="currentColor"
						strokeWidth="1.8"
						strokeLinecap="round"
						strokeLinejoin="round"
					/>
					<path
						d="m4.75 4.75 4.75 4.75"
						fill="none"
						stroke="currentColor"
						strokeWidth="1.8"
						strokeLinecap="round"
						strokeLinejoin="round"
					/>
					<path
						d="M14.5 19.25V14.5h4.75"
						fill="none"
						stroke="currentColor"
						strokeWidth="1.8"
						strokeLinecap="round"
						strokeLinejoin="round"
					/>
					<path
						d="m19.25 19.25-4.75-4.75"
						fill="none"
						stroke="currentColor"
						strokeWidth="1.8"
						strokeLinecap="round"
						strokeLinejoin="round"
					/>
				</>
			) : (
				<>
					<path
						d="M4.75 9.5V4.75H9.5"
						fill="none"
						stroke="currentColor"
						strokeWidth="1.8"
						strokeLinecap="round"
						strokeLinejoin="round"
					/>
					<path
						d="m4.75 4.75 5 5"
						fill="none"
						stroke="currentColor"
						strokeWidth="1.8"
						strokeLinecap="round"
						strokeLinejoin="round"
					/>
					<path
						d="M19.25 14.5v4.75H14.5"
						fill="none"
						stroke="currentColor"
						strokeWidth="1.8"
						strokeLinecap="round"
						strokeLinejoin="round"
					/>
					<path
						d="m19.25 19.25-5-5"
						fill="none"
						stroke="currentColor"
						strokeWidth="1.8"
						strokeLinecap="round"
						strokeLinejoin="round"
					/>
				</>
			)}
		</svg>
	);
}

function formatPlaybackTime(totalSeconds) {
	const safeTotal = Math.max(
		0,
		Math.floor(Number.isFinite(totalSeconds) ? totalSeconds : 0),
	);
	const hours = Math.floor(safeTotal / 3600);
	const minutes = Math.floor((safeTotal % 3600) / 60);
	const seconds = safeTotal % 60;

	if (hours > 0) {
		return `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
	}

	return `${minutes}:${String(seconds).padStart(2, "0")}`;
}

function formatLiveOffset(totalSeconds) {
	return `-${formatPlaybackTime(totalSeconds)}`;
}

function estimateDanmakuTextWidth(text, fontSize) {
	let units = 0;
	for (const char of text) {
		if (/\s/.test(char)) {
			units += 0.35;
		} else if (char.charCodeAt(0) <= 0x00ff) {
			units += 0.58;
		} else {
			units += 1;
		}
	}
	return Math.ceil((units + 0.4) * fontSize);
}

function streamerSyncKeys(streamer) {
	const platform = streamer.platform || "";
	const target = streamer.target || "";
	return [
		streamer.id ? `id:${streamer.id}` : "",
		platform || target ? `target:${platform}:${target}` : "",
	].filter(Boolean);
}

function mergeSyncedStreamerUpdates(
	currentStreamers,
	sourceStreamers,
	syncedStreamers,
) {
	const updates = new Map();
	syncedStreamers.forEach((syncedStreamer, index) => {
		const sourceStreamer = sourceStreamers[index];
		if (!sourceStreamer) {
			return;
		}
		for (const key of streamerSyncKeys(sourceStreamer)) {
			updates.set(key, syncedStreamer);
		}
	});

	return currentStreamers.map((streamer) => {
		const update = streamerSyncKeys(streamer)
			.map((key) => updates.get(key))
			.find(Boolean);
		return update
			? { ...streamer, ...update, id: streamer.id || update.id }
			: streamer;
	});
}

function getImageForStreamer(streamer) {
	if (streamer.isOnline && streamer.screenshotUrl) {
		return streamer.screenshotUrl;
	}
	return streamer.avatarUrl || streamer.screenshotUrl || "";
}

function StreamerCard({
	streamer,
	iconUrls,
	menuId,
	openMenuId,
	setOpenMenuId,
	onPlay,
	menuLabel,
	onMenuAction,
	menuDisabled = false,
	menuTone = "",
}) {
	const imageUrl = getImageForStreamer(streamer);
	const [isClicking, setIsClicking] = useState(false);
	const clickTimerRef = useRef(null);

	useEffect(() => {
		return () => {
			if (clickTimerRef.current) {
				window.clearTimeout(clickTimerRef.current);
			}
		};
	}, []);

	function triggerClickFeedback() {
		if (clickTimerRef.current) {
			window.clearTimeout(clickTimerRef.current);
		}
		setIsClicking(false);
		requestAnimationFrame(() => {
			setIsClicking(true);
			clickTimerRef.current = window.setTimeout(() => {
				setIsClicking(false);
			}, 520);
		});
	}

	return (
		<article
			className={`favorite-card ${isClicking ? "is-clicking" : ""}`}
			onMouseLeave={(event) => {
				setOpenMenuId((current) => (current === menuId ? null : current));
				if (event.currentTarget.contains(document.activeElement)) {
					document.activeElement?.blur();
				}
			}}
			onClick={() => {
				triggerClickFeedback();
				onPlay(streamer);
			}}
			onKeyDown={(event) => {
				if (event.key === "Enter" || event.key === " ") {
					event.preventDefault();
					triggerClickFeedback();
					onPlay(streamer);
				}
			}}
			role="button"
			tabIndex={0}
		>
			<details
				className="card-menu"
				open={openMenuId === menuId}
				onClick={(event) => event.stopPropagation()}
				onKeyDown={(event) => event.stopPropagation()}
			>
				<summary
					className="menu-trigger"
					aria-label={`${streamer.name} 更多操作`}
					title="更多操作"
					onClick={(event) => {
						event.preventDefault();
						event.stopPropagation();
						setOpenMenuId((current) => (current === menuId ? null : menuId));
					}}
				>
					<span />
					<span />
					<span />
				</summary>
				<div className="menu-popover">
					<button
						type="button"
						className={`menu-item ${menuTone}`.trim()}
						disabled={menuDisabled}
						onClick={(event) => {
							event.stopPropagation();
							onMenuAction(streamer);
						}}
					>
						{menuLabel}
					</button>
				</div>
			</details>
			<div className="avatar-wrap">
				<PlatformIcon platform={streamer.platform} iconUrls={iconUrls} />
				{imageUrl ? (
					<img className="avatar-image" src={imageUrl} alt={streamer.name} />
				) : (
					<div className="avatar-fallback">{streamer.name.slice(0, 1)}</div>
				)}
				{streamer.isOnline ? (
					<div className="card-play-indicator" aria-hidden="true">
						<span className="play-triangle" />
					</div>
				) : null}
			</div>
			<div className="card-caption">
				<h3>{streamer.name}</h3>
			</div>
			{streamer.heatText ? (
				<div className="card-heat">
					<span
						className={`heat-dot ${streamer.isOnline ? "live" : "offline"}`}
					/>
					<span>{streamer.heatText}</span>
				</div>
			) : null}
		</article>
	);
}

function StreamerGroup({
	title,
	streamers,
	emptyText,
	iconUrls,
	getMenuProps,
	openMenuId,
	setOpenMenuId,
	onPlay,
}) {
	return (
		<section className="favorites-group">
			<div className="group-heading">
				<h2>{title}</h2>
				<span className="count-chip">{streamers.length}</span>
			</div>
			{streamers.length === 0 ? (
				<div className="group-empty">{emptyText}</div>
			) : (
				<div className="card-grid">
					{streamers.map((streamer) => {
						const menu = getMenuProps(streamer);
						return (
							<StreamerCard
								key={menu.menuId}
								streamer={streamer}
								iconUrls={iconUrls}
								menuId={menu.menuId}
								openMenuId={openMenuId}
								setOpenMenuId={setOpenMenuId}
								onPlay={onPlay}
								menuLabel={menu.label}
								onMenuAction={menu.onAction}
								menuDisabled={menu.disabled}
								menuTone={menu.tone}
							/>
						);
					})}
				</div>
			)}
		</section>
	);
}

function PlaybackPage({
	pageRef,
	surfaceRef,
	controlsVisible,
	fullscreenActive,
	children,
	playbackTitle,
	playbackPositionSeconds,
	playbackTrackStartSeconds,
	playbackTrackEndSeconds,
	playbackTrackMode,
	playbackSeekable,
	playbackTrackDragging,
	danmakuItems,
	danmakuFontSize,
	onBack,
	onToggleFullscreen,
	onPointerMove,
	onPointerLeave,
	onStageDoubleClick,
	onTrackPointerDown,
	onTrackPointerMove,
	onTrackPointerUp,
	onTrackPointerCancel,
}) {
	const trackStart = Number.isFinite(playbackTrackStartSeconds)
		? playbackTrackStartSeconds
		: 0;
	const trackEnd = Number.isFinite(playbackTrackEndSeconds)
		? playbackTrackEndSeconds
		: 0;
	const normalizedDuration = trackEnd > trackStart ? trackEnd - trackStart : 0;
	const normalizedPosition =
		normalizedDuration > 0
			? Math.min(trackEnd, Math.max(trackStart, playbackPositionSeconds))
			: trackStart;
	const progressPercent =
		playbackSeekable && normalizedDuration > 0
			? `${((normalizedPosition - trackStart) / normalizedDuration) * 100}%`
			: "100%";
	const isLiveCacheTrack = playbackTrackMode === "live-cache";

	return (
		<main
			ref={pageRef}
			className="playback-page"
			tabIndex={-1}
			onPointerMove={onPointerMove}
			onPointerLeave={onPointerLeave}
			onPointerDown={() => {
				pageRef.current?.focus?.({ preventScroll: true });
			}}
		>
			<div
				ref={surfaceRef}
				className="playback-stage"
				aria-label="直播画面区域"
				onDoubleClick={onStageDoubleClick}
			>
				{children}
				<div className="playback-danmaku-layer" aria-hidden="true">
					{danmakuItems.map((item) => (
						<div
							key={item.id}
							className="playback-danmaku-item"
							style={{
								top: `${item.top}px`,
								fontSize: `${danmakuFontSize}px`,
								"--danmaku-distance": `${item.distancePx}px`,
								"--danmaku-translate-x": `${-item.distancePx}px`,
								animationDuration: `${item.durationMs}ms`,
							}}
						>
							{item.text}
						</div>
					))}
				</div>
			</div>
			<button
				type="button"
				className={`playback-back-button ${controlsVisible ? "visible" : ""}`}
				onClick={onBack}
				aria-label="返回"
				title="返回"
			>
				<LeftChevronIcon />
			</button>
			<button
				type="button"
				className={`playback-fullscreen-button ${controlsVisible ? "visible" : ""}`}
				onClick={onToggleFullscreen}
				aria-label={fullscreenActive ? "退出全屏" : "进入全屏"}
				title={fullscreenActive ? "退出全屏" : "进入全屏"}
			>
				<FullscreenIcon active={fullscreenActive} />
			</button>
			<div
				className={`playback-track-region ${controlsVisible ? "visible" : ""}`}
			>
				<div className="playback-track-gradient" aria-hidden="true" />
				<section className="playback-track-panel" aria-label="播放轨道">
					<div className="playback-track-meta">
						<p className="playback-track-title">
							{playbackTitle || "正在播放"}
						</p>
						{isLiveCacheTrack ? (
							<span className="playback-track-live-time">
								<span className="playback-track-live-pill">LIVE</span>
								<span className="playback-track-time">
									{formatPlaybackTime(normalizedPosition - trackStart)} /{" "}
									{formatPlaybackTime(normalizedDuration)}
								</span>
							</span>
						) : playbackSeekable ? (
							<span className="playback-track-time">
								{formatPlaybackTime(normalizedPosition - trackStart)} /{" "}
								{formatPlaybackTime(normalizedDuration)}
							</span>
						) : (
							<span className="playback-track-live-pill">LIVE</span>
						)}
					</div>
					<div
						className={`playback-track-rail ${playbackSeekable ? "seekable" : "live"} ${isLiveCacheTrack ? "live-cache" : ""} ${playbackTrackDragging ? "dragging" : ""}`.trim()}
						onPointerDown={onTrackPointerDown}
						onPointerMove={onTrackPointerMove}
						onPointerUp={onTrackPointerUp}
						onPointerCancel={onTrackPointerCancel}
						onLostPointerCapture={onTrackPointerCancel}
						aria-disabled={!playbackSeekable}
					>
						<div className="playback-track-rail-base" aria-hidden="true" />
						<div
							className="playback-track-rail-fill"
							aria-hidden="true"
							style={{ width: progressPercent }}
						/>
						<div
							className="playback-track-thumb"
							aria-hidden="true"
							style={{ left: progressPercent }}
						/>
					</div>
				</section>
			</div>
		</main>
	);
}

function detectFrontendStreamType(url) {
	const normalized = String(url || "").split("?")[0].toLowerCase();
	if (normalized.endsWith(".m3u8")) {
		return "hls";
	}
	return "flv";
}

function FrontendVideoPlayer({ playInfo, onReady, onError }) {
	const videoRef = useRef(null);

	useEffect(() => {
		const video = videoRef.current;
		const url = playInfo?.proxyUrls?.[0] || playInfo?.urls?.[0] || "";
		if (!video || !url) {
			return undefined;
		}

		let disposed = false;
		let player = null;
		let hls = null;

		async function startNativeVideo(src) {
			video.src = src;
			video.load();
			try {
				await video.play();
				onReady?.();
			} catch (err) {
				if (!disposed) {
					onError?.(`网页播放器启动失败：${String(err)}`);
				}
			}
		}

		const type = detectFrontendStreamType(url);
		if (type === "hls") {
			if (video.canPlayType("application/vnd.apple.mpegurl")) {
				void startNativeVideo(url);
			} else if (Hls.isSupported()) {
				hls = new Hls({ lowLatencyMode: true });
				hls.on(Hls.Events.ERROR, (_event, data) => {
					if (data?.fatal && !disposed) {
						onError?.(`HLS 播放失败：${data?.details || data?.type || "未知错误"}`);
					}
				});
				hls.loadSource(url);
				hls.attachMedia(video);
				video.play()
					.then(() => onReady?.())
					.catch((err) => {
						if (!disposed) {
							onError?.(`网页播放器启动失败：${String(err)}`);
						}
					});
			} else {
				onError?.("当前 WebView 不支持 HLS/MSE 播放");
			}
		} else if (mpegts.getFeatureList?.()?.mseLivePlayback || mpegts.isSupported()) {
			player = mpegts.createPlayer({ type: "flv", isLive: true, url });
			player.on(mpegts.Events.ERROR, (typeName, detail) => {
				if (!disposed) {
					onError?.(`FLV 播放失败：${typeName || "未知错误"} ${detail || ""}`.trim());
				}
			});
			player.attachMediaElement(video);
			player.load();
			video.play()
				.then(() => onReady?.())
				.catch((err) => {
					if (!disposed) {
						onError?.(`网页播放器启动失败：${String(err)}`);
					}
				});
		} else {
			onError?.("当前 WebView 不支持 MSE，无法使用网页 FLV 播放器");
		}

		return () => {
			disposed = true;
			try {
				player?.unload?.();
				player?.detachMediaElement?.();
				player?.destroy?.();
				hls?.destroy?.();
				video.pause();
				video.removeAttribute("src");
				video.load();
			} catch {
				// Ignore cleanup errors from media internals.
			}
		};
	}, [playInfo, onReady, onError]);

	return <video ref={videoRef} className="frontend-video" playsInline autoPlay />;
}

function App() {
	const [streamers, setStreamers] = useState([]);
	const [platformIconUrls, setPlatformIconUrls] = useState({
		bilibili: "",
		douyu: "",
		huya: "",
		douyin: "",
	});
	const [settings, setSettings] = useState({
		bilibiliCookie: "",
		danmakuFontSize: DEFAULT_DANMAKU_FONT_SIZE,
	});
	const [activeTab, setActiveTab] = useState("favorites");
	const [searchInput, setSearchInput] = useState("");
	const [searchResults, setSearchResults] = useState([]);
	const [searchPerformed, setSearchPerformed] = useState(false);
	const [searching, setSearching] = useState(false);
	const [openMenuId, setOpenMenuId] = useState(null);
	const [loading, setLoading] = useState(true);
	const [saving, setSaving] = useState(false);
	const [refreshing, setRefreshing] = useState(false);
	const [message, setMessage] = useState("");
	const [error, setError] = useState("");
	const [viewMode, setViewMode] = useState("browse");
	const [playbackBackVisible, setPlaybackBackVisible] = useState(false);
	const [playbackFullscreen, setPlaybackFullscreen] = useState(false);
	const [currentPlaybackStreamer, setCurrentPlaybackStreamer] = useState(null);
	const [frontendPlayback, setFrontendPlayback] = useState({ phase: "idle", playInfo: null, errorMessage: "" });
	const [danmakuItems, setDanmakuItems] = useState([]);
	const playbackPageRef = useRef(null);
	const playerSurfaceRef = useRef(null);
	const playbackOriginRef = useRef({ activeTab: "favorites", scrollY: 0 });
	const restoreScrollRef = useRef(null);
	const playbackBackHideTimerRef = useRef(0);
	const playbackPointerPositionRef = useRef({ x: null, y: null });
	const playbackControlsHoldCountRef = useRef(0);
	const playbackExitInFlightRef = useRef(false);
	const playbackAwaitingStartRef = useRef(false);
	const playbackFullscreenRef = useRef(false);
	const playbackFullscreenCommandInFlightRef = useRef(false);
	const playbackFullscreenToggleQueuedRef = useRef(false);
	const playbackFocusTimersRef = useRef([]);
	const danmakuSocketRef = useRef(null);
	const danmakuTimersRef = useRef(new Map());
	const danmakuRowAvailabilityRef = useRef([]);
	const danmakuPendingTextsRef = useRef([]);
	const danmakuFlushFrameRef = useRef(0);
	const danmakuNextIdRef = useRef(1);

	function clearPlaybackBackHideTimer() {
		if (playbackBackHideTimerRef.current) {
			window.clearTimeout(playbackBackHideTimerRef.current);
			playbackBackHideTimerRef.current = 0;
		}
	}

	function schedulePlaybackBackHide() {
		if (viewMode !== "playback" || playbackControlsHoldCountRef.current > 0) {
			return;
		}
		clearPlaybackBackHideTimer();
		playbackBackHideTimerRef.current = window.setTimeout(() => {
			setPlaybackBackVisible(false);
			playbackBackHideTimerRef.current = 0;
		}, PLAYBACK_BACK_HIDE_DELAY_MS);
	}

	function holdPlaybackControls() {
		playbackControlsHoldCountRef.current += 1;
		clearPlaybackBackHideTimer();
		setPlaybackBackVisible(true);
	}

	function releasePlaybackControls() {
		playbackControlsHoldCountRef.current = Math.max(
			0,
			playbackControlsHoldCountRef.current - 1,
		);
		if (playbackControlsHoldCountRef.current === 0) {
			schedulePlaybackBackHide();
		}
	}

	function clearPlaybackFocusTimers() {
		for (const timer of playbackFocusTimersRef.current) {
			window.clearTimeout(timer);
		}
		playbackFocusTimersRef.current = [];
	}

	function focusPlaybackPage() {
		const page = playbackPageRef.current;
		if (!page) {
			return;
		}

		page.focus?.({ preventScroll: true });
	}

	function schedulePlaybackFocusRefresh() {
		clearPlaybackFocusTimers();
		focusPlaybackPage();
		playbackFocusTimersRef.current = [120, 360, 900].map((delay) =>
			window.setTimeout(() => {
				if (document.documentElement.classList.contains("playback-mode")) {
					focusPlaybackPage();
				}
			}, delay),
		);
	}

	async function setPlaybackFullscreenMode(value, { reportError = true } = {}) {
		const nextValue = Boolean(value);
		try {
			if (nextValue && !document.fullscreenElement) {
				await document.documentElement.requestFullscreen?.();
			} else if (!nextValue && document.fullscreenElement) {
				await document.exitFullscreen?.();
			}
			playbackFullscreenRef.current = nextValue;
			setPlaybackFullscreen(nextValue);
			return true;
		} catch (err) {
			if (reportError) {
				setError(String(err));
			}
			return false;
		}
	}

	async function togglePlaybackFullscreen() {
		if (playbackFullscreenCommandInFlightRef.current) {
			playbackFullscreenToggleQueuedRef.current = true;
			return;
		}

		playbackFullscreenCommandInFlightRef.current = true;
		schedulePlaybackFocusRefresh();
		revealPlaybackControls();
		try {
			await setPlaybackFullscreenMode(!playbackFullscreenRef.current);
		} finally {
			playbackFullscreenCommandInFlightRef.current = false;
			schedulePlaybackFocusRefresh();
			if (playbackFullscreenToggleQueuedRef.current) {
				playbackFullscreenToggleQueuedRef.current = false;
				window.setTimeout(() => {
					if (document.documentElement.classList.contains("playback-mode")) {
						void togglePlaybackFullscreen();
					}
				}, 0);
			}
		}
	}

	async function handlePlaybackEscapeKey() {
		if (playbackFullscreenRef.current) {
			await setPlaybackFullscreenMode(false);
			schedulePlaybackFocusRefresh();
			revealPlaybackControls();
			return;
		}

		await exitPlaybackView();
	}

	function normalizeDanmakuFontSize(value) {
		const parsed = Number(value);
		if (!Number.isFinite(parsed)) {
			return DEFAULT_DANMAKU_FONT_SIZE;
		}
		return Math.min(28, Math.max(9, Math.round(parsed)));
	}

	function clearDanmakuTimers() {
		for (const timer of danmakuTimersRef.current.values()) {
			window.clearTimeout(timer);
		}
		danmakuTimersRef.current.clear();
	}

	function clearDanmakuState() {
		if (danmakuFlushFrameRef.current) {
			window.cancelAnimationFrame(danmakuFlushFrameRef.current);
			danmakuFlushFrameRef.current = 0;
		}
		danmakuPendingTextsRef.current = [];
		clearDanmakuTimers();
		danmakuRowAvailabilityRef.current = [];
		setDanmakuItems([]);
	}

	function removeDanmakuItem(id) {
		const timer = danmakuTimersRef.current.get(id);
		if (timer) {
			window.clearTimeout(timer);
			danmakuTimersRef.current.delete(id);
		}
		setDanmakuItems((current) => current.filter((item) => item.id !== id));
	}

	function scheduleDanmakuFlush() {
		if (danmakuFlushFrameRef.current) {
			return;
		}

		danmakuFlushFrameRef.current = window.requestAnimationFrame(() => {
			danmakuFlushFrameRef.current = 0;
			const pending = danmakuPendingTextsRef.current.splice(
				0,
				DANMAKU_MAX_FLUSH_PER_FRAME,
			);
			for (const text of pending) {
				enqueueDanmaku(text);
			}
			if (danmakuPendingTextsRef.current.length > 0) {
				scheduleDanmakuFlush();
			}
		});
	}

	function queueDanmakuText(text) {
		danmakuPendingTextsRef.current.push(text);
		if (danmakuPendingTextsRef.current.length > DANMAKU_MAX_PENDING_TEXTS) {
			danmakuPendingTextsRef.current.splice(
				0,
				danmakuPendingTextsRef.current.length - DANMAKU_MAX_PENDING_TEXTS,
			);
		}
		scheduleDanmakuFlush();
	}

	function enqueueDanmaku(text) {
		const content = text.trim();
		if (!content) {
			return;
		}

		const danmakuFontSize = normalizeDanmakuFontSize(settings.danmakuFontSize);
		const danmakuRowHeight = danmakuFontSize + 12;
		const stageWidth =
			playerSurfaceRef.current?.clientWidth || window.innerWidth || 1280;
		const stageHeight =
			playerSurfaceRef.current?.clientHeight || window.innerHeight || 720;
		const textWidth = estimateDanmakuTextWidth(content, danmakuFontSize);
		const distancePx = Math.ceil(
			stageWidth + textWidth + DANMAKU_EXIT_PADDING_PX,
		);
		const availableHeight = Math.max(
			danmakuRowHeight,
			stageHeight - DANMAKU_TOP_PADDING - DANMAKU_BOTTOM_PADDING,
		);
		const rowCount = Math.max(
			4,
			Math.floor(availableHeight / danmakuRowHeight),
		);
		const now = performance.now();
		const rowAvailability = danmakuRowAvailabilityRef.current;

		while (rowAvailability.length < rowCount) {
			rowAvailability.push(0);
		}

		let rowIndex = 0;
		let earliestAvailable = rowAvailability[0] || 0;
		for (let index = 0; index < rowCount; index += 1) {
			const availableAt = rowAvailability[index] || 0;
			if (availableAt <= now) {
				rowIndex = index;
				earliestAvailable = availableAt;
				break;
			}
			if (availableAt < earliestAvailable) {
				rowIndex = index;
				earliestAvailable = availableAt;
			}
		}

		const gapMs = Math.ceil(
			((textWidth + DANMAKU_ROW_GAP_PX) / DANMAKU_SPEED_PX_PER_SECOND) * 1000,
		);
		rowAvailability[rowIndex] = Math.max(now, earliestAvailable) + gapMs;

		const durationMs = Math.ceil(
			(distancePx / DANMAKU_SPEED_PX_PER_SECOND) * 1000,
		);
		const item = {
			id: danmakuNextIdRef.current,
			text: content,
			top: DANMAKU_TOP_PADDING + rowIndex * danmakuRowHeight,
			distancePx,
			durationMs,
		};
		danmakuNextIdRef.current += 1;

		setDanmakuItems((current) => {
			const next = [...current, item];
			return next.length > 80 ? next.slice(next.length - 80) : next;
		});

		const timer = window.setTimeout(() => {
			removeDanmakuItem(item.id);
		}, durationMs + 260);
		danmakuTimersRef.current.set(item.id, timer);
	}

	function restoreBrowseView() {
		playbackAwaitingStartRef.current = false;
		clearPlaybackBackHideTimer();
		clearPlaybackFocusTimers();
		playbackControlsHoldCountRef.current = 0;
		playbackPointerPositionRef.current = { x: null, y: null };
		playbackFullscreenToggleQueuedRef.current = false;
		clearDanmakuState();
		setCurrentPlaybackStreamer(null);
		setFrontendPlayback({ phase: "idle", playInfo: null, errorMessage: "" });
		setPlaybackBackVisible(false);
		playbackFullscreenRef.current = false;
		setPlaybackFullscreen(false);
		setViewMode("browse");
		setActiveTab(playbackOriginRef.current.activeTab);
		restoreScrollRef.current = playbackOriginRef.current.scrollY;
	}

	function revealPlaybackControls() {
		if (viewMode !== "playback") {
			return;
		}
		setPlaybackBackVisible(true);
		schedulePlaybackBackHide();
	}

	function hidePlaybackControls() {
		clearPlaybackBackHideTimer();
		playbackPointerPositionRef.current = { x: null, y: null };
		if (playbackControlsHoldCountRef.current === 0) {
			setPlaybackBackVisible(false);
		}
	}

	function handlePlaybackPointerMove(event) {
		if (viewMode !== "playback") {
			return;
		}

		const nextX = Number(event.clientX);
		const nextY = Number(event.clientY);
		const lastPosition = playbackPointerPositionRef.current;
		if (lastPosition.x === nextX && lastPosition.y === nextY) {
			return;
		}

		playbackPointerPositionRef.current = { x: nextX, y: nextY };
		revealPlaybackControls();
	}

	function handlePlaybackPointerLeave(event) {
		if (viewMode !== "playback") {
			return;
		}

		const relatedTarget = event.relatedTarget;
		if (
			relatedTarget instanceof Node &&
			event.currentTarget.contains(relatedTarget)
		) {
			return;
		}

		hidePlaybackControls();
	}

	function handlePlaybackStageDoubleClick(event) {
		event.preventDefault();
		focusPlaybackPage();
		void togglePlaybackFullscreen();
	}

	function enterPlaybackView() {
		playbackAwaitingStartRef.current = true;
		playbackOriginRef.current = {
			activeTab,
			scrollY: window.scrollY || window.pageYOffset || 0,
		};
		restoreScrollRef.current = null;
		clearPlaybackBackHideTimer();
		playbackControlsHoldCountRef.current = 0;
		playbackPointerPositionRef.current = { x: null, y: null };
		playbackFullscreenToggleQueuedRef.current = false;
		setPlaybackBackVisible(false);
		setViewMode("playback");
		window.scrollTo(0, 0);
	}

	async function exitPlaybackView() {
		if (playbackExitInFlightRef.current) {
			return;
		}
		playbackExitInFlightRef.current = true;

		try {
			if (playbackFullscreenRef.current) {
				await setPlaybackFullscreenMode(false, { reportError: false });
			}
		} catch (err) {
			setError(String(err));
		} finally {
			restoreBrowseView();
			playbackExitInFlightRef.current = false;
		}
	}

	async function syncStreamerStatusInBackground(
		streamersToSync,
		{ showMessage = false, shouldApply = () => true } = {},
	) {
		if (!streamersToSync.length) {
			return;
		}

		try {
			const syncedStreamers = await invoke("sync_streamers_status", {
				streamers: streamersToSync,
			});
			if (!shouldApply()) {
				return;
			}
			setStreamers((current) =>
				mergeSyncedStreamerUpdates(current, streamersToSync, syncedStreamers),
			);
			if (showMessage) {
				setMessage("已刷新主播状态");
			}
		} catch (err) {
			if (shouldApply()) {
				setError(String(err));
			}
		}
	}

	useEffect(() => {
		let cancelled = false;
		let unlistenUpdated;
		let unlistenError;

		async function bootstrap() {
			try {
				const icons = await invoke("load_platform_icons");
				if (!cancelled) {
					setPlatformIconUrls(icons);
				}

				unlistenUpdated = await listen("bilibili-login-updated", (event) => {
					setSettings((current) => ({
						...current,
						...event.payload,
						danmakuFontSize: normalizeDanmakuFontSize(
							event.payload?.danmakuFontSize,
						),
					}));
					setMessage("已更新 B站 登录态");
					setError("");
				});
				unlistenError = await listen("bilibili-login-error", (event) => {
					setError(String(event.payload));
				});
				const [savedStreamers, savedSettings] = await Promise.all([
					invoke("load_streamers"),
					invoke("load_settings"),
				]);

				if (!cancelled) {
					setSettings({
						...savedSettings,
						danmakuFontSize: normalizeDanmakuFontSize(
							savedSettings.danmakuFontSize,
						),
					});
					setStreamers(savedStreamers);
					setLoading(false);

					if (savedStreamers.length > 0) {
						void syncStreamerStatusInBackground(savedStreamers, {
							shouldApply: () => !cancelled,
						});
					}
				}
			} catch (err) {
				if (!cancelled) {
					setError(String(err));
				}
			} finally {
				if (!cancelled) {
					setLoading(false);
				}
			}
		}

		bootstrap();
		return () => {
			cancelled = true;
			unlistenUpdated?.();
			unlistenError?.();
		};
	}, []);

	useEffect(() => {
		return () => {
			clearPlaybackBackHideTimer();
			clearPlaybackFocusTimers();
			clearDanmakuState();
			const socket = danmakuSocketRef.current;
			danmakuSocketRef.current = null;
			if (
				socket &&
				(socket.readyState === WebSocket.OPEN ||
					socket.readyState === WebSocket.CONNECTING)
			) {
				socket.close();
			}
		};
	}, []);

	useEffect(() => {
		if (viewMode !== "browse" || restoreScrollRef.current == null) {
			return undefined;
		}

		const scrollY = restoreScrollRef.current;
		const frame = window.requestAnimationFrame(() => {
			window.scrollTo(0, scrollY);
			restoreScrollRef.current = null;
		});

		return () => {
			window.cancelAnimationFrame(frame);
		};
	}, [viewMode, activeTab]);

	useEffect(() => {
		const html = document.documentElement;
		const body = document.body;
		const root = document.getElementById("root");
		const playbackActive = viewMode === "playback";

		html.classList.toggle("playback-mode", playbackActive);
		body.classList.toggle("playback-mode", playbackActive);
		root?.classList.toggle("playback-mode-root", playbackActive);

		return () => {
			html.classList.remove("playback-mode");
			body.classList.remove("playback-mode");
			root?.classList.remove("playback-mode-root");
		};
	}, [viewMode]);

	useEffect(() => {
		if (viewMode !== "playback") {
			return undefined;
		}

		const frame = window.requestAnimationFrame(() => {
			schedulePlaybackFocusRefresh();
		});

		return () => {
			window.cancelAnimationFrame(frame);
			clearPlaybackFocusTimers();
		};
	}, [viewMode]);

	useEffect(() => {
		if (viewMode !== "playback") {
			return undefined;
		}

		function handleWindowPointerOut(event) {
			if (event.relatedTarget == null) {
				hidePlaybackControls();
			}
		}

		function handleWindowBlur() {
			hidePlaybackControls();
		}

		window.addEventListener("pointerout", handleWindowPointerOut, true);
		window.addEventListener("mouseout", handleWindowPointerOut, true);
		window.addEventListener("blur", handleWindowBlur);
		document.documentElement.addEventListener("mouseleave", handleWindowBlur);

		return () => {
			window.removeEventListener("pointerout", handleWindowPointerOut, true);
			window.removeEventListener("mouseout", handleWindowPointerOut, true);
			window.removeEventListener("blur", handleWindowBlur);
			document.documentElement.removeEventListener(
				"mouseleave",
				handleWindowBlur,
			);
		};
	}, [viewMode]);

	useEffect(() => {
		if (viewMode !== "playback") {
			return undefined;
		}

		function handleKeyDown(event) {
			if (event.key === "Escape") {
				event.preventDefault();
				void handlePlaybackEscapeKey();
				return;
			}

			const key = typeof event.key === "string" ? event.key.toLowerCase() : "";
			const code = typeof event.code === "string" ? event.code : "";
			if (
				!IS_MAC_PLATFORM &&
				(key === "f" || code === "KeyF") &&
				!event.repeat &&
				!event.metaKey &&
				!event.ctrlKey &&
				!event.altKey
			) {
				event.preventDefault();
				void togglePlaybackFullscreen();
				return;
			}

		}

		document.addEventListener("keydown", handleKeyDown, true);
		return () => {
			document.removeEventListener("keydown", handleKeyDown, true);
		};
	}, [viewMode]);

	useEffect(() => {
		const target = currentPlaybackStreamer?.target?.trim();
		if (viewMode !== "playback" || !target) {
			const socket = danmakuSocketRef.current;
			danmakuSocketRef.current = null;
			if (
				socket &&
				(socket.readyState === WebSocket.OPEN ||
					socket.readyState === WebSocket.CONNECTING)
			) {
				socket.close();
			}
			clearDanmakuState();
			return undefined;
		}

		clearDanmakuState();
		let cancelled = false;
		let socket;

		async function connectDanmaku() {
			try {
				const port = await invoke("ensure_danmaku_server");
				if (cancelled) {
					return;
				}

				socket = new WebSocket(`ws://127.0.0.1:${port}/danmaku-websocket`);
				danmakuSocketRef.current = socket;

				socket.addEventListener("open", () => {
					if (cancelled || socket.readyState !== WebSocket.OPEN) {
						return;
					}
					socket.send(target);
				});

				socket.addEventListener("message", (event) => {
					if (cancelled || typeof event.data !== "string") {
						return;
					}
					const { kind, texts } = parseDanmakuPayload(event.data);
					for (const text of texts) {
						if (kind === "error") {
							setError(text);
							continue;
						}
						if (kind !== "chat") {
							continue;
						}
						queueDanmakuText(text);
					}
				});

				socket.addEventListener("error", () => {
					if (!cancelled) {
						setError("连接本地弹幕服务失败");
					}
				});
			} catch (err) {
				if (!cancelled) {
					setError(`启动弹幕服务失败：${String(err)}`);
				}
			}
		}

		void connectDanmaku();

		return () => {
			cancelled = true;
			if (danmakuSocketRef.current === socket) {
				danmakuSocketRef.current = null;
			}
			if (
				socket &&
				(socket.readyState === WebSocket.OPEN ||
					socket.readyState === WebSocket.CONNECTING)
			) {
				socket.close();
			}
			clearDanmakuState();
		};
	}, [viewMode, currentPlaybackStreamer?.target]);

	async function persistStreamers(
		nextStreamers,
		nextMessage = "主播列表已保存",
	) {
		setSaving(true);
		setError("");
		setMessage("");
		try {
			const saved = await invoke("save_streamers", {
				streamers: nextStreamers,
			});
			setStreamers(saved);
			setMessage(nextMessage);
			return saved;
		} catch (err) {
			setError(String(err));
			return null;
		} finally {
			setSaving(false);
		}
	}

	async function persistSettings(nextSettings, nextMessage = "设置已保存") {
		setSaving(true);
		setError("");
		setMessage("");
		try {
			const saved = await invoke("save_settings", {
				settings: {
					...nextSettings,
					danmakuFontSize: normalizeDanmakuFontSize(
						nextSettings.danmakuFontSize,
					),
				},
			});
			setSettings(saved);
			setMessage(nextMessage);
			return saved;
		} catch (err) {
			setError(String(err));
			return null;
		} finally {
			setSaving(false);
		}
	}

	async function handleOpenBilibiliLogin() {
		setError("");
		setMessage("");
		try {
			await invoke("open_bilibili_login");
			setMessage("已打开 B站 登录窗口。登录成功后会自动保存。");
		} catch (err) {
			setError(String(err));
		}
	}

	async function handleClearBilibiliLogin() {
		setSaving(true);
		setError("");
		setMessage("");
		try {
			const saved = await invoke("clear_bilibili_login");
			setSettings({
				...saved,
				danmakuFontSize: normalizeDanmakuFontSize(saved.danmakuFontSize),
			});
			setMessage("已清除 B站 登录态");
		} catch (err) {
			setError(String(err));
		} finally {
			setSaving(false);
		}
	}

	function isFavorited(streamer) {
		return streamers.some(
			(favorite) =>
				(favorite.platform || "douyu") === (streamer.platform || "douyu") &&
				favorite.target === streamer.target,
		);
	}

	async function addStreamerToFavorites(streamer, nextMessage) {
		if (isFavorited(streamer)) {
			setMessage(`${streamer.name} 已经在收藏列表里了`);
			return;
		}

		const next = [
			...streamers,
			{
				id: crypto.randomUUID(),
				name: streamer.name,
				platform: streamer.platform || "douyu",
				target: streamer.target,
				avatarUrl: streamer.avatarUrl || null,
				isOnline: streamer.isOnline,
				screenshotUrl: streamer.screenshotUrl || null,
				heatText: streamer.heatText || null,
			},
		];

		const saved = await persistStreamers(next, nextMessage);
		if (saved) {
			setStreamers(saved);
		}
	}

	async function handleRefresh() {
		if (streamers.length === 0 || refreshing) {
			return;
		}

		setRefreshing(true);
		setError("");
		try {
			await syncStreamerStatusInBackground(streamers, { showMessage: true });
		} catch (err) {
			setError(String(err));
		} finally {
			setRefreshing(false);
		}
	}

	async function handleSearch(event) {
		event.preventDefault();
		const keyword = searchInput.trim();
		if (!keyword) {
			setError("请输入主播名字");
			return;
		}

		setSearching(true);
		setSearchPerformed(true);
		setOpenMenuId(null);
		setSearchResults([]);
		setError("");
		setMessage("");

		try {
			const settled = await Promise.all(
				searchPlatforms.map((platform) =>
					invoke("search_streamers_by_platform", { platform, keyword })
						.then((results) => {
							setSearchResults((current) => {
								const next = [...current];
								for (const result of results) {
									const exists = next.some(
										(item) =>
											item.platform === result.platform &&
											item.target === result.target,
									);
									if (!exists) {
										next.push(result);
									}
								}
								return next;
							});
							return { ok: true, results };
						})
						.catch((err) => ({ ok: false, error: String(err) })),
				),
			);
			const failed = settled.filter((result) => !result.ok);
			const totalResults = settled.reduce(
				(count, result) => count + (result.ok ? result.results.length : 0),
				0,
			);
			if (failed.length === settled.length) {
				setError(`搜索主播失败：${failed[0]?.error || "全部平台搜索失败"}`);
			} else if (failed.length > 0 && totalResults === 0) {
				setError("部分平台搜索失败，且没有找到匹配主播");
			}
		} catch (err) {
			setError(`搜索主播失败：${String(err)}`);
			setSearchResults([]);
		} finally {
			setSearching(false);
		}
	}

	async function handleRemove(id) {
		setOpenMenuId(null);
		const next = streamers.filter((streamer) => streamer.id !== id);
		await persistStreamers(next, "已从收藏中移除主播");
	}

	async function handlePlay(streamer) {
		setError("");
		setMessage("");
		try {
			const playbackStreamer = {
				id: streamer.id || crypto.randomUUID(),
				name: streamer.name,
				platform: streamer.platform || "douyu",
				target: streamer.target,
				avatarUrl: streamer.avatarUrl || null,
				isOnline: streamer.isOnline ?? false,
				screenshotUrl: streamer.screenshotUrl || null,
				heatText: streamer.heatText || null,
			};
			setCurrentPlaybackStreamer(playbackStreamer);
			setFrontendPlayback({ phase: "loading", playInfo: null, errorMessage: "" });
			enterPlaybackView();
			await new Promise((resolve) => requestAnimationFrame(() => resolve()));
			const playInfo = await invoke("resolve_stream_playback", {
				streamer: playbackStreamer,
				settings,
			});
			setFrontendPlayback({ phase: "loading", playInfo, errorMessage: "" });
		} catch (err) {
			restoreBrowseView();
			setError(String(err));
		}
	}

	const handleFrontendPlayerReady = useCallback(() => {
		playbackAwaitingStartRef.current = false;
		setFrontendPlayback((current) => ({ ...current, phase: "playing", errorMessage: "" }));
	}, []);

	const handleFrontendPlayerError = useCallback((message) => {
		const errorMessage = message || "网页播放器播放失败";
		playbackAwaitingStartRef.current = false;
		setFrontendPlayback((current) => ({ ...current, phase: "error", errorMessage }));
		setError(errorMessage);
		setPlaybackBackVisible(true);
	}, []);

	if (loading) {
		return (
			<main className="shell">
				<section className="loading-view">正在加载应用数据...</section>
			</main>
		);
	}

	if (viewMode === "playback") {
		const playbackTitle =
			frontendPlayback.playInfo?.title || currentPlaybackStreamer?.name || "正在播放";

		return (
			<PlaybackPage
				pageRef={playbackPageRef}
				surfaceRef={playerSurfaceRef}
				controlsVisible={playbackBackVisible}
				fullscreenActive={playbackFullscreen}
				playbackTitle={playbackTitle}
				playbackPositionSeconds={0}
				playbackTrackStartSeconds={0}
				playbackTrackEndSeconds={0}
				playbackTrackMode="live"
				playbackSeekable={false}
				playbackTrackDragging={false}
				danmakuItems={danmakuItems}
				danmakuFontSize={normalizeDanmakuFontSize(settings.danmakuFontSize)}
				onBack={() => {
					void exitPlaybackView();
				}}
				onToggleFullscreen={() => {
					void togglePlaybackFullscreen();
				}}
				onPointerMove={handlePlaybackPointerMove}
				onPointerLeave={handlePlaybackPointerLeave}
				onStageDoubleClick={handlePlaybackStageDoubleClick}
				onTrackPointerDown={() => {}}
				onTrackPointerMove={() => {}}
				onTrackPointerUp={() => {}}
				onTrackPointerCancel={() => {}}
			>
				{frontendPlayback.playInfo ? (
					<FrontendVideoPlayer
						playInfo={frontendPlayback.playInfo}
						onReady={handleFrontendPlayerReady}
						onError={handleFrontendPlayerError}
					/>
				) : null}
				{frontendPlayback.errorMessage ? (
					<div className="frontend-player-error">
						<strong>网页播放器失败</strong>
						<span>{frontendPlayback.errorMessage}</span>
					</div>
				) : null}
			</PlaybackPage>
		);
	}

	const activeTabIndex = Math.max(
		0,
		tabs.findIndex((tab) => tab.id === activeTab),
	);
	const liveStreamers = streamers.filter((streamer) => streamer.isOnline);
	const offlineStreamers = streamers.filter((streamer) => !streamer.isOnline);
	const liveSearchResults = searchResults.filter(
		(streamer) => streamer.isOnline,
	);
	const offlineSearchResults = searchResults.filter(
		(streamer) => !streamer.isOnline,
	);

	return (
		<main className="shell">
			<div className="top-row">
				<nav
					className="top-nav"
					aria-label="主导航"
					style={{ "--active-index": activeTabIndex }}
				>
					<span className="nav-pill-indicator" aria-hidden="true" />
					{tabs.map((tab) => (
						<button
							key={tab.id}
							type="button"
							className={`nav-pill ${activeTab === tab.id ? "active" : ""}`}
							onClick={() => setActiveTab(tab.id)}
						>
							{tab.label}
						</button>
					))}
				</nav>

				<button
					type="button"
					className="refresh-button"
					onClick={handleRefresh}
					disabled={refreshing || streamers.length === 0}
					aria-label={refreshing ? "刷新中" : "刷新"}
					title={refreshing ? "刷新中" : "刷新"}
				>
					<RefreshIcon />
				</button>
			</div>

			{activeTab === "favorites" ? (
				<section className="page-section">
					{streamers.length === 0 ? (
						<div className="empty-board">
							<p>还没有收藏的主播</p>
							<button
								type="button"
								className="ghost-button"
								onClick={() => setActiveTab("search")}
							>
								去搜索页添加
							</button>
						</div>
					) : (
						<div className="favorites-groups">
							<StreamerGroup
								title="已开播"
								streamers={liveStreamers}
								emptyText="当前没有已开播的主播"
								iconUrls={platformIconUrls}
								openMenuId={openMenuId}
								setOpenMenuId={setOpenMenuId}
								onPlay={handlePlay}
								getMenuProps={(streamer) => ({
									menuId: `favorite:${streamer.id}`,
									label: "移除",
									tone: "danger",
									onAction: () => handleRemove(streamer.id),
								})}
							/>

							<StreamerGroup
								title="未开播"
								streamers={offlineStreamers}
								emptyText="当前没有未开播的主播"
								iconUrls={platformIconUrls}
								openMenuId={openMenuId}
								setOpenMenuId={setOpenMenuId}
								onPlay={handlePlay}
								getMenuProps={(streamer) => ({
									menuId: `favorite:${streamer.id}`,
									label: "移除",
									tone: "danger",
									onAction: () => handleRemove(streamer.id),
								})}
							/>
						</div>
					)}
				</section>
			) : activeTab === "search" ? (
				<section className="page-section search-page">
					<form className="search-panel" onSubmit={handleSearch}>
						<label className="search-field">
							<input
								value={searchInput}
								onChange={(event) => setSearchInput(event.target.value)}
								placeholder="输入主播名字 例如：软软甜 / 某幻君"
							/>
						</label>
						<button type="submit" disabled={searching}>
							{searching ? "搜索中..." : "搜索"}
						</button>
					</form>

					{searchResults.length > 0 ? (
						<div className="favorites-groups">
							<StreamerGroup
								title="已开播"
								streamers={liveSearchResults}
								emptyText="没有匹配的已开播主播"
								iconUrls={platformIconUrls}
								openMenuId={openMenuId}
								setOpenMenuId={setOpenMenuId}
								onPlay={handlePlay}
								getMenuProps={(streamer) => ({
									menuId: `search:${streamer.platform}:${streamer.target}`,
									label: isFavorited(streamer) ? "已收藏" : "收藏",
									disabled: isFavorited(streamer),
									onAction: async () => {
										await addStreamerToFavorites(
											streamer,
											`已收藏 ${streamer.name}`,
										);
										setOpenMenuId(null);
									},
								})}
							/>

							<StreamerGroup
								title="未开播"
								streamers={offlineSearchResults}
								emptyText="没有匹配的未开播主播"
								iconUrls={platformIconUrls}
								openMenuId={openMenuId}
								setOpenMenuId={setOpenMenuId}
								onPlay={handlePlay}
								getMenuProps={(streamer) => ({
									menuId: `search:${streamer.platform}:${streamer.target}`,
									label: isFavorited(streamer) ? "已收藏" : "收藏",
									disabled: isFavorited(streamer),
									onAction: async () => {
										await addStreamerToFavorites(
											streamer,
											`已收藏 ${streamer.name}`,
										);
										setOpenMenuId(null);
									},
								})}
							/>
						</div>
					) : searching ? (
						<div className="group-empty">正在搜索主播...</div>
					) : searchPerformed && searchResults.length === 0 ? (
						<div className="group-empty">没有找到匹配的主播</div>
					) : (
						<div className="group-empty">
							输入主播名字后，就会在这里显示搜索结果
						</div>
					)}
				</section>
			) : (
				<section className="page-section settings-page">
					<div className="sub-panel">
						<div className="sub-panel-head">
							<h3>播放器设置</h3>
						</div>
						<div className="player-inline-note">
							<strong>网页播放器</strong>
							<span>使用 video + MSE 播放 HTTP-FLV/HLS。</span>
						</div>
						<div className="player-extra">
							<label className="setting-stack" htmlFor="danmaku-font-size">
								<div className="setting-label-row">
									<span>弹幕字号</span>
									<strong>
										{normalizeDanmakuFontSize(settings.danmakuFontSize)}px
									</strong>
								</div>
								<input
									id="danmaku-font-size"
									type="range"
									min="9"
									max="28"
									step="1"
									value={normalizeDanmakuFontSize(settings.danmakuFontSize)}
									onChange={(event) =>
										setSettings((current) => ({
											...current,
											danmakuFontSize: normalizeDanmakuFontSize(
												event.target.value,
											),
										}))
									}
								/>
							</label>
							<div className="settings-actions">
								<button
									type="button"
									className="ghost-button"
									disabled={saving}
									onClick={() => persistSettings(settings, "弹幕字号已保存")}
								>
									保存字号
								</button>
							</div>
						</div>
						<div className="player-extra">
							<div className="login-status-row compact">
								<span
									className={`login-state ${settings.bilibiliCookie?.includes("SESSDATA=") ? "online" : "offline"}`}
								>
									{settings.bilibiliCookie?.includes("SESSDATA=")
										? "B站 已登录"
										: "B站 未登录"}
								</span>
								<div className="login-actions">
									<button
										type="button"
										className="ghost-button"
										disabled={saving}
										onClick={handleOpenBilibiliLogin}
									>
										打开 B站 登录面板
									</button>
									<button
										type="button"
										className="ghost-button"
										disabled={saving}
										onClick={handleClearBilibiliLogin}
									>
										清除登录态
									</button>
								</div>
							</div>
						</div>
					</div>
				</section>
			)}

			{message ? <div className="notice-strip success">{message}</div> : null}
			{error ? <div className="notice-strip error">{error}</div> : null}
		</main>
	);
}

export default App;
