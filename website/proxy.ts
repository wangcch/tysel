import { NextRequest, NextResponse } from "next/server";
import { isMarkdownPreferred, rewritePath } from "fumadocs-core/negotiation";
import {
  docsContentRoute,
  docsRoute,
  referenceContentRoute,
  referenceRoute,
} from "@/lib/shared";

const { rewrite: rewriteDocs } = rewritePath(
  `${docsRoute}{/*path}`,
  `${docsContentRoute}{/*path}/content.md`,
);
const { rewrite: rewriteDocsSuffix } = rewritePath(
  `${docsRoute}{/*path}.md`,
  `${docsContentRoute}{/*path}/content.md`,
);
const { rewrite: rewriteReference } = rewritePath(
  `${referenceRoute}{/*path}`,
  `${referenceContentRoute}{/*path}/content.md`,
);
const { rewrite: rewriteReferenceSuffix } = rewritePath(
  `${referenceRoute}{/*path}.md`,
  `${referenceContentRoute}{/*path}/content.md`,
);

export default function proxy(request: NextRequest) {
  const pathname = request.nextUrl.pathname;

  const suffixResult = rewriteDocsSuffix(pathname) ?? rewriteReferenceSuffix(pathname);
  if (suffixResult) {
    return NextResponse.rewrite(new URL(suffixResult, request.nextUrl));
  }

  if (isMarkdownPreferred(request)) {
    const markdownResult = rewriteDocs(pathname) ?? rewriteReference(pathname);
    if (markdownResult) {
      return NextResponse.rewrite(new URL(markdownResult, request.nextUrl), {
        headers: { Vary: "Accept" },
      });
    }
  }

  return NextResponse.next();
}
