import { referenceSource, source } from "@/lib/source";
import { createFromSource } from "fumadocs-core/search/server";

const searchSource = {
  getPages() {
    return [...source.getPages(), ...referenceSource.getPages()];
  },
  getPageTree() {
    return {
      name: "Tysel",
      children: [
        ...source.getPageTree().children,
        ...referenceSource.getPageTree().children,
      ],
    };
  },
};

const search = createFromSource(searchSource as unknown as typeof source);

export const dynamic = "force-static";
export const GET = search.staticGET;
