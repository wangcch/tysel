import {
  getLLMText,
  getPageImageUrl,
  getPageMarkdownUrl,
  getPageSourcePath,
  referenceSource,
  source,
} from "@/lib/source";
import {
  DocsBody,
  DocsDescription,
  DocsPage,
  DocsTitle,
  MarkdownCopyButton,
  ViewOptionsPopover,
} from "fumadocs-ui/layouts/docs/page";
import { notFound } from "next/navigation";
import { getMDXComponents } from "@/components/mdx";
import { ReferenceIndex } from "@/components/reference/reference-index";
import type { Metadata } from "next";
import { createRelativeLink } from "fumadocs-ui/mdx";
import { gitConfig } from "@/lib/shared";

type ReferencePageProps = PageProps<"/reference/[[...slug]]">;

export default async function Page(props: ReferencePageProps) {
  const params = await props.params;
  const page = referenceSource.getPage(params.slug);
  if (!page) notFound();

  const MDX = page.data.body;
  const markdownUrl = getPageMarkdownUrl(page).url;
  const isIndex = !params.slug || params.slug.length === 0;

  return (
    <DocsPage toc={isIndex ? [] : page.data.toc} full={page.data.full}>
      <DocsTitle>{isIndex ? "API reference" : page.data.title}</DocsTitle>
      <DocsDescription className="mb-0">
        {isIndex
          ? "Exact interfaces — accepted values, defaults, side effects, and what each profile can still deny."
          : page.data.description}
      </DocsDescription>
      <div className="flex flex-row items-center gap-2 border-b pb-6">
        <MarkdownCopyButton markdownUrl={markdownUrl} />
        <ViewOptionsPopover
          markdownUrl={markdownUrl}
          githubUrl={`https://github.com/${gitConfig.user}/${gitConfig.repo}/blob/${gitConfig.branch}/${getPageSourcePath(page)}`}
        />
      </div>
      <DocsBody>
        {isIndex ? (
          <>
            <ReferenceIndex />
            <div className="prose mt-12 border-t border-fd-border pt-8">
              <MDX
                components={getMDXComponents({
                  a: createRelativeLink(referenceSource, page),
                })}
              />
            </div>
          </>
        ) : (
          <MDX
            components={getMDXComponents({
              a: createRelativeLink(referenceSource, page),
            })}
          />
        )}
      </DocsBody>
    </DocsPage>
  );
}

export async function generateStaticParams() {
  return referenceSource.generateParams();
}

export async function generateMetadata(props: ReferencePageProps): Promise<Metadata> {
  const params = await props.params;
  const page = referenceSource.getPage(params.slug);
  if (!page) notFound();

  const isIndex = !params.slug || params.slug.length === 0;

  return {
    title: isIndex ? "API reference" : page.data.title,
    description: page.data.description,
    openGraph: {
      images: getPageImageUrl(page).url,
    },
  };
}
