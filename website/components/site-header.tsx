"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect, useId, useState } from "react";
import {
  FullSearchTrigger,
  SearchTrigger,
} from "fumadocs-ui/layouts/shared/slots/search-trigger";
import { Menu, X } from "lucide-react";
import { cn } from "@/lib/cn";
import { githubUrl } from "@/lib/shared";
import { Wordmark } from "@/components/wordmark";
import { ThemeToggle } from "@/components/theme-toggle";

const links = [
  { href: "/", label: "Home" },
  { href: "/docs", label: "Docs" },
  { href: "/blog", label: "Blog" },
  { href: "/reference", label: "Reference" },
  { href: "/examples", label: "Examples" },
  { href: "/benchmarks", label: "Benchmarks" },
];

function GitHubIcon() {
  return (
    <svg role="img" viewBox="0 0 24 24" fill="currentColor" className="size-4">
      <path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
    </svg>
  );
}

function isActive(pathname: string, href: string) {
  return pathname === href || (href !== "/" && pathname.startsWith(href));
}

/** Shared top nav — padding matches fumadocs docs sidebar (`p-4`). */
export function SiteHeader() {
  const pathname = usePathname();
  const [open, setOpen] = useState(false);
  const menuId = useId();

  useEffect(() => {
    setOpen(false);
  }, [pathname]);

  const docsChrome =
    pathname.startsWith("/docs") || pathname.startsWith("/reference");

  return (
    <header className="sticky top-0 z-50 border-b border-fd-border bg-fd-background/80 backdrop-blur-sm">
      <div
        className={cn(
          "mx-auto flex h-14 w-full items-center gap-3 transition-[max-width,padding] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)] motion-reduce:transition-none",
          docsChrome
            ? "max-w-[var(--fd-layout-width,97rem)] px-4"
            : "max-w-6xl px-6",
        )}
      >
        <Link
          href="/"
          className="me-2 inline-flex shrink-0 items-center text-fd-foreground"
        >
          <Wordmark className="h-6" />
        </Link>

        <nav className="hidden shrink-0 items-center gap-0.5 lg:flex">
          {links.map((link) => (
            <Link
              key={link.href}
              href={link.href}
              className={cn(
                "rounded-md px-2.5 py-1.5 text-sm text-fd-muted-foreground transition-colors hover:text-fd-foreground",
                isActive(pathname, link.href)
                  ? "text-fd-foreground"
                  : undefined,
              )}
            >
              {link.label}
            </Link>
          ))}
        </nav>

        <div className="ms-auto flex min-w-0 flex-1 items-center justify-end gap-2">
          <FullSearchTrigger
            hideIfDisabled
            className="hidden w-[min(100%,13rem)] shrink rounded-none md:inline-flex"
          />
          <div className="flex shrink-0 items-center gap-1.5">
            <SearchTrigger hideIfDisabled className="shrink-0 rounded-none p-2 md:hidden" />
            <ThemeToggle />
            <Link
              href={githubUrl}
              target="_blank"
              rel="noreferrer"
              aria-label="GitHub"
              className="inline-flex size-8 shrink-0 items-center justify-center text-fd-muted-foreground transition-colors hover:bg-fd-accent hover:text-fd-foreground"
            >
              <GitHubIcon />
            </Link>
            <Link
              href="/docs/getting-started"
              className="hidden shrink-0 items-center whitespace-nowrap bg-fd-foreground px-3 py-1.5 text-sm font-medium text-fd-background lg:inline-flex"
            >
              Get started
            </Link>
            <button
              type="button"
              className="inline-flex size-8 items-center justify-center text-fd-muted-foreground transition-colors hover:bg-fd-accent hover:text-fd-foreground lg:hidden"
              aria-expanded={open}
              aria-controls={menuId}
              aria-label={open ? "Close menu" : "Open menu"}
              onClick={() => setOpen((value) => !value)}
            >
              {open ? <X className="size-4" /> : <Menu className="size-4" />}
            </button>
          </div>
        </div>
      </div>

      {open ? (
        <nav
          id={menuId}
          className={cn(
            "border-t border-fd-border py-3 lg:hidden",
            docsChrome ? "px-4" : "px-6",
          )}
        >
          <ul className="space-y-0.5">
            {links.map((link) => (
              <li key={link.href}>
                <Link
                  href={link.href}
                  className={cn(
                    "block rounded-md px-3 py-2 text-sm text-fd-muted-foreground transition-colors hover:bg-fd-accent hover:text-fd-foreground",
                    isActive(pathname, link.href)
                      ? "bg-fd-accent text-fd-foreground"
                      : undefined,
                  )}
                >
                  {link.label}
                </Link>
              </li>
            ))}
            <li>
              <Link
                href="/docs/getting-started"
                className="mt-2 block bg-fd-foreground px-3 py-2 text-center text-sm font-medium text-fd-background"
              >
                Get started
              </Link>
            </li>
          </ul>
        </nav>
      ) : null}
    </header>
  );
}
