"use client";

import { Moon, Sun } from "lucide-react";
import { useEffect, useState } from "react";

function isDarkMode() {
  return document.documentElement.classList.contains("dark");
}

function applyTheme(dark: boolean) {
  document.documentElement.classList.toggle("dark", dark);
  document.documentElement.style.colorScheme = dark ? "dark" : "light";
  window.localStorage.setItem("theme", dark ? "dark" : "light");
}

export function ThemeToggle() {
  const [dark, setDark] = useState(true);

  useEffect(() => {
    setDark(isDarkMode());
  }, []);

  return (
    <button
      type="button"
      aria-label={dark ? "Switch to light theme" : "Switch to dark theme"}
      className="inline-flex size-8 shrink-0 items-center justify-center text-fd-muted-foreground transition-colors hover:bg-fd-accent hover:text-fd-foreground"
      onClick={() => {
        const next = !isDarkMode();
        applyTheme(next);
        setDark(next);
      }}
    >
      {dark ? <Sun className="size-4" /> : <Moon className="size-4" />}
    </button>
  );
}
