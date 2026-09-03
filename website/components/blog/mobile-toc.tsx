"use client";

import { useId, useState } from "react";
import type { TOCItemType } from "fumadocs-core/toc";
import { ChevronDown } from "lucide-react";
import { cn } from "@/lib/cn";

type BlogMobileTocProps = {
  items: TOCItemType[];
};

/** Native disclosure TOC — avoids fumadocs InlineTOC hydration issues on static export. */
export function BlogMobileToc({ items }: BlogMobileTocProps) {
  const [open, setOpen] = useState(false);
  const panelId = useId();
  const headings = items.filter((item) => item.depth <= 2);

  if (headings.length === 0) return null;

  return (
    <div className="mb-8 border border-fd-border lg:hidden">
      <button
        type="button"
        className="flex w-full items-center justify-between px-4 py-3 text-sm font-medium"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={() => setOpen((value) => !value)}
      >
        On this page
        <ChevronDown
          className={cn(
            "size-4 text-fd-muted-foreground transition-transform duration-200",
            open && "rotate-180",
          )}
        />
      </button>
      {open ? (
        <nav id={panelId} className="border-t border-fd-border px-4 py-3">
          <ul className="space-y-2">
            {headings.map((item) => (
              <li key={item.url}>
                <a
                  href={item.url}
                  className="block text-sm leading-5 text-fd-muted-foreground transition-colors hover:text-fd-foreground"
                  onClick={() => setOpen(false)}
                >
                  {item.title}
                </a>
              </li>
            ))}
          </ul>
        </nav>
      ) : null}
    </div>
  );
}
