import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

const tabs = [
  { id: "favorites", label: "收藏" },
  { id: "search", label: "搜索" },
  { id: "settings", label: "设置" },
];

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

function getImageForStreamer(streamer) {
  if (streamer.isOnline && streamer.screenshotUrl) {
    return streamer.screenshotUrl;
  }
  return streamer.avatarUrl || streamer.screenshotUrl || "";
}

function StreamerCard({
  streamer,
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

  return (
    <article
      className="favorite-card"
      onMouseLeave={event => {
        setOpenMenuId(current => (current === menuId ? null : current));
        if (event.currentTarget.contains(document.activeElement)) {
          document.activeElement?.blur();
        }
      }}
      onClick={() => onPlay(streamer)}
      onKeyDown={event => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
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

function App() {
  const [streamers, setStreamers] = useState([]);
  const [settings, setSettings] = useState({
    player: "iina",
    iinaPath: "",
    mpvPath: "",
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
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    let cancelled = false;

    async function bootstrap() {
      try {
        const [savedStreamers, savedSettings] = await Promise.all([
          invoke("load_streamers"),
          invoke("load_settings"),
        ]);

        if (!cancelled) {
          setSettings(savedSettings);
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
    };
  }, []);

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

  function isFavorited(streamer) {
    return streamers.some(
      favorite => favorite.target === streamer.target || favorite.name === streamer.name,
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
    if (streamers.length === 0) {
      return;
    }

    setSaving(true);
    setError("");
    try {
      const syncedStreamers = await invoke("sync_streamers_status", { streamers });
      setStreamers(syncedStreamers);
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
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
    setError("");
    setMessage("");

    try {
      const results = await invoke("search_streamers", { keyword });
      setSearchResults(results);
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
      await invoke("play_streamer", {
        streamer: {
          id: streamer.id || crypto.randomUUID(),
          name: streamer.name,
          target: streamer.target,
          avatarUrl: streamer.avatarUrl || null,
          isOnline: streamer.isOnline ?? false,
          screenshotUrl: streamer.screenshotUrl || null,
          heatText: streamer.heatText || null,
        },
        settings,
      });
    } catch (err) {
      setError(String(err));
    }
  }

  async function handleInstallIinaPlugin() {
    setSaving(true);
    setError("");
    setMessage("");
    try {
      const result = await invoke("install_iina_danmaku_plugin");
      setMessage(result);
    } catch (err) {
      setError(String(err));
    } finally {
      setSaving(false);
    }
  }

  if (loading) {
    return (
      <main className="shell">
        <section className="loading-view">正在加载应用数据...</section>
      </main>
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
          disabled={saving || streamers.length === 0}
          aria-label="刷新"
          title="刷新"
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
                placeholder="输入主播名字 例如：软软甜"
              />
            </label>
            <button type="submit" disabled={searching}>
              {searching ? "搜索中..." : "搜索"}
            </button>
          </form>

          {searching ? (
            <div className="group-empty">正在搜索主播...</div>
          ) : searchPerformed && searchResults.length === 0 ? (
            <div className="group-empty">没有找到匹配的主播</div>
          ) : searchResults.length > 0 ? (
            <div className="favorites-groups">
              <StreamerGroup
                title="已开播"
                streamers={liveSearchResults}
                emptyText="没有匹配的已开播主播"
                openMenuId={openMenuId}
                setOpenMenuId={setOpenMenuId}
                onPlay={handlePlay}
                getMenuProps={streamer => ({
                  menuId: `search:${streamer.target}`,
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
                openMenuId={openMenuId}
                setOpenMenuId={setOpenMenuId}
                onPlay={handlePlay}
                getMenuProps={streamer => ({
                  menuId: `search:${streamer.target}`,
                  label: isFavorited(streamer) ? "已收藏" : "收藏",
                  disabled: isFavorited(streamer),
                  onAction: async () => {
                    await addStreamerToFavorites(streamer, `已收藏 ${streamer.name}`);
                    setOpenMenuId(null);
                  },
                })}
              />
            </div>
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
            </div>
            <div className="player-settings">
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
              <button type="button" className="ghost-button" disabled={saving} onClick={() => persistSettings(settings)}>
                保存
              </button>
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
                <div className="overlay-actions">
                  <button type="button" className="ghost-button" disabled={saving} onClick={handleInstallIinaPlugin}>
                    安装或更新 IINA 弹幕插件
                  </button>
                </div>
                <p className="settings-hint">
                  选择 IINA 时，播放前会自动安装并更新内置弹幕插件。mpv 模式只播放直播，不显示弹幕。
                </p>
              </div>
            ) : (
              <p className="settings-hint">mpv 模式当前不启用弹幕。</p>
            )}
          </div>
        </section>
      )}

    </main>
  );
}

export default App;
