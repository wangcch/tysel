import { sourceLocale } from "@/lib/i18n/config";

/** Quiet meta link back to the English source — sits in page chrome, not in the prose flow. */
export function LocaleOriginalLink({
  href,
  label,
  className,
}: {
  href: string;
  label: string;
  className?: string;
}) {
  return (
    <a
      href={href}
      hrefLang={sourceLocale}
      className={
        className ??
        "text-sm text-fd-muted-foreground transition-colors hover:text-fd-foreground"
      }
    >
      {label}
    </a>
  );
}
