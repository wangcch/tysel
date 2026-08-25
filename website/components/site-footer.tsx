import Link from "next/link";
import { githubUrl } from "@/lib/shared";
import { Wordmark } from "@/components/wordmark";

const columns = [
  {
    title: "Product",
    links: [
      { href: "/docs", label: "Docs" },
      { href: "/reference", label: "Reference" },
      { href: "/examples", label: "Examples" },
      { href: "/benchmarks", label: "Benchmarks" },
    ],
  },
  {
    title: "Source",
    links: [
      { href: githubUrl, label: "GitHub", external: true },
      {
        href: `${githubUrl}/blob/main/LICENSE`,
        label: "Apache-2.0",
        external: true,
      },
    ],
  },
];

export function SiteFooter() {
  return (
    <footer className="mt-auto border-t border-fd-border bg-fd-background">
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-10 px-6 py-10 sm:flex-row sm:items-start sm:justify-between">
        <div>
          <Link
            href="/"
            className="inline-flex text-fd-foreground"
            aria-label="Tysel home"
          >
            <Wordmark className="h-7" />
          </Link>
          <p className="mt-3 max-w-xs text-sm leading-6 text-fd-muted-foreground">
            A native TypeScript runtime for services and agents.
          </p>
        </div>

        <div className="flex gap-16">
          {columns.map((column) => (
            <div key={column.title}>
              <p className="text-xs font-medium uppercase tracking-[0.14em] text-fd-muted-foreground">
                {column.title}
              </p>
              <ul className="mt-3 space-y-2">
                {column.links.map((link) => (
                  <li key={link.href}>
                    <Link
                      href={link.href}
                      {...("external" in link && link.external
                        ? { target: "_blank", rel: "noreferrer" }
                        : {})}
                      className="text-sm text-fd-muted-foreground transition-colors hover:text-fd-foreground"
                    >
                      {link.label}
                    </Link>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
      </div>
    </footer>
  );
}
