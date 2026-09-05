"use client";

import { useLocale } from "@/components/locale-provider";
import { headerIconButtonClass, headerIconClass } from "@/components/header-chrome";

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
  const { t } = useLocale();
  const [dark, setDark] = useState(true);

  useEffect(() => {
    setDark(isDarkMode());
  }, []);

  return (
    <button
      type="button"
      aria-label={t(dark ? "theme.light" : "theme.dark")}
      className={headerIconButtonClass}
      onClick={() => {
        const next = !isDarkMode();
        applyTheme(next);
        setDark(next);
      }}
    >
      {dark ? <Sun className={headerIconClass} /> : <Moon className={headerIconClass} />}
    </button>
  );
}
