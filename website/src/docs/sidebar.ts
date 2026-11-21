import { mount, lockScroll, unlockScroll, generateId } from "@cloudflare/nimbus-docs/client";

const CLOSE_DURATION_MS = 250;
const DESKTOP_QUERY = "(min-width: 48rem)";

mount("[data-mobile-sidebar]", (root) => {
  const trigger = document.querySelector<HTMLElement>("[data-menu-btn]");
  if (!(root instanceof HTMLDialogElement) || !trigger) return () => {};
  const dialog = root;

  const controller = new AbortController();
  const { signal } = controller;
  let closeTimer: ReturnType<typeof setTimeout> | undefined;

  const prefersReducedMotion = () =>
    window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  if (!dialog.id) dialog.id = generateId("nb-mobile-sidebar");
  trigger.setAttribute("aria-controls", dialog.id);
  trigger.setAttribute("aria-expanded", "false");
  if (!trigger.hasAttribute("aria-haspopup")) trigger.setAttribute("aria-haspopup", "dialog");

  const clearTimer = () => {
    if (closeTimer) {
      clearTimeout(closeTimer);
      closeTimer = undefined;
    }
  };

  const tree = document.querySelector<HTMLElement>("[data-nb-sidebar-tree]");
  const rail = tree?.parentElement ?? null;
  const drawerSlot = dialog.querySelector<HTMLElement>("[data-nb-sidebar-drawer-slot]");
  const moveTreeToDrawer = () => {
    if (tree && drawerSlot && tree.parentElement !== drawerSlot) drawerSlot.append(tree);
  };
  const moveTreeToRail = () => {
    if (tree && rail && tree.parentElement !== rail) rail.prepend(tree);
  };

  const openSidebar = () => {
    clearTimer();
    if (dialog.open) {
      if (dialog.dataset.state === "closing") dialog.dataset.state = "open";
      return;
    }

    moveTreeToDrawer();
    dialog.showModal();
    dialog.dataset.state = "closed";
    lockScroll();
    trigger.setAttribute("aria-expanded", "true");

    void dialog.offsetWidth;
    dialog.dataset.state = "open";

    dialog.querySelector<HTMLElement>("[data-close-sidebar]")?.focus({ preventScroll: true });
  };

  const closeNow = () => {
    if (!dialog.open) return;
    clearTimer();
    dialog.close();
  };

  const closeSidebar = () => {
    if (!dialog.open || dialog.dataset.state === "closing") return;
    if (prefersReducedMotion()) {
      closeNow();
      return;
    }
    dialog.dataset.state = "closing";
    closeTimer = setTimeout(() => dialog.close(), CLOSE_DURATION_MS);
  };

  trigger.addEventListener("click", openSidebar, { signal });

  dialog.addEventListener("cancel", (event) => {
    event.preventDefault();
    closeSidebar();
  }, { signal });

  dialog.addEventListener("close", () => {
    clearTimer();
    dialog.dataset.state = "closed";
    moveTreeToRail();
    unlockScroll();
    trigger.setAttribute("aria-expanded", "false");
    trigger.focus();
  }, { signal });

  dialog.querySelectorAll<HTMLElement>("[data-close-sidebar]")
    .forEach((el) => el.addEventListener("click", closeSidebar, { signal }));

  dialog.addEventListener("click", (e) => {
    if (e.target === dialog) closeSidebar();
  }, { signal });

  const mql = window.matchMedia(DESKTOP_QUERY);
  mql.addEventListener("change", (event) => {
    if (event.matches) closeNow();
  }, { signal });
  if (mql.matches) closeNow();

  return () => {
    controller.abort();
    clearTimer();
    moveTreeToRail();
    trigger.setAttribute("aria-expanded", "false");
    if (dialog.open) unlockScroll();
  };
});
