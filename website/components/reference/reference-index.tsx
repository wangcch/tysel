import Link from "next/link";
import { referenceCatalog } from "@/lib/reference-catalog";

export function ReferenceIndex() {
  return (
    <div className="not-prose -mx-1 space-y-12">
      {referenceCatalog.map((section) => (
        <section key={section.title}>
          <div className="mb-4">
            <h2 className="text-lg font-semibold tracking-tight">
              {section.title}
            </h2>
            {section.blurb ? (
              <p className="mt-1 max-w-2xl text-sm leading-6 text-fd-muted-foreground">
                {section.blurb}
              </p>
            ) : null}
          </div>

          <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {section.tiles.map((tile) => (
              <Link
                key={tile.href}
                href={tile.href}
                className="group border border-fd-border bg-fd-background p-4 transition-colors hover:bg-fd-accent"
              >
                <p className="font-mono text-sm text-tysel-blue group-hover:underline">
                  {tile.name}
                </p>
                <p className="mt-2 text-sm leading-5 text-fd-muted-foreground">
                  {tile.description}
                </p>
              </Link>
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}
