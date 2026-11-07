import { mount } from "@cloudflare/nimbus-docs/client";

const STORAGE_KEY = "sidebar-state";

interface SidebarState {
  hash: string;
  open: boolean[];
  scroll: number;
}

function groupsOf(root: HTMLElement): HTMLDetailsElement[] {
  return Array.from(
    root.querySelectorAll<HTMLDetailsElement>("details"),
  );
}

function initSidebar(root: HTMLElement): () => void {
  const teardowns: Array<() => void> = [];
  const persist = root.hasAttribute("data-nb-sidebar-persist");

  const filterTeardown = initFilter(root);
  if (filterTeardown) teardowns.push(filterTeardown);

  if (persist) {
    const persistTeardown = initPersistence(root);
    if (persistTeardown) teardowns.push(persistTeardown);
  }

  return () => teardowns.forEach((t) => t());
}

function initFilter(root: HTMLElement): (() => void) | null {
  const input = root.querySelector<HTMLInputElement>("[data-nb-sidebar-filter-input]");
  const inputElement =
    input ?? root.parentElement?.querySelector<HTMLInputElement>("[data-nb-sidebar-filter-input]") ?? null;
  if (!inputElement) return null;

  function handleInput() {
    const query = inputElement!.value.trim().toLowerCase();
    if (!query) {
      resetFilter(root);
      return;
    }
    applyFilter(root, query);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      inputElement!.value = "";
      handleInput();
      inputElement!.blur();
    }
  }

  inputElement.addEventListener("input", handleInput);
  inputElement.addEventListener("keydown", handleKeydown);

  return () => {
    inputElement.removeEventListener("input", handleInput);
    inputElement.removeEventListener("keydown", handleKeydown);
    resetFilter(root);
  };
}

function resetFilter(root: HTMLElement): void {
  root.querySelectorAll<HTMLElement>("[data-nb-sidebar-hidden]").forEach((el) => {
    el.removeAttribute("data-nb-sidebar-hidden");
  });
  root
    .querySelectorAll<HTMLDetailsElement>("details[data-nb-opened-by-filter]")
    .forEach((group) => {
      group.open = false;
      group.removeAttribute("data-nb-opened-by-filter");
    });
}

function applyFilter(root: HTMLElement, query: string): void {
  const links = root.querySelectorAll<HTMLElement>("li > a");
  const groups = root.querySelectorAll<HTMLElement>("details");

  links.forEach((link) => link.setAttribute("data-nb-sidebar-hidden", ""));
  groups.forEach((group) => group.setAttribute("data-nb-sidebar-hidden", ""));

  links.forEach((link) => {
    const method = link.querySelector<HTMLElement>("i[data-m]")?.dataset.m ?? "";
    const text = `${link.textContent ?? ""} ${method}`.toLowerCase();
    if (!text.includes(query)) return;
    link.removeAttribute("data-nb-sidebar-hidden");
    revealAncestors(link, root);
  });

  groups.forEach((group) => {
    const label = group.querySelector(":scope > summary");
    const text = label?.textContent?.toLowerCase() ?? "";
    if (!text.includes(query)) return;
    group.removeAttribute("data-nb-sidebar-hidden");
    openGroup(group);
    group.querySelectorAll<HTMLElement>("li > a, details")
      .forEach((child) => child.removeAttribute("data-nb-sidebar-hidden"));
  });
}

function revealAncestors(el: HTMLElement, scope: Element): void {
  let parent: HTMLElement | null = el.parentElement;
  while (parent && parent !== scope) {
    if (parent instanceof HTMLDetailsElement) {
      parent.removeAttribute("data-nb-sidebar-hidden");
      openGroup(parent);
    }
    parent = parent.parentElement;
  }
}

function openGroup(group: HTMLElement): void {
  if (!(group instanceof HTMLDetailsElement) || group.open) return;
  group.setAttribute("data-nb-opened-by-filter", "");
  group.open = true;
}

function initPersistence(root: HTMLElement): (() => void) | null {
  const scrollHost: HTMLElement =
    root.closest<HTMLElement>("[data-nb-sidebar-tree]") ?? root.closest("aside") ?? root;
  const hash = root.dataset.nbSidebarHash ?? "";

  function savedScroll(): number | null {
    try {
      const saved = JSON.parse(sessionStorage.getItem(STORAGE_KEY) ?? "null");
      return saved?.hash === hash && typeof saved.scroll === "number" ? saved.scroll : null;
    } catch {
      return null;
    }
  }

  function readState(): SidebarState {
    return {
      hash,
      open: groupsOf(root).map((group) => group.open),
      scroll: scrollHost.clientHeight > 0 ? scrollHost.scrollTop : (savedScroll() ?? 0),
    };
  }

  function restoreScroll() {
    const scroll = savedScroll();
    if (scroll !== null) scrollHost.scrollTop = scroll;
    const active = root.querySelector("[aria-current='page']");
    if (!active) return;
    const box = scrollHost.getBoundingClientRect();
    const item = active.getBoundingClientRect();
    if (item.top < box.top || item.bottom > box.bottom) {
      scrollHost.scrollTop += item.top - box.top - box.height / 2 + item.height / 2;
    }
  }

  let visible = scrollHost.clientHeight > 0;
  const resize =
    typeof ResizeObserver === "undefined"
      ? undefined
      : new ResizeObserver(() => {
          const nowVisible = scrollHost.clientHeight > 0;
          if (nowVisible && !visible) restoreScroll();
          visible = nowVisible;
        });
  resize?.observe(scrollHost);

  function save() {
    try {
      sessionStorage.setItem(STORAGE_KEY, JSON.stringify(readState()));
    } catch {}
  }

  const handleToggle = (event: Event) => {
    const group = event.target as HTMLElement | null;
    if (!(group instanceof HTMLDetailsElement)) return;
    if (group.hasAttribute("data-nb-opened-by-filter")) return;
    save();
  };
  root.addEventListener("toggle", handleToggle, true);

  function handleVisibility() {
    if (document.visibilityState === "hidden") save();
  }
  document.addEventListener("visibilitychange", handleVisibility);
  window.addEventListener("pagehide", save);

  let raf = 0;
  function handleScroll() {
    cancelAnimationFrame(raf);
    raf = requestAnimationFrame(save);
  }
  scrollHost.addEventListener("scroll", handleScroll);

  return () => {
    resize?.disconnect();
    root.removeEventListener("toggle", handleToggle, true);
    document.removeEventListener("visibilitychange", handleVisibility);
    window.removeEventListener("pagehide", save);
    scrollHost.removeEventListener("scroll", handleScroll);
    cancelAnimationFrame(raf);
  };
}

(function bindFilterShortcut() {
  if (document.documentElement.hasAttribute("data-nb-sidebar-shortcut-bound")) return;
  document.documentElement.setAttribute("data-nb-sidebar-shortcut-bound", "");

  document.addEventListener("keydown", (e) => {
    if (e.key !== "/") return;
    const active = document.activeElement as HTMLElement | null;
    if (
      active &&
      (active.tagName === "INPUT" ||
        active.tagName === "TEXTAREA" ||
        active.isContentEditable)
    ) {
      return;
    }
    const input = Array.from(
      document.querySelectorAll<HTMLInputElement>("[data-nb-sidebar-filter-input]"),
    ).find((candidate) => candidate.offsetParent !== null);
    if (!input) return;
    e.preventDefault();
    input.focus();
  });
})();

mount("[data-nb-sidebar]", initSidebar);
