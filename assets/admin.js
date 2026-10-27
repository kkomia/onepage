(() => {
  "use strict";

  const sidebar = document.getElementById("admin-sidebar");
  const scrim = document.querySelector(".sidebar-scrim");

  function openSidebar() {
    sidebar?.classList.add("is-open");
  }

  function closeSidebar() {
    sidebar?.classList.remove("is-open");
  }

  function focusDialog(dialog) {
    const focusTarget = dialog.querySelector("input:not([type=hidden]), textarea, select");
    if (focusTarget) setTimeout(() => focusTarget.focus(), 40);
  }

  document.addEventListener("click", (event) => {
    const dialogTrigger = event.target.closest("[data-dialog-open]");
    if (dialogTrigger) {
      const dialog = document.getElementById(dialogTrigger.dataset.dialogOpen);
      if (dialog) {
        dialog.showModal();
        focusDialog(dialog);
      }
      return;
    }
    const dialogClose = event.target.closest("[data-dialog-close]");
    if (dialogClose) {
      dialogClose.closest("dialog")?.close();
      return;
    }
    if (event.target.closest("[data-sidebar-open]")) {
      openSidebar();
      return;
    }
    if (event.target.closest("[data-sidebar-close]")) {
      closeSidebar();
    }
  });

  scrim?.addEventListener("click", closeSidebar);

  // Dialogs are opened by the server through data-open="true".
  const openDialogs = Array.from(document.querySelectorAll("dialog[data-open='true']"));
  openDialogs.forEach((dialog) => {
    dialog.showModal();
    focusDialog(dialog);
  });

  document.querySelectorAll("dialog").forEach((dialog) => {
    dialog.addEventListener("click", (event) => {
      if (event.target === dialog) dialog.close();
    });
    dialog.addEventListener("close", () => {
      dialog.removeAttribute("data-open");
      cleanupDialogQuery();
    });
  });

  function cleanupDialogQuery() {
    const url = new URL(window.location.href);
    const keys = ["dialog", "edit", "reset"];
    let changed = false;
    keys.forEach((key) => {
      if (url.searchParams.has(key)) {
        url.searchParams.delete(key);
        changed = true;
      }
    });
    if (changed) history.replaceState({}, "", url.toString());
  }

  // Confirm dialog for destructive forms.
  const confirmDialog = document.createElement("dialog");
  confirmDialog.className = "modal confirm-modal";
  confirmDialog.innerHTML = `
    <div class="modal-head">
      <div>
        <p class="eyebrow">请确认</p>
        <h2>确认操作</h2>
      </div>
      <button type="button" class="icon-btn" data-confirm-cancel aria-label="关闭">
        <svg class="icon" aria-hidden="true"><use href="/assets/icons.svg#i-close"></use></svg>
      </button>
    </div>
    <div class="modal-form">
      <p class="confirm-copy" data-confirm-copy></p>
      <div class="modal-actions">
        <button type="button" class="btn btn-ghost" data-confirm-cancel>取消</button>
        <button type="button" class="btn btn-danger" data-confirm-accept>确认</button>
      </div>
    </div>`;
  document.body.appendChild(confirmDialog);

  let pendingForm = null;
  let lastTrigger = null;

  document.addEventListener("submit", (event) => {
    const form = event.target;
    if (!(form instanceof HTMLFormElement)) return;
    if (form.dataset.confirmed === "1") {
      delete form.dataset.confirmed;
      return;
    }
    const message = form.dataset.confirm;
    if (!message) return;
    event.preventDefault();
    pendingForm = form;
    lastTrigger = document.activeElement;
    confirmDialog.querySelector("[data-confirm-copy]").textContent = message;
    confirmDialog.showModal();
    confirmDialog.querySelector("[data-confirm-accept]").focus();
  }, true);

  confirmDialog.addEventListener("click", (event) => {
    if (event.target === confirmDialog) {
      confirmDialog.close();
      return;
    }
    if (event.target.closest("[data-confirm-cancel]")) {
      confirmDialog.close();
      return;
    }
    if (event.target.closest("[data-confirm-accept]")) {
      const form = pendingForm;
      pendingForm = null;
      confirmDialog.close();
      if (form) {
        form.dataset.confirmed = "1";
        form.requestSubmit();
      }
    }
  });

  confirmDialog.addEventListener("close", () => {
    pendingForm = null;
    if (lastTrigger && lastTrigger.isConnected) lastTrigger.focus();
    lastTrigger = null;
  });

  // Broken favicon images fall back to a letter mark.
  function bindImageFallbacks() {
    document.querySelectorAll("img.site-icon").forEach((img) => {
      if (img.dataset.bound) return;
      img.dataset.bound = "1";
      img.addEventListener("error", () => {
        const row = img.closest("tr");
        const title = row?.querySelector(".entity-title")?.textContent?.trim() || "·";
        const fallback = document.createElement("span");
        fallback.className = "site-icon site-icon-fallback tone-0";
        fallback.setAttribute("aria-hidden", "true");
        fallback.textContent = Array.from(title)[0]?.toUpperCase() || "·";
        img.replaceWith(fallback);
      }, { once: true });
    });
  }

  bindImageFallbacks();

  // Close the mobile sidebar when the viewport grows.
  window.addEventListener("resize", () => {
    if (window.innerWidth > 960) closeSidebar();
  });
})();
