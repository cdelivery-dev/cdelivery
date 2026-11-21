import { mount } from "@cloudflare/nimbus-docs/client";

declare global {
  interface Window {
    __nbApplyTheme?: (mode?: "light" | "dark") => void;
  }
}

function initThemeToggle(button: HTMLElement): () => void {
  function handleClick() {
    const mode = document.documentElement.getAttribute("data-mode") === "dark" ? "light" : "dark";
    try {
      localStorage.setItem("ui-mode", mode);
      window.__nbApplyTheme?.();
    } catch {
      window.__nbApplyTheme?.(mode);
    }
  }

  window.__nbApplyTheme?.();
  button.addEventListener("click", handleClick);
  return () => button.removeEventListener("click", handleClick);
}

mount("[data-nb-theme-toggle]", initThemeToggle);
