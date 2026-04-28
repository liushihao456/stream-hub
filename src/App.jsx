import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const tabs = [
  { id: "favorites", label: "收藏" },
  { id: "search", label: "搜索" },
  { id: "settings", label: "设置" },
];

const searchPlatforms = ["douyu", "bilibili_live", "huya", "douyin_live"];
const PLAYBACK_BACK_HIDE_DELAY_MS = 5000;
const emptyEmbeddedPlayerState = {
  phase: "idle",
  title: "",
  streamerName: "",
  platform: "",
  visible: false,
  paused: false,
  muted: false,
  volume: 100,
  usingExternalPlayer: false,
  errorMessage: "",
};

function PlatformIcon({ platform, iconUrls }) {
  const iconMap = {
    douyu: { label: "斗鱼", url: iconUrls.douyu, className: "douyu" },
    bilibili_live: { label: "B站", url: iconUrls.bilibili, className: "bilibili" },
    huya: { label: "虎牙", url: iconUrls.huya, className: "huya" },
    douyin_live: { label: "抖音", url: iconUrls.douyin, className: "douyin" },
  };
  const config = iconMap[platform] || iconMap.douyu;

  return (
    <span className={`platform-icon ${config.className}`} aria-label={config.label} title={config.label}>
      {config.url ? (
        <img className="platform-icon-image" src={config.url} alt="" aria-hidden="true" />
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
      onMouseLeave={event => {
        setOpenMenuId(current => (current === menuId ? null : current));
        if (event.currentTarget.contains(document.activeElement)) {
          document.activeElement?.blur();
        }
      }}
      onClick={() => {
        triggerClickFeedback();
        onPlay(streamer);
      }}
      onKeyDown={event => {
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
        onClick={event => event.stopPropagation()}
        onKeyDown={event => event.stopPropagation()}
      >
        <summary
          className="menu-trigger"
          aria-label={`${streamer.name} 更多操作`}
          title="更多操作"
          onClick={event => {
            event.preventDefault();
            event.stopPropagation();
            setOpenMenuId(current => (current === menuId ? null : menuId));
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
            onClick={event => {
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
          <span className={`heat-dot ${streamer.isOnline ? "live" : "offline"}`} />
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
          {streamers.map(streamer => {
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
  surfaceRef,
  backButtonVisible,
  onBack,
  onPointerMove,
}) {
  return (
    <main className="playback-page" onPointerMove={onPointerMove}>
      <div
        ref={surfaceRef}
        className="playback-stage"
        aria-label="直播画面区域"
      />
      <button
        type="button"
        className={`playback-back-button ${backButtonVisible ? "visible" : ""}`}
        onClick={onBack}
        aria-label="返回"
        title="返回"
      >
        <LeftChevronIcon />
      </button>
    </main>
  );
}

function App() {
  const [streamers, setStreamers] = useState([]);
  const [platformIconUrls, setPlatformIconUrls] = useState({ bilibili: "", douyu: "", huya: "", douyin: "" });
  const [settings, setSettings] = useState({
    player: "iina",
    iinaPath: "",
    mpvPath: "",
    bilibiliCookie: "",
    enableIinaDanmaku: true,
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
  const [embeddedPlayer, setEmbeddedPlayer] = useState(emptyEmbeddedPlayerState);
  const [viewMode, setViewMode] = useState("browse");
  const [playbackBackVisible, setPlaybackBackVisible] = useState(false);
  const playerSurfaceRef = useRef(null);
  const boundsFrameRef = useRef(0);
  const lastBoundsRef = useRef("");
  const boundsRequestInFlightRef = useRef(false);
  const pendingBoundsRef = useRef(null);
  const playbackOriginRef = useRef({ activeTab: "favorites", scrollY: 0 });
  const restoreScrollRef = useRef(null);
  const playbackBackHideTimerRef = useRef(0);
  const playbackExitInFlightRef = useRef(false);
  const playbackAwaitingStartRef = useRef(false);

  function clearPlaybackBackHideTimer() {
    if (playbackBackHideTimerRef.current) {
      window.clearTimeout(playbackBackHideTimerRef.current);
      playbackBackHideTimerRef.current = 0;
    }
  }

  function restoreBrowseView() {
    playbackAwaitingStartRef.current = false;
    clearPlaybackBackHideTimer();
    setPlaybackBackVisible(false);
    setViewMode("browse");
    setActiveTab(playbackOriginRef.current.activeTab);
    restoreScrollRef.current = playbackOriginRef.current.scrollY;
  }

  function revealPlaybackBackButton() {
    if (viewMode !== "playback") {
      return;
    }
    setPlaybackBackVisible(true);
    clearPlaybackBackHideTimer();
    playbackBackHideTimerRef.current = window.setTimeout(() => {
      setPlaybackBackVisible(false);
      playbackBackHideTimerRef.current = 0;
    }, PLAYBACK_BACK_HIDE_DELAY_MS);
  }

  function enterPlaybackView() {
    playbackAwaitingStartRef.current = true;
    playbackOriginRef.current = {
      activeTab,
      scrollY: window.scrollY || window.pageYOffset || 0,
    };
    restoreScrollRef.current = null;
    clearPlaybackBackHideTimer();
    setPlaybackBackVisible(false);
    setViewMode("playback");
    window.scrollTo(0, 0);
  }

  async function exitPlaybackView({ stopPlayback }) {
    if (playbackExitInFlightRef.current) {
      return;
    }
    playbackExitInFlightRef.current = true;

    try {
      if (stopPlayback) {
        const next = await invoke("embedded_player_command", {
          command: { kind: "stop" },
        });
        setEmbeddedPlayer(next);
      }
    } catch (err) {
      setError(String(err));
    } finally {
      restoreBrowseView();
      playbackExitInFlightRef.current = false;
    }
  }

  useEffect(() => {
    let cancelled = false;
    let unlistenUpdated;
    let unlistenError;
    let unlistenEmbeddedState;
    let unlistenEmbeddedError;

    async function bootstrap() {
      try {
        const icons = await invoke("load_platform_icons");
        if (!cancelled) {
          setPlatformIconUrls(icons);
        }

        unlistenUpdated = await listen("bilibili-login-updated", event => {
          setSettings(event.payload);
          setMessage("已更新 B站 登录态");
          setError("");
        });
        unlistenError = await listen("bilibili-login-error", event => {
          setError(String(event.payload));
        });
        unlistenEmbeddedState = await listen("embedded-player-state", event => {
          setEmbeddedPlayer(current => ({ ...current, ...event.payload }));
        });
        unlistenEmbeddedError = await listen("embedded-player-error", event => {
          setError(String(event.payload));
        });

        const [savedStreamers, savedSettings, embeddedState] = await Promise.all([
          invoke("load_streamers"),
          invoke("load_settings"),
          invoke("embedded_player_get_state"),
        ]);

        if (!cancelled) {
          setSettings(savedSettings);
          setEmbeddedPlayer(embeddedState);
          if (savedStreamers.length > 0) {
            const syncedStreamers = await invoke("sync_streamers_status", { streamers: savedStreamers });
            if (!cancelled) {
              setStreamers(syncedStreamers);
            }
          } else {
            setStreamers(savedStreamers);
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
      unlistenEmbeddedState?.();
      unlistenEmbeddedError?.();
    };
  }, []);

  useEffect(() => {
    return () => {
      clearPlaybackBackHideTimer();
    };
  }, []);

  useEffect(() => {
    if (settings.player === "libmpv") {
      return;
    }

    if (viewMode === "playback") {
      restoreBrowseView();
    }

    if (!embeddedPlayer.visible && embeddedPlayer.phase === "idle") {
      return;
    }

    invoke("embedded_player_command", {
      command: { kind: "stop" },
    }).catch(() => {});
  }, [settings.player, embeddedPlayer.phase, embeddedPlayer.visible, viewMode]);

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

    function handleKeyDown(event) {
      if (event.key !== "Escape") {
        return;
      }
      event.preventDefault();
      void exitPlaybackView({ stopPlayback: true });
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [viewMode, embeddedPlayer.visible, embeddedPlayer.phase]);

  useEffect(() => {
    if (viewMode !== "playback") {
      playbackAwaitingStartRef.current = false;
      return;
    }

    if (embeddedPlayer.visible || embeddedPlayer.phase !== "idle") {
      playbackAwaitingStartRef.current = false;
    }
  }, [viewMode, embeddedPlayer.visible, embeddedPlayer.phase]);

  useEffect(() => {
    if (viewMode !== "playback" || playbackExitInFlightRef.current) {
      return;
    }

    if (embeddedPlayer.errorMessage) {
      void exitPlaybackView({ stopPlayback: embeddedPlayer.visible || embeddedPlayer.phase !== "idle" });
      return;
    }

    if (embeddedPlayer.phase === "ended") {
      void exitPlaybackView({ stopPlayback: embeddedPlayer.visible || embeddedPlayer.phase !== "idle" });
      return;
    }

    if (embeddedPlayer.phase === "idle" && !embeddedPlayer.visible) {
      if (playbackAwaitingStartRef.current) {
        return;
      }
      restoreBrowseView();
    }
  }, [viewMode, embeddedPlayer.phase, embeddedPlayer.visible, embeddedPlayer.errorMessage]);

  useEffect(() => {
    const element = playerSurfaceRef.current;
    const shouldTrack = settings.player === "libmpv" && viewMode === "playback";
    if (!element || !shouldTrack) {
      return undefined;
    }

    function pushBounds(bounds, signature) {
      boundsRequestInFlightRef.current = true;
      invoke("embedded_player_set_bounds", { bounds })
        .catch(err => {
          setError(String(err));
        })
        .finally(() => {
          boundsRequestInFlightRef.current = false;
          const pending = pendingBoundsRef.current;
          pendingBoundsRef.current = null;
          if (pending && pending.signature !== signature) {
            pushBounds(pending.bounds, pending.signature);
          }
        });
    }

    function scheduleBoundsSync() {
      if (boundsFrameRef.current) {
        return;
      }
      boundsFrameRef.current = window.requestAnimationFrame(() => {
        boundsFrameRef.current = 0;
        const target = playerSurfaceRef.current;
        if (!target) {
          return;
        }
        const rect = target.getBoundingClientRect();
        const scaleFactor = window.devicePixelRatio || 1;
        const bounds = {
          x: Math.round(rect.left * scaleFactor),
          y: Math.round(rect.top * scaleFactor),
          width: Math.max(1, Math.round(rect.width * scaleFactor)),
          height: Math.max(1, Math.round(rect.height * scaleFactor)),
          scaleFactor,
          viewportHeight: Math.max(1, Math.round(window.innerHeight * scaleFactor)),
        };
        const signature = `${bounds.x}:${bounds.y}:${bounds.width}:${bounds.height}:${bounds.scaleFactor}:${bounds.viewportHeight}`;
        if (signature === lastBoundsRef.current) {
          return;
        }
        lastBoundsRef.current = signature;
        if (boundsRequestInFlightRef.current) {
          pendingBoundsRef.current = { bounds, signature };
          return;
        }
        pushBounds(bounds, signature);
      });
    }

    const observer = new ResizeObserver(() => {
      scheduleBoundsSync();
    });
    observer.observe(element);
    window.addEventListener("resize", scheduleBoundsSync);
    scheduleBoundsSync();

    return () => {
      observer.disconnect();
      window.removeEventListener("resize", scheduleBoundsSync);
      if (boundsFrameRef.current) {
        window.cancelAnimationFrame(boundsFrameRef.current);
        boundsFrameRef.current = 0;
      }
      pendingBoundsRef.current = null;
      boundsRequestInFlightRef.current = false;
      lastBoundsRef.current = "";
    };
  }, [settings.player, viewMode]);

  async function persistStreamers(nextStreamers, nextMessage = "主播列表已保存") {
    setSaving(true);
    setError("");
    setMessage("");
    try {
      const saved = await invoke("save_streamers", { streamers: nextStreamers });
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

  async function persistSettings(nextSettings) {
    setSaving(true);
    setError("");
    setMessage("");
    try {
      const saved = await invoke("save_settings", { settings: nextSettings });
      setSettings(saved);
      setMessage("播放器设置已保存");
    } catch (err) {
      setError(String(err));
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
      setSettings(saved);
      setMessage("已清除 B站 登录态");
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }

  function isFavorited(streamer) {
    return streamers.some(
      favorite =>
        (favorite.platform || "douyu") === (streamer.platform || "douyu")
        && favorite.target === streamer.target,
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
      const syncedStreamers = await invoke("sync_streamers_status", { streamers });
      setStreamers(syncedStreamers);
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
        searchPlatforms.map(platform =>
          invoke("search_streamers_by_platform", { platform, keyword })
            .then(results => {
              setSearchResults(current => {
                const next = [...current];
                for (const result of results) {
                  const exists = next.some(
                    item => item.platform === result.platform && item.target === result.target,
                  );
                  if (!exists) {
                    next.push(result);
                  }
                }
                return next;
              });
              return { ok: true, results };
            })
            .catch(err => ({ ok: false, error: String(err) })),
        ),
      );
      const failed = settled.filter(result => !result.ok);
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
    const next = streamers.filter(streamer => streamer.id !== id);
    await persistStreamers(next, "已从收藏中移除主播");
  }

  async function handlePlay(streamer) {
    setError("");
    setMessage("");
    try {
      if (settings.player === "libmpv") {
        enterPlaybackView();
      }
      await new Promise(resolve => requestAnimationFrame(() => resolve()));
      await invoke("play_streamer", {
        streamer: {
          id: streamer.id || crypto.randomUUID(),
          name: streamer.name,
          platform: streamer.platform || "douyu",
          target: streamer.target,
          avatarUrl: streamer.avatarUrl || null,
          isOnline: streamer.isOnline ?? false,
          screenshotUrl: streamer.screenshotUrl || null,
          heatText: streamer.heatText || null,
        },
        settings,
      });
    } catch (err) {
      if (settings.player === "libmpv") {
        restoreBrowseView();
      }
      setError(String(err));
    }
  }

  if (loading) {
    return (
      <main className="shell">
        <section className="loading-view">正在加载应用数据...</section>
      </main>
    );
  }

  if (viewMode === "playback") {
    return (
      <PlaybackPage
        surfaceRef={playerSurfaceRef}
        backButtonVisible={playbackBackVisible}
        onBack={() => {
          void exitPlaybackView({ stopPlayback: true });
        }}
        onPointerMove={revealPlaybackBackButton}
      />
    );
  }

  const activeTabIndex = Math.max(
    0,
    tabs.findIndex(tab => tab.id === activeTab),
  );
  const liveStreamers = streamers.filter(streamer => streamer.isOnline);
  const offlineStreamers = streamers.filter(streamer => !streamer.isOnline);
  const liveSearchResults = searchResults.filter(streamer => streamer.isOnline);
  const offlineSearchResults = searchResults.filter(streamer => !streamer.isOnline);

  return (
    <main className="shell">
      <div className="top-row">
        <nav
          className="top-nav"
          aria-label="主导航"
          style={{ "--active-index": activeTabIndex }}
        >
          <span className="nav-pill-indicator" aria-hidden="true" />
          {tabs.map(tab => (
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
              <button type="button" className="ghost-button" onClick={() => setActiveTab("search")}>
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
                getMenuProps={streamer => ({
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
                getMenuProps={streamer => ({
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
                onChange={event => setSearchInput(event.target.value)}
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
                getMenuProps={streamer => ({
                  menuId: `search:${streamer.platform}:${streamer.target}`,
                  label: isFavorited(streamer) ? "已收藏" : "收藏",
                  disabled: isFavorited(streamer),
                  onAction: async () => {
                    await addStreamerToFavorites(streamer, `已收藏 ${streamer.name}`);
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
                getMenuProps={streamer => ({
                  menuId: `search:${streamer.platform}:${streamer.target}`,
                  label: isFavorited(streamer) ? "已收藏" : "收藏",
                  disabled: isFavorited(streamer),
                  onAction: async () => {
                    await addStreamerToFavorites(streamer, `已收藏 ${streamer.name}`);
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
            <div className="group-empty">输入主播名字后，就会在这里显示搜索结果</div>
          )}
        </section>
      ) : (
        <section className="page-section settings-page">
          <div className="sub-panel">
            <div className="sub-panel-head">
              <h3>播放器设置</h3>
            </div>
            <div className="player-picker" role="tablist" aria-label="播放器类型">
              <button
                type="button"
                className={`player-picker-button ${settings.player === "iina" ? "active" : ""}`}
                onClick={() => setSettings(current => ({ ...current, player: "iina" }))}
              >
                IINA
              </button>
              <button
                type="button"
                className={`player-picker-button ${settings.player === "mpv" ? "active" : ""}`}
                onClick={() => setSettings(current => ({ ...current, player: "mpv" }))}
              >
                mpv
              </button>
              <button
                type="button"
                className={`player-picker-button ${settings.player === "libmpv" ? "active" : ""}`}
                onClick={() => setSettings(current => ({ ...current, player: "libmpv" }))}
              >
                libmpv
              </button>
            </div>
            <div className={`player-settings ${settings.player === "libmpv" ? "single-action" : ""}`}>
              {settings.player === "libmpv" ? (
                <div className="player-inline-note">
                  <strong>主窗口内嵌播放已启用。</strong>
                  <span>该模式依赖系统已安装 `libmpv`，首版暂不启用弹幕。</span>
                </div>
              ) : (
                <label className="search-field">
                  <input
                    value={settings.player === "iina" ? settings.iinaPath : settings.mpvPath}
                    onChange={event =>
                      setSettings(current => (
                        settings.player === "iina"
                          ? { ...current, iinaPath: event.target.value }
                          : { ...current, mpvPath: event.target.value }
                      ))
                    }
                    placeholder={
                      settings.player === "iina"
                        ? "IINA.app 或 iina-cli 路径 留空则自动查找"
                        : "mpv 路径 留空则使用系统 PATH 中的 mpv"
                    }
                  />
                </label>
              )}
              <button type="button" className="ghost-button" disabled={saving} onClick={() => persistSettings(settings)}>
                保存
              </button>
            </div>
            <div className="player-extra">
              <div className="login-status-row compact">
                <span className={`login-state ${settings.bilibiliCookie?.includes("SESSDATA=") ? "online" : "offline"}`}>
                  {settings.bilibiliCookie?.includes("SESSDATA=") ? "B站 已登录" : "B站 未登录"}
                </span>
                <div className="login-actions">
                  <button type="button" className="ghost-button" disabled={saving} onClick={handleOpenBilibiliLogin}>
                    打开 B站 登录面板
                  </button>
                  <button type="button" className="ghost-button" disabled={saving} onClick={handleClearBilibiliLogin}>
                    清除登录态
                  </button>
                </div>
              </div>
            </div>
            {settings.player === "iina" ? (
              <div className="player-extra">
                <label className="toggle-row">
                  <input
                    type="checkbox"
                    checked={settings.enableIinaDanmaku}
                    onChange={event =>
                      setSettings(current => ({ ...current, enableIinaDanmaku: event.target.checked }))
                    }
                  />
                  <span>启用 IINA 弹幕</span>
                </label>
                <p className="settings-hint">
                  当前 IINA 弹幕支持斗鱼、B站、虎牙、抖音。
                </p>
                <p className="settings-hint">
                  B站高画质通常需要登录态。可以直接用上面的登录面板完成登录。
                </p>
              </div>
            ) : settings.player === "libmpv" ? (
              <div className="player-extra">
                <p className="settings-hint">
                  libmpv 模式会把画面直接嵌入主窗口，并沿用当前直播提流与备用线路逻辑。
                </p>
                <p className="settings-hint">
                  如果系统缺少 `libmpv` 或播放异常，可以随时切回 IINA / mpv 外部模式。
                </p>
              </div>
            ) : (
              <p className="settings-hint">mpv 模式当前不启用弹幕。B站高画质同样可以使用上面的登录面板。</p>
            )}
          </div>
        </section>
      )}

      {message ? <div className="notice-strip success">{message}</div> : null}
      {error ? <div className="notice-strip error">{error}</div> : null}

    </main>
  );
}

export default App;
