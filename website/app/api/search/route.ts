import { blog, referenceSource, source } from "@/lib/source";
import { createFromSource } from "fumadocs-core/search/server";

const searchSource = {
  getPages() {
    return [
      ...source.getPages(),
      ...referenceSource.getPages(),
      ...blog.getPages(),
    ];
  },
  getPageTree() {
    return {
      name: "Tysel",
      children: [
        ...source.getPageTree().children,
        ...referenceSource.getPageTree().children,
        {
          type: "folder" as const,
          name: "Blog",
          root: true,
          children: blog.getPages().map((page) => ({
            type: "page" as const,
            name: page.data.title,
            url: page.url,
          })),
        },
      ],
    };
  },
};

const search = createFromSource(searchSource as unknown as typeof source);

export const dynamic = "force-static";
export const GET = search.staticGET;
