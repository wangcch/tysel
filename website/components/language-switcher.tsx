"use client";

import { useLocale } from "@/components/locale-provider";
import { headerIconButtonClass, headerIconLabelClass } from "@/components/header-chrome";
import { localeConfig, type Locale } from "@/lib/i18n/config";
import { availableLocalePath } from "@/lib/i18n/routing";
import { cn } from "@/lib/cn";
import { Check, ChevronDown } from "lucide-react";
import { usePathname } from "next/navigation";
import { useEffect, useId, useRef, useState } from "react";

export type LanguageChoice = { locale: Locale; name: string; paths: string[] };

function shortLabel(locale: Locale) {
  return localeConfig[locale].short ?? localeConfig[locale].name;
}

/** Two published locales: one-click switch to the other language. */
function LocaleToggle({ choices }: { choices: [LanguageChoice, LanguageChoice] }) {
  const { locale, t } = useLocale();
  const pathname = usePathname();
  const other = choices.find((choice) => choice.locale !== locale) ?? choices[0];

  return (
    <a
      href={availableLocalePath(pathname, other.locale, other.paths)}
      hrefLang={other.locale}
      lang={other.locale}
      aria-label={`${t("locale.label")}: ${other.name}`}
      title={other.name}
      className={headerIconButtonClass}
    >
      <span className={headerIconLabelClass}>{shortLabel(other.locale)}</span>
    </a>
  );
}

/** Three or more locales: compact menu of native names. */
function LocaleMenu({ choices }: { choices: LanguageChoice[] }) {
  const { locale, t } = useLocale();
  const pathname = usePathname();
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);
  const menuId = useId();

  useEffect(() => {
    if (!open) return;
    const onPointer = (event: MouseEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onPointer);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onPointer);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  useEffect(() => {
    setOpen(false);
  }, [pathname, locale]);

  return (
    <div ref={rootRef} className="relative">
      <button
        type="button"
        aria-label={t("locale.label")}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={menuId}
        className={cn(headerIconButtonClass, "gap-0.5")}
        onClick={() => setOpen((value) => !value)}
      >
        <span className={headerIconLabelClass}>{shortLabel(locale)}</span>
        <ChevronDown className={cn("size-2.5 opacity-70 transition-transform", open && "rotate-180")} />
      </button>
      {open ? (
        <ul
          id={menuId}
          role="menu"
          aria-label={t("locale.label")}
          className="absolute end-0 top-full z-50 mt-1 min-w-[10rem] border border-fd-border bg-fd-popover py-1 text-sm shadow-md"
        >
          {choices.map((choice) => {
            const active = choice.locale === locale;
            return (
              <li key={choice.locale} role="none">
                <a
                  role="menuitem"
                  href={availableLocalePath(pathname, choice.locale, choice.paths)}
                  hrefLang={choice.locale}
                  lang={choice.locale}
                  aria-current={active ? "true" : undefined}
                  className={cn(
                    "flex items-center justify-between gap-3 px-3 py-1.5 text-fd-muted-foreground transition-colors hover:bg-fd-accent hover:text-fd-foreground",
                    active && "bg-fd-accent/60 text-fd-foreground",
                  )}
                  onClick={() => setOpen(false)}
                >
                  <span>{choice.name}</span>
                  {active ? <Check className="size-3.5 shrink-0 opacity-70" aria-hidden /> : <span className="size-3.5" />}
                </a>
              </li>
            );
          })}
        </ul>
      ) : null}
    </div>
  );
}

export function LanguageSwitcher({ choices }: { choices: LanguageChoice[] }) {
  if (choices.length < 2) return null;
  if (choices.length === 2) {
    return <LocaleToggle choices={choices as [LanguageChoice, LanguageChoice]} />;
  }
  return <LocaleMenu choices={choices} />;
}
