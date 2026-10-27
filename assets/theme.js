(() => {
  const KEY = "onepage-theme";
  const root = document.documentElement;
  const media = window.matchMedia("(prefers-color-scheme: dark)");

  function stored() {
    try {
      const value = localStorage.getItem(KEY);
      return value === "light" || value === "dark" ? value : null;
    } catch (_) {
      return null;
    }
  }

  function preferred() {
    return stored() || (media.matches ? "dark" : "light");
  }

  function apply(theme, persist) {
    root.dataset.theme = theme;
    if (persist) {
      try { localStorage.setItem(KEY, theme); } catch (_) { /* private mode */ }
    }
    const label = theme === "dark" ? "切换浅色模式" : "切换深色模式";
    document.querySelectorAll("[data-theme-toggle]").forEach((button) => {
      button.setAttribute("aria-label", label);
      button.setAttribute("title", label);
    });
    window.dispatchEvent(new CustomEvent("onepage:theme", { detail: { theme } }));
  }

  apply(preferred(), false);

  document.addEventListener("click", (event) => {
    const button = event.target.closest("[data-theme-toggle]");
    if (!button) return;
    apply(root.dataset.theme === "dark" ? "light" : "dark", true);
  });

  media.addEventListener?.("change", () => {
    if (!stored()) apply(preferred(), false);
  });

  document.addEventListener("DOMContentLoaded", () => {
    apply(root.dataset.theme === "dark" ? "dark" : "light", false);
  });

  window.OnePageTheme = {
    get: () => root.dataset.theme,
    set: (theme) => apply(theme === "dark" ? "dark" : "light", true),
  };
})();
