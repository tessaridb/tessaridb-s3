//! Light or dark. Until the operator chooses, the page follows the system's
//! preference; a choice is remembered in this browser only (it is a viewer's
//! convenience, not console state), and a browser that refuses storage simply
//! forgets it.

import { icon } from "./icons.ts";

type Theme = "light" | "dark";

const KEY = "tessaridb-s3-console-theme";
const darkQuery = matchMedia("(prefers-color-scheme: dark)");

function remembered(): Theme | null {
  try {
    const value = localStorage.getItem(KEY);
    return value === "light" || value === "dark" ? value : null;
  } catch {
    return null;
  }
}

function remember(theme: Theme): void {
  try {
    localStorage.setItem(KEY, theme);
  } catch {
    // Storage refused: the choice lasts for this page only.
  }
}

const current = (): Theme => remembered() ?? (darkQuery.matches ? "dark" : "light");

/** Applies the remembered or system theme and makes `button` switch between the two. */
export function themes(button: HTMLElement): void {
  const show = (): void => {
    const theme = current();
    if (remembered() === null) {
      document.documentElement.removeAttribute("data-theme");
    } else {
      document.documentElement.setAttribute("data-theme", theme);
    }
    button.replaceChildren(icon(theme === "dark" ? "sun" : "moon"), theme === "dark" ? "Light theme" : "Dark theme");
  };
  button.addEventListener("click", () => {
    remember(current() === "dark" ? "light" : "dark");
    show();
  });
  darkQuery.addEventListener("change", show);
  show();
}
