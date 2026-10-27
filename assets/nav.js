/* OnePage navigation app */
(() => {
  "use strict";

  const root = document.getElementById("nav-root");
  if (!root) return;

  let boot;
  try {
    boot = JSON.parse(root.dataset.bootstrap || "{}");
  } catch (error) {
    root.innerHTML = '<div class="noscript-card"><h1>导航加载失败</h1><p>页面数据不完整，请刷新后重试。</p></div>';
    return;
  }

  const ICONS = "/assets/icons.svg";
  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const isMac = /Mac|iPhone|iPad/.test(navigator.platform || "");

  const state = {
    filter: "all",
    categories: Array.isArray(boot.categories) ? boot.categories.map((category) => ({ ...category })) : [],
    links: Array.isArray(boot.links) ? boot.links.map((link) => ({ ...link })) : [],
    searchFilter: "all",
    searchResults: [],
    selectedIndex: 0,
    accountOpen: false,
    editingId: null,
    iconRefreshing: new Set(),
    confirmCallback: null,
  };

  const $ = (selector, scope = document) => scope.querySelector(selector);
  const $$ = (selector, scope = document) => Array.from(scope.querySelectorAll(selector));

  function esc(value) {
    return String(value ?? "").replace(/[&<>"']/g, (char) => ({
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      '"': "&quot;",
      "'": "&#39;",
    }[char]));
  }

  function icon(name, extra = "") {
    return `<svg class="icon ${extra}" aria-hidden="true"><use href="${ICONS}?v=${encodeURIComponent(boot.version || "")}#i-${name}"></use></svg>`;
  }

  function initialOf(text) {
    const value = String(text || "").trim();
    return Array.from(value)[0]?.toUpperCase() || "·";
  }

  function hostOf(url) {
    try {
      return new URL(url).hostname.replace(/^www\./, "");
    } catch (_) {
      return String(url || "").replace(/^https?:\/\//, "").split("/")[0];
    }
  }

  function nameMap() {
    const map = new Map();
    state.categories.forEach((category) => map.set(category.id, category.name));
    return map;
  }

  function linksFor(scope) {
    return state.links.filter((link) => link.scope === scope);
  }

  function categoriesFor(scope) {
    return state.categories.filter((category) => category.scope === scope);
  }

  function countForCategory(categoryId, scope) {
    return state.links.filter((link) => link.scope === scope && link.categoryId === categoryId).length;
  }

  function roleLabel() {
    if (boot.role === "super") return "系统管理员";
    if (boot.role === "admin") return "空间管理员";
    return "成员";
  }

  function dateMarkup() {
    const now = new Date();
    const weekdays = ["星期日", "星期一", "星期二", "星期三", "星期四", "星期五", "星期六"];
    return `${now.getMonth() + 1} 月 ${now.getDate()} 日<span class="topbar-weekday"> · ${weekdays[now.getDay()]}</span>`;
  }

  function iconBlock(link, kind = "card") {
    const wrapper = kind === "result" ? "result-icon" : "card-icon";
    const tone = `tone-${Math.abs(Number(link.id) || 0) % 6}`;
    const fallback = `<span class="site-icon-fallback ${tone}" ${link.iconUrl ? "hidden" : ""}>${esc(initialOf(link.title))}</span>`;
    const image = link.iconUrl
      ? `<img src="${esc(link.iconUrl)}" alt="" loading="lazy" data-icon-img>`
      : "";
    return `<span class="${wrapper}" data-icon-for="${esc(link.id)}">${image}${fallback}</span>`;
  }

  function bindIconFallbacks(scope = document) {
    $$("img[data-icon-img]", scope).forEach((img) => {
      if (img.dataset.bound) return;
      img.dataset.bound = "1";
      img.addEventListener("error", () => {
        const fallback = img.parentElement?.querySelector(".site-icon-fallback");
        if (fallback) fallback.hidden = false;
        img.remove();
      }, { once: true });
    });
  }

  function shellMarkup() {
    return `
      <div class="nav-app">
        <header class="nav-topbar">
          <div class="nav-topbar-inner">
            <div class="topbar-brand">
              <a class="brand" href="${esc(boot.urls.public)}" aria-label="${esc(boot.siteName)}">
                <span class="brand-mark" aria-hidden="true"><span class="brand-orbit"></span><span class="brand-core"></span></span>
                <span class="brand-word">${esc(boot.siteName)}</span>
              </a>
              <span class="topbar-divider" aria-hidden="true"></span>
              <span class="topbar-space" title="${esc(boot.space.name)}">${esc(boot.space.name)}</span>
              <span class="topbar-date">${dateMarkup()}</span>
            </div>
            <div class="topbar-actions">
              ${boot.isMember ? `
              <div class="view-switch" role="group" aria-label="筛选导航">
                <button type="button" data-nav-filter="all">全部</button>
                <button type="button" data-nav-filter="public">公共</button>
                <button type="button" data-nav-filter="mine">我的</button>
              </div>` : ""}
              <button type="button" class="topbar-search" data-search-open aria-label="搜索导航">
                ${icon("search")}
                <span class="topbar-search-label">搜索导航</span>
                <kbd>${isMac ? "⌘" : "Ctrl"} K</kbd>
              </button>
              ${boot.isMember ? `<button type="button" class="topbar-add" data-topbar-add data-add-link="" aria-label="添加链接" title="添加链接">${icon("plus")}<span>添加链接</span></button>` : ""}
            </div>
          </div>
        </header>
        <main class="nav-main" id="nav-main"></main>
        <div class="account-dock">
          <button type="button" class="account-avatar" data-account-toggle aria-expanded="${state.accountOpen}" aria-label="打开设置" title="设置">
            <span class="avatar">${esc(boot.user.initials)}</span>
            <span class="account-gear" aria-hidden="true">${icon("settings")}</span>
          </button>
          <div class="settings-panel" id="settings-panel" ${state.accountOpen ? "" : "hidden"}></div>
        </div>
      </div>

      <div class="command-overlay" id="command-overlay" hidden>
        <div class="command-panel" role="dialog" aria-modal="true" aria-label="搜索导航">
          <div class="command-head">
            ${icon("search")}
            <input class="command-input" id="command-input" type="text" placeholder="搜索工具、平台或网址" autocomplete="off" spellcheck="false" aria-label="搜索导航">
            <button type="button" class="command-close" data-search-close>Esc</button>
          </div>
          <div class="command-filters" id="command-filters"></div>
          <div class="command-results" id="command-results"></div>
          <div class="command-foot">
            <span><kbd>↑</kbd><span class="sep">/</span><kbd>↓</kbd> 选择</span>
            <span><kbd>↵</kbd> 打开</span>
            <span class="foot-end"><kbd>esc</kbd> 关闭</span>
          </div>
        </div>
      </div>

      <dialog class="modal" id="link-dialog">
        <div class="modal-head">
          <div>
            <p class="eyebrow">我的导航</p>
            <h2 id="link-dialog-title">添加链接</h2>
          </div>
          <button type="button" class="icon-btn" data-dialog-close aria-label="关闭">${icon("close")}</button>
        </div>
        <form class="modal-form" id="link-form" novalidate>
          <div class="form-alert" id="link-form-alert" role="alert" hidden></div>
          <label class="field">
            <span class="field-label">名称</span>
            <input class="input" type="text" name="title" maxlength="80" required placeholder="例如 数据质量算子仓库">
          </label>
          <label class="field">
            <span class="field-label">网址</span>
            <input class="input" type="text" name="url" maxlength="2048" required spellcheck="false" placeholder="git.example.com/data/quality">
            <span class="field-note">可省略 https://，保存时会自动补全。</span>
          </label>
          <label class="field">
            <span class="field-label">分类</span>
            <select class="input select" name="category_id" id="link-category-select"></select>
          </label>
          <label class="field">
            <span class="field-label">新建分类</span>
            <input class="input" type="text" name="new_category" maxlength="24" placeholder="可选，例如 数据质量">
            <span class="field-note">填写后会优先使用这个新分类。</span>
          </label>
          <label class="field">
            <span class="field-label">描述</span>
            <textarea class="input textarea" name="description" maxlength="280" rows="3" placeholder="一句话说明这个入口的用途"></textarea>
          </label>
          <div class="modal-actions">
            <button type="button" class="btn btn-ghost" data-dialog-close>取消</button>
            <button type="submit" class="btn btn-primary" id="link-save-button">保存链接</button>
          </div>
        </form>
      </dialog>

      <dialog class="modal category-manager" id="category-dialog">
        <div class="modal-head">
          <div>
            <p class="eyebrow">我的导航</p>
            <h2>管理分类</h2>
          </div>
          <button type="button" class="icon-btn" data-dialog-close aria-label="关闭">${icon("close")}</button>
        </div>
        <div class="modal-form">
          <div id="category-manager-body"></div>
          <form class="category-add-row" id="category-add-form">
            <input class="input" type="text" name="name" maxlength="24" placeholder="新分类名称" aria-label="新分类名称" required>
            <button class="btn btn-secondary" type="submit">${icon("plus")}<span>添加</span></button>
          </form>
        </div>
      </dialog>

      <dialog class="modal confirm-modal" id="confirm-dialog">
        <div class="modal-head">
          <div>
            <p class="eyebrow">请确认</p>
            <h2 id="confirm-title">确认操作</h2>
          </div>
          <button type="button" class="icon-btn" data-dialog-close aria-label="关闭">${icon("close")}</button>
        </div>
        <div class="modal-form">
          <p class="confirm-copy" id="confirm-copy"></p>
          <div class="modal-actions">
            <button type="button" class="btn btn-ghost" data-dialog-close>取消</button>
            <button type="button" class="btn btn-danger" id="confirm-action">确认</button>
          </div>
        </div>
      </dialog>

      <div class="toast-region" id="toast-region" aria-live="polite"></div>
    `;
  }

  function spaceRole(space) {
    if (!space.member) return "系统管理员";
    return space.role === "admin" ? "空间管理员" : "成员";
  }

  function renderTopbar() {
    $$("[data-nav-filter]").forEach((button) => {
      const active = button.dataset.navFilter === state.filter;
      button.classList.toggle("is-active", active);
      button.setAttribute("aria-pressed", String(active));
    });
    const addButton = $("[data-topbar-add]");
    if (addButton) addButton.hidden = state.filter === "public";
    const shortcut = $(".topbar-search kbd");
    if (shortcut) shortcut.textContent = `${isMac ? "⌘" : "Ctrl"} K`;
  }

  function renderSettingsPanel() {
    const panel = $("#settings-panel");
    if (!panel) return;
    const otherSpaces = boot.spaces.filter((space) => !space.current);
    panel.innerHTML = `
      <div class="settings-head">
        <span class="avatar">${esc(boot.user.initials)}</span>
        <span class="settings-identity">
          <strong>${esc(boot.user.name)}</strong>
          ${boot.user.name !== boot.user.username ? `<span>@${esc(boot.user.username)}</span>` : ""}
        </span>
        <button type="button" class="icon-btn settings-close" data-settings-close aria-label="关闭设置">${icon("close")}</button>
      </div>

      <div class="settings-space">
        <span class="settings-label">当前空间</span>
        <div class="settings-current-space">
          <span class="space-emblem">${esc(initialOf(boot.space.name))}</span>
          <span class="settings-space-copy">
            <strong>${esc(boot.space.name)}</strong>
            <span>${esc(roleLabel())}</span>
          </span>
        </div>
        ${otherSpaces.length ? `
        <span class="settings-label settings-label-spaced">切换空间</span>
        <div class="space-options">
          ${otherSpaces.map((space) => `
            <button type="button" class="space-option" data-space="${esc(space.slug)}">
              <span class="space-emblem">${esc(initialOf(space.name))}</span>
              <span class="space-option-copy">
                <strong>${esc(space.name)}</strong>
                <span>${esc(spaceRole(space))}</span>
              </span>
              ${icon("arrow-right")}
            </button>
          `).join("")}
        </div>` : ""}
      </div>

      <div class="settings-actions">
        ${boot.isAdmin ? `<a class="settings-action" href="${esc(boot.urls.admin)}">${icon("layers")}<span>管理公共导航</span></a>` : ""}
        ${boot.isMember ? `<button type="button" class="settings-action" data-manage-categories>${icon("edit")}<span>管理个人分类</span></button>` : ""}
        <a class="settings-action" href="${esc(boot.urls.password)}">${icon("key")}<span>修改密码</span></a>
        ${boot.user.isSuperAdmin ? `<a class="settings-action" href="${esc(boot.urls.globalAdmin)}">${icon("shield")}<span>系统管理</span></a>` : ""}
        <button type="button" class="settings-action" data-theme-toggle>${icon("sun")}<span>切换主题</span></button>
        <div class="settings-sep"></div>
        <form method="post" action="${esc(boot.urls.logout)}">
          <input type="hidden" name="csrf" value="${esc(boot.csrf)}">
          <button class="settings-action is-danger" type="submit">${icon("logout")}<span>退出登录</span></button>
        </form>
      </div>
    `;
  }

  function setSettings(open) {
    state.accountOpen = Boolean(open);
    const panel = $("#settings-panel");
    if (panel) panel.hidden = !state.accountOpen;
    $("[data-account-toggle]")?.setAttribute("aria-expanded", String(state.accountOpen));
  }

  function closeSettings() {
    setSettings(false);
  }

  function sectionMarkup({ key, title, links, categoryId, scope, showScopeTag }) {
    const isPersonal = scope === "mine";
    const add = isPersonal
      ? `<button type="button" class="section-action" data-add-link="${categoryId ?? ""}">${icon("plus")}<span>添加</span></button>`
      : "";
    const cards = links.map((link, index) => cardMarkup(link, index, isPersonal)).join("");
    const emptyAdd = isPersonal && links.length === 0
      ? `<button type="button" class="card-add" data-add-link="${categoryId ?? ""}">${icon("plus")}<span>添加链接</span></button>`
      : "";
    const scopeTag = showScopeTag && isPersonal ? `<span class="section-scope">我的</span>` : "";
    return `
      <section class="link-section" id="section-${key}" data-category="${key}" data-scope="${scope}">
        <header class="section-head">
          <div class="section-title"><h2>${esc(title)}</h2>${scopeTag}</div>
          <div class="section-actions">${add}</div>
        </header>
        <div class="link-grid">
          ${cards}${emptyAdd}
        </div>
      </section>
    `;
  }

  function cardMarkup(link, index, isPersonal) {
    const actions = isPersonal ? `
      <div class="card-actions">
        <button type="button" class="icon-btn" data-edit-link="${esc(link.id)}" aria-label="编辑 ${esc(link.title)}" title="编辑">${icon("edit")}</button>
        <button type="button" class="icon-btn icon-btn-danger" data-delete-link="${esc(link.id)}" aria-label="删除 ${esc(link.title)}" title="删除">${icon("trash")}</button>
      </div>` : "";
    const description = link.description
      ? `<p class="card-desc">${esc(link.description)}</p>`
      : "";
    return `
      <article class="link-card" data-link-id="${esc(link.id)}" style="--i:${Math.min(index, 9)}">
        ${actions}
        <a class="card-body" href="/go/${esc(link.id)}" target="_blank" rel="noopener noreferrer" referrerpolicy="no-referrer">
          <div class="card-main">
            ${iconBlock(link)}
            <div class="card-copy">
              <h3>${esc(link.title)}</h3>
              ${description}
            </div>
          </div>
          <div class="card-foot">
            <span class="card-host">${esc(hostOf(link.url))}</span>
            <span class="card-open">${icon("arrow-up-right")}</span>
          </div>
        </a>
      </article>
    `;
  }

  function emptyState() {
    if (boot.isMember && state.filter !== "public") {
      return `
        <div class="empty-state">
          <div class="empty-orbit" aria-hidden="true"><span></span></div>
          <h2>把常用入口留在身边</h2>
          <p>这里只属于你，其他成员看不到。先添加第一个链接，或者先建一个分类。</p>
          <div class="button-row">
            <button type="button" class="btn btn-primary" data-add-link="">${icon("plus")}<span>添加第一个链接</span></button>
            <button type="button" class="btn btn-secondary" data-manage-categories>${icon("layers")}<span>管理分类</span></button>
          </div>
        </div>`;
    }
    const action = boot.isAdmin
      ? `<a class="btn btn-primary" href="${esc(boot.urls.admin)}">${icon("plus")}<span>去后台添加</span></a>`
      : "";
    return `
      <div class="empty-state">
        <div class="empty-orbit" aria-hidden="true"><span></span></div>
        <h2>这里还很安静</h2>
        <p>公共导航还没有入口。空间管理员可以把团队平台、工具和仓库收进来。</p>
        ${action}
      </div>`;
  }

  function renderMain() {
    const main = $("#nav-main");
    if (!main) return;
    const scopes = boot.isMember
      ? (state.filter === "all" ? ["public", "mine"] : [state.filter])
      : ["public"];
    const showScopeTag = scopes.length > 1;
    const sections = [];
    const railItems = [];

    scopes.forEach((scope) => {
      const scopeLinks = linksFor(scope);
      const categories = categoriesFor(scope);
      const grouped = new Map();
      scopeLinks.forEach((link) => {
        const key = link.categoryId ?? "unfiled";
        if (!grouped.has(key)) grouped.set(key, []);
        grouped.get(key).push(link);
      });
      const includeEmpty = scope === "mine" && state.filter === "mine";
      const ordered = includeEmpty
        ? categories
        : categories.filter((category) => (grouped.get(category.id) || []).length > 0);
      ordered.forEach((category) => {
        const links = grouped.get(category.id) || [];
        const key = `${scope}-${category.id}`;
        sections.push(sectionMarkup({
          key,
          title: category.name,
          links,
          categoryId: category.id,
          scope,
          showScopeTag,
        }));
        railItems.push({ key, title: category.name, scope });
      });
      const unfiled = grouped.get("unfiled") || [];
      if (unfiled.length) {
        const key = `${scope}-unfiled`;
        sections.push(sectionMarkup({
          key,
          title: "未分类",
          links: unfiled,
          categoryId: null,
          scope,
          showScopeTag,
        }));
        railItems.push({ key, title: "未分类", scope });
      }
    });

    const hasContent = sections.length > 0;
    if (!hasContent) {
      sections.push(emptyState());
    }

    const rail = hasContent ? `
      <aside class="section-rail" aria-label="板块快速定位">
        <button type="button" class="rail-item is-active" data-category-jump="all">
          <span>全部</span>
        </button>
        ${railItems.map((item) => `
          <button type="button" class="rail-item" data-category-jump="${esc(item.key)}" data-scope="${esc(item.scope)}">
            <span>${esc(item.title)}</span>
            ${item.scope === "mine" && showScopeTag ? `<span class="rail-scope">我的</span>` : ""}
          </button>
        `).join("")}
      </aside>` : "";

    main.innerHTML = `
      <div class="nav-columns${hasContent ? "" : " no-rail"}">
        <div class="nav-content">
          ${sections.join("")}
        </div>
        ${rail}
      </div>
    `;

    bindIconFallbacks(main);
    updateScrollspy();
  }

  function renderAll() {
    renderTopbar();
    renderSettingsPanel();
    renderMain();
    bindIconFallbacks(root);
  }

  function setActiveCategory(key) {
    $$(".rail-item").forEach((button) => {
      button.classList.toggle("is-active", String(button.dataset.categoryJump) === String(key));
    });
  }

  function updateScrollspy() {
    const sections = $$("#nav-main .link-section");
    if (!sections.length) return;
    let active = "all";
    for (const section of sections) {
      if (section.getBoundingClientRect().top <= 150) active = section.dataset.category;
    }
    setActiveCategory(active);
  }

  function setFilter(filter) {
    if (!boot.isMember) return;
    const next = ["all", "public", "mine"].includes(filter) ? filter : "all";
    if (next === state.filter) {
      closeSettings();
      window.scrollTo({ top: 0, behavior: reduceMotion ? "auto" : "smooth" });
      return;
    }
    state.filter = next;
    closeSettings();
    const apply = () => {
      renderAll();
      window.scrollTo({ top: 0, behavior: "auto" });
    };
    if (document.startViewTransition && !reduceMotion) {
      document.startViewTransition(apply);
    } else {
      apply();
    }
  }

  /* ---------- Search ---------- */

  function commandFiltersMarkup() {
    const filters = [
      ["all", "全部"],
      ["public", "公共"],
      ...(boot.isMember ? [["mine", "我的"]] : []),
    ];
    return filters.map(([value, label]) => `
      <button type="button" class="filter-chip ${state.searchFilter === value ? "is-active" : ""}" data-search-filter="${value}">${label}</button>
    `).join("") + `<span class="filter-spacer"></span><span class="command-count" id="command-count"></span>`;
  }

  function commandCandidates() {
    const map = nameMap();
    let links = state.links;
    if (state.searchFilter === "public") links = links.filter((link) => link.scope === "public");
    if (state.searchFilter === "mine") links = links.filter((link) => link.scope === "mine");
    return links.map((link) => ({
      ...link,
      categoryName: link.categoryId ? map.get(link.categoryId) || "" : "未分类",
      host: hostOf(link.url),
    }));
  }

  function scoreLink(link, query, tokens, compact) {
    const title = link.title.toLowerCase();
    const description = (link.description || "").toLowerCase();
    const url = link.url.toLowerCase();
    const category = (link.categoryName || "").toLowerCase();
    const py = (link.py || "").toLowerCase();
    const pyi = (link.pyi || "").toLowerCase();
    const needle = query.toLowerCase();

    for (const token of tokens) {
      if (!(title.includes(token) || description.includes(token) || url.includes(token) || category.includes(token) || py.includes(token) || pyi.includes(token))) {
        return -1;
      }
    }

    let score = 0;
    if (title === needle) score += 160;
    else if (title.startsWith(needle)) score += 110;
    else if (title.includes(needle)) score += 65;
    if (category.includes(needle)) score += 28;
    if (description.includes(needle)) score += 14;
    if (url.includes(needle)) score += 8;
    if (pyi.startsWith(compact)) score += 55;
    else if (pyi.includes(compact)) score += 38;
    if (py.startsWith(compact)) score += 34;
    else if (py.includes(compact)) score += 20;
    score += Math.min(Number(link.clickCount) || 0, 24);
    return score;
  }

  function renderSearchResults() {
    const input = $("#command-input");
    const resultsEl = $("#command-results");
    const countEl = $("#command-count");
    if (!input || !resultsEl) return;
    const query = input.value.trim();
    const candidates = commandCandidates();
    let results;
    let label;
    if (!query) {
      results = candidates
        .slice()
        .sort((a, b) => (b.clickCount || 0) - (a.clickCount || 0) || a.title.localeCompare(b.title, "zh-Hans-CN"))
        .slice(0, 8);
      label = results.length ? "常用入口" : "还没有入口";
    } else {
      const tokens = query.toLowerCase().split(/\s+/).filter(Boolean);
      const compact = query.toLowerCase().replace(/\s+/g, "");
      results = candidates
        .map((link) => ({ link, score: scoreLink(link, query, tokens, compact) }))
        .filter((item) => item.score >= 0)
        .sort((a, b) => b.score - a.score || a.link.title.localeCompare(b.link.title, "zh-Hans-CN"))
        .slice(0, 30)
        .map((item) => item.link);
      label = "搜索结果";
    }

    state.searchResults = results;
    if (state.selectedIndex >= results.length) state.selectedIndex = Math.max(0, results.length - 1);
    if (state.selectedIndex < 0) state.selectedIndex = 0;
    if (countEl) countEl.textContent = query ? `${results.length} 个结果` : `${results.length} 个常用`;

    if (!results.length) {
      resultsEl.innerHTML = `
        <div class="command-empty">
          <strong>${query ? "没有找到" : "还没有入口"}</strong>
          <span>${query ? `试试「${esc(query)}」的其他写法，或按拼音首字母搜索。` : "添加链接后会在这里出现。"}</span>
        </div>`;
      return;
    }

    resultsEl.innerHTML = `
      <p class="command-group-label">${label}</p>
      ${results.map((link, index) => `
        <button type="button" class="command-result ${index === state.selectedIndex ? "is-selected" : ""}" data-result-index="${index}">
          ${iconBlock(link, "result")}
          <span class="result-copy">
            <span class="result-title-row"><span class="result-title">${esc(link.title)}</span></span>
            <span class="result-desc">${esc(link.description || link.categoryName || hostOf(link.url))}</span>
          </span>
          <span class="result-meta">
            <span class="scope-pill ${link.scope === "mine" ? "mine" : ""}">${link.scope === "mine" ? "我的" : "公共"}</span>
            <span class="result-host">${esc(link.host)}</span>
          </span>
        </button>
      `).join("")}
    `;
    bindIconFallbacks(resultsEl);
  }

  function openSearch(initial = "") {
    const overlay = $("#command-overlay");
    if (!overlay) return;
    closeSettings();
    state.searchFilter = "all";
    state.selectedIndex = 0;
    overlay.hidden = false;
    document.body.style.overflow = "hidden";
    $("#command-filters").innerHTML = commandFiltersMarkup();
    const input = $("#command-input");
    input.value = initial;
    renderSearchResults();
    requestAnimationFrame(() => input.focus());
  }

  function closeSearch() {
    const overlay = $("#command-overlay");
    if (!overlay || overlay.hidden) return;
    overlay.hidden = true;
    document.body.style.overflow = "";
  }

  function selectSearchResult(index) {
    if (!state.searchResults.length) return;
    state.selectedIndex = Math.max(0, Math.min(index, state.searchResults.length - 1));
    $$("#command-results .command-result").forEach((node, i) => {
      node.classList.toggle("is-selected", i === state.selectedIndex);
      if (i === state.selectedIndex) node.scrollIntoView({ block: "nearest" });
    });
  }

  function openResult(index) {
    const link = state.searchResults[index];
    if (!link) return;
    window.open(`/go/${link.id}`, "_blank", "noopener,noreferrer");
    closeSearch();
  }

  /* ---------- Toasts / confirm ---------- */

  function toast(message, kind = "success") {
    const region = $("#toast-region");
    if (!region) return;
    const node = document.createElement("div");
    node.className = `toast ${kind === "error" ? "is-error" : ""}`;
    node.innerHTML = `${icon(kind === "error" ? "alert" : "check")}<span>${esc(message)}</span>`;
    region.appendChild(node);
    setTimeout(() => {
      node.classList.add("is-leaving");
      setTimeout(() => node.remove(), 220);
    }, 3200);
  }

  function confirmAction(title, message, label, callback) {
    const dialog = $("#confirm-dialog");
    $("#confirm-title").textContent = title;
    $("#confirm-copy").textContent = message;
    const action = $("#confirm-action");
    action.textContent = label || "确认";
    state.confirmCallback = callback;
    dialog.showModal();
  }

  /* ---------- API ---------- */

  async function api(path, options = {}) {
    const headers = {
      "X-CSRF-Token": state.csrf || boot.csrf,
      ...(options.body ? { "Content-Type": "application/json" } : {}),
      ...(options.headers || {}),
    };
    const response = await fetch(path, { ...options, headers });
    let data = null;
    try { data = await response.json(); } catch (_) { /* empty */ }
    if (response.status === 401) {
      window.location.href = "/login";
      throw new Error("登录已过期");
    }
    if (!response.ok) {
      const error = new Error(data?.error || "操作没有完成，请稍后再试");
      error.field = data?.field;
      throw error;
    }
    return data;
  }

  async function refreshIcon(link) {
    if (!boot.iconFetch || !link || link.iconUrl || state.iconRefreshing.has(link.id)) return;
    state.iconRefreshing.add(link.id);
    const iconNode = $(`[data-icon-for="${link.id}"]`);
    iconNode?.classList.add("is-loading");
    try {
      const data = await api(`${boot.apiBase}/links/${link.id}/refresh-icon`, { method: "POST" });
      if (data?.iconUrl) {
        link.iconUrl = data.iconUrl;
        const current = $(`[data-icon-for="${link.id}"]`);
        if (current) {
          current.classList.remove("is-loading");
          current.innerHTML = `<img src="${esc(data.iconUrl)}" alt="" loading="lazy" data-icon-img><span class="site-icon-fallback tone-${Math.abs(Number(link.id) || 0) % 6}" hidden>${esc(initialOf(link.title))}</span>`;
          bindIconFallbacks(current);
        }
      } else {
        iconNode?.classList.remove("is-loading");
      }
    } catch (_) {
      iconNode?.classList.remove("is-loading");
    } finally {
      state.iconRefreshing.delete(link.id);
    }
  }

  /* ---------- Link dialog ---------- */

  function categoryOptions(selectedId) {
    const categories = categoriesFor("mine");
    return `<option value="">未分类</option>` + categories.map((category) => (
      `<option value="${esc(category.id)}" ${String(selectedId ?? "") === String(category.id) ? "selected" : ""}>${esc(category.name)}</option>`
    )).join("");
  }

  function openLinkDialog(link = null, categoryId = null) {
    const dialog = $("#link-dialog");
    const form = $("#link-form");
    form.reset();
    $("#link-form-alert").hidden = true;
    state.editingId = link ? link.id : null;
    $("#link-dialog-title").textContent = link ? "编辑链接" : "添加链接";
    $("#link-save-button").textContent = link ? "保存修改" : "保存链接";
    $("#link-category-select").innerHTML = categoryOptions(link ? link.categoryId : categoryId);
    if (link) {
      form.elements.title.value = link.title;
      form.elements.url.value = link.url;
      form.elements.description.value = link.description || "";
    }
    dialog.showModal();
    setTimeout(() => form.elements.title.focus(), 30);
  }

  async function submitLinkForm(event) {
    event.preventDefault();
    const form = event.currentTarget;
    const alert = $("#link-form-alert");
    alert.hidden = true;
    if (!form.reportValidity()) return;
    const title = form.elements.title.value.trim();
    const url = form.elements.url.value.trim();
    const description = form.elements.description.value.trim();
    const newCategory = form.elements.new_category.value.trim();
    const categoryValue = form.elements.category_id.value;
    if (!title || !url) {
      alert.textContent = "请填写名称和网址。";
      alert.hidden = false;
      return;
    }
    const wasEditing = Boolean(state.editingId);
    const button = $("#link-save-button");
    const original = button.textContent;
    button.disabled = true;
    button.textContent = "保存中…";
    try {
      const endpoint = wasEditing
        ? `${boot.apiBase}/links/${state.editingId}`
        : `${boot.apiBase}/links`;
      const method = wasEditing ? "PUT" : "POST";
      const data = await api(endpoint, {
        method,
        body: JSON.stringify({
          title,
          url,
          description,
          category_id: categoryValue ? Number(categoryValue) : null,
          new_category: newCategory,
        }),
      });
      if (data.category && !state.categories.some((item) => item.id === data.category.id)) {
        state.categories.push({ ...data.category, count: 0 });
      }
      if (data.link) {
        const index = state.links.findIndex((item) => item.id === data.link.id);
        if (index >= 0) state.links[index] = data.link;
        else state.links.push(data.link);
      }
      $("#link-dialog").close();
      renderAll();
      toast(wasEditing ? "链接已更新" : "链接已添加");
      if (!wasEditing) {
        requestAnimationFrame(() => {
          const target = $(`#section-mine-${data.link.categoryId ?? "unfiled"}`);
          target?.scrollIntoView({ behavior: reduceMotion ? "auto" : "smooth", block: "start" });
        });
      }
      if (!data.link.iconUrl) refreshIcon(data.link);
      state.editingId = null;
    } catch (error) {
      alert.textContent = error.message || "保存没有完成";
      alert.hidden = false;
      if (error.field && form.elements[error.field]) form.elements[error.field].focus();
    } finally {
      button.disabled = false;
      button.textContent = original;
    }
  }

  function deleteLink(link) {
    confirmAction("删除链接", `「${link.title}」会从你的个人导航中移除。`, "删除", async () => {
      try {
        await api(`${boot.apiBase}/links/${link.id}`, { method: "DELETE" });
        state.links = state.links.filter((item) => item.id !== link.id);
        $("#confirm-dialog").close();
        renderAll();
        toast("链接已删除");
      } catch (error) {
        toast(error.message || "删除没有完成", "error");
      }
    });
  }

  /* ---------- Category manager ---------- */

  function renderCategoryManager() {
    const body = $("#category-manager-body");
    if (!body) return;
    const categories = categoriesFor("mine");
    if (!categories.length) {
      body.innerHTML = `<div class="category-manager-empty">还没有分类。给常用工具分个组，会更容易找到。</div>`;
      return;
    }
    body.innerHTML = `<div class="category-manager-list">${categories.map((category) => `
      <div class="category-manager-row">
        <input class="input" type="text" maxlength="24" value="${esc(category.name)}" data-category-name="${esc(category.id)}" aria-label="分类名称">
        <span class="cat-count">${countForCategory(category.id, "mine")} 个</span>
        <button type="button" class="icon-btn icon-btn-sm" data-category-save="${esc(category.id)}" title="保存" aria-label="保存 ${esc(category.name)}">${icon("check")}</button>
        <button type="button" class="icon-btn icon-btn-sm icon-btn-danger" data-category-delete="${esc(category.id)}" title="删除" aria-label="删除 ${esc(category.name)}">${icon("trash")}</button>
      </div>
    `).join("")}</div>`;
  }

  function openCategoryManager() {
    renderCategoryManager();
    $("#category-dialog").showModal();
    setTimeout(() => {
      const input = $("#category-add-form input");
      input?.focus();
    }, 30);
  }

  async function saveCategory(id) {
    const input = $(`[data-category-name="${id}"]`);
    if (!input) return;
    const name = input.value.trim();
    if (!name) return;
    try {
      await api(`${boot.apiBase}/categories/${id}`, {
        method: "PATCH",
        body: JSON.stringify({ name }),
      });
      const category = state.categories.find((item) => item.id === Number(id));
      if (category) category.name = name;
      renderAll();
      renderCategoryManager();
      toast("分类名称已更新");
    } catch (error) {
      toast(error.message || "分类没有保存", "error");
      input.focus();
    }
  }

  function deleteCategory(category) {
    confirmAction("删除分类", `「${category.name}」中的链接会移到未分类，不会被删除。`, "删除分类", async () => {
      try {
        await api(`${boot.apiBase}/categories/${category.id}`, { method: "DELETE" });
        state.categories = state.categories.filter((item) => item.id !== category.id);
        state.links.forEach((link) => {
          if (link.categoryId === category.id) link.categoryId = null;
        });
        $("#confirm-dialog").close();
        renderAll();
        renderCategoryManager();
        toast("分类已删除");
      } catch (error) {
        toast(error.message || "分类没有删除", "error");
      }
    });
  }

  async function addCategory(event) {
    event.preventDefault();
    const form = event.currentTarget;
    const input = form.elements.name;
    const name = input.value.trim();
    if (!name) return;
    const button = form.querySelector("button");
    button.disabled = true;
    try {
      const data = await api(`${boot.apiBase}/categories`, {
        method: "POST",
        body: JSON.stringify({ name }),
      });
      state.categories.push({ ...data.category, count: 0 });
      input.value = "";
      renderAll();
      renderCategoryManager();
      toast("分类已添加");
    } catch (error) {
      toast(error.message || "分类没有添加", "error");
      input.focus();
    } finally {
      button.disabled = false;
    }
  }

  /* ---------- Settings panel ---------- */

  function jumpToCategory(key) {
    closeSettings();
    if (key === "all") {
      window.scrollTo({ top: 0, behavior: reduceMotion ? "auto" : "smooth" });
      setActiveCategory("all");
    } else {
      const section = $(`#section-${key}`);
      if (section) {
        section.scrollIntoView({ behavior: reduceMotion ? "auto" : "smooth", block: "start" });
        setActiveCategory(key);
      }
    }
  }

  /* ---------- Events ---------- */

  root.addEventListener("click", (event) => {
    const target = event.target;
    const closest = (selector) => target.closest(selector);

    if (!closest(".account-dock") && state.accountOpen) {
      closeSettings();
    }
    if (closest("[data-search-open]")) {
      closeSettings();
      openSearch();
      return;
    }
    if (closest("[data-search-close]")) {
      closeSearch();
      return;
    }
    if (closest("[data-nav-filter]")) {
      setFilter(closest("[data-nav-filter]").dataset.navFilter);
      return;
    }
    if (closest("[data-category-jump]")) {
      jumpToCategory(closest("[data-category-jump]").dataset.categoryJump);
      return;
    }
    if (closest("[data-manage-categories]")) {
      closeSettings();
      if (state.filter !== "mine") setFilter("mine");
      openCategoryManager();
      return;
    }
    if (closest("[data-account-toggle]")) {
      setSettings(!state.accountOpen);
      return;
    }
    if (closest("[data-settings-close]")) {
      closeSettings();
      return;
    }
    const spaceButton = closest("[data-space]");
    if (spaceButton) {
      window.location.href = `/s/${spaceButton.dataset.space}`;
      return;
    }

    const addButton = closest("[data-add-link]");
    if (addButton) {
      closeSettings();
      const raw = addButton.dataset.addLink;
      openLinkDialog(null, raw ? Number(raw) : null);
      return;
    }
    const editButton = closest("[data-edit-link]");
    if (editButton) {
      const link = state.links.find((item) => item.id === Number(editButton.dataset.editLink));
      if (link) openLinkDialog(link);
      return;
    }
    const deleteButton = closest("[data-delete-link]");
    if (deleteButton) {
      const link = state.links.find((item) => item.id === Number(deleteButton.dataset.deleteLink));
      if (link) deleteLink(link);
      return;
    }
    const categorySave = closest("[data-category-save]");
    if (categorySave) {
      saveCategory(categorySave.dataset.categorySave);
      return;
    }
    const categoryDelete = closest("[data-category-delete]");
    if (categoryDelete) {
      const category = state.categories.find((item) => item.id === Number(categoryDelete.dataset.categoryDelete));
      if (category) deleteCategory(category);
      return;
    }
    const dialogClose = closest("[data-dialog-close]");
    if (dialogClose) {
      dialogClose.closest("dialog")?.close();
      return;
    }
    const filter = closest("[data-search-filter]");
    if (filter) {
      state.searchFilter = filter.dataset.searchFilter;
      state.selectedIndex = 0;
      $("#command-filters").innerHTML = commandFiltersMarkup();
      renderSearchResults();
      $("#command-input")?.focus();
      return;
    }
    const result = closest("[data-result-index]");
    if (result) {
      openResult(Number(result.dataset.resultIndex));
      return;
    }
    if (target.id === "command-overlay") {
      closeSearch();
    }
  });

  root.addEventListener("mouseover", (event) => {
    const result = event.target.closest("[data-result-index]");
    if (result) {
      state.selectedIndex = Number(result.dataset.resultIndex);
      $$("#command-results .command-result").forEach((node, index) => {
        node.classList.toggle("is-selected", index === state.selectedIndex);
      });
    }
  });

  document.addEventListener("input", (event) => {
    if (event.target.id === "command-input") {
      state.selectedIndex = 0;
      renderSearchResults();
    }
  });

  document.addEventListener("keydown", (event) => {
    const commandOpen = !$("#command-overlay")?.hidden;
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
      event.preventDefault();
      if (commandOpen) closeSearch();
      else openSearch();
      return;
    }
    const activeTag = document.activeElement?.tagName?.toLowerCase();
    const typing = activeTag === "input" || activeTag === "textarea" || activeTag === "select";
    if (event.key === "/" && !typing && !commandOpen) {
      event.preventDefault();
      openSearch();
      return;
    }
    if (event.key === "Escape") {
      if (commandOpen) {
        closeSearch();
        return;
      }
      closeSettings();
      return;
    }
    if (!commandOpen) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      selectSearchResult(state.selectedIndex + 1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      selectSearchResult(state.selectedIndex - 1);
    } else if (event.key === "Enter") {
      event.preventDefault();
      openResult(state.selectedIndex);
    }
  });

  // initial paint
  root.innerHTML = shellMarkup();

  $("#link-form")?.addEventListener("submit", submitLinkForm);
  $("#category-add-form")?.addEventListener("submit", addCategory);
  $("#confirm-action")?.addEventListener("click", () => {
    const callback = state.confirmCallback;
    state.confirmCallback = null;
    if (typeof callback === "function") callback();
  });

  // Clicking the blank area around a dialog, or pressing Esc, used to discard a
  // half-typed form. Only dismiss without asking when nothing was entered; the
  // explicit 取消/关闭 buttons always work.
  function dialogHasContent(dialog) {
    return $$("input, textarea, select", dialog).some((field) => {
      if (field.disabled || field.type === "hidden") return false;
      if (field.type === "checkbox" || field.type === "radio") return field.checked;
      if (field.type === "file") return field.files.length > 0;
      if (field.tagName === "SELECT") return field.selectedIndex > 0;
      return String(field.value || "").trim() !== "";
    });
  }

  function bindDialogGuard(dialog) {
    dialog.addEventListener("click", (event) => {
      if (event.target !== dialog) return;
      if (!dialogHasContent(dialog)) dialog.close();
    });
    dialog.addEventListener("cancel", (event) => {
      if (dialogHasContent(dialog)) event.preventDefault();
    });
  }

  $$("dialog").forEach(bindDialogGuard);

  window.addEventListener("scroll", updateScrollspy, { passive: true });

  renderAll();
  if (boot.flash && boot.flash.message) {
    setTimeout(() => toast(boot.flash.message, boot.flash.kind === "error" ? "error" : "success"), 260);
  }
})();
