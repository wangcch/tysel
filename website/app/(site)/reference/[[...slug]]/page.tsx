import { alternates } from "@/lib/i18n/seo";
import {
  getPageImageUrl,
  getPageMarkdownUrl,
  getPageSourcePath,
  referenceSource,
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
import { canonicalUrl, gitConfig } from "@/lib/shared";
import { StructuredData } from "@/components/seo/structured-data";
import { createArticleJsonLd } from "@/lib/seo";

type ReferencePageProps = PageProps<"/reference/[[...slug]]">;

const referenceIndexTitle = "API reference";
const referenceIndexDescription =
  "Exact interfaces — accepted values, defaults, side effects, and what each profile can still deny.";

export default async function Page(props: ReferencePageProps) {
  const params = await props.params;
  const page = referenceSource.getPage(params.slug);
  if (!page) notFound();

  const MDX = page.data.body;
  const markdownUrl = getPageMarkdownUrl(page).url;
  const isIndex = !params.slug || params.slug.length === 0;
  const title = isIndex ? referenceIndexTitle : page.data.title;
  const description = isIndex ? referenceIndexDescription : page.data.description;
  const jsonLd = createArticleJsonLd(
    page,
    {
      name: referenceIndexTitle,
      url: "/reference",
    },
    { title, description },
  );

  return (
    <>
      <StructuredData data={jsonLd} />
      <DocsPage toc={isIndex ? [] : page.data.toc} full={page.data.full}>
        <DocsTitle>{title}</DocsTitle>
        <DocsDescription className="mb-0">{description}</DocsDescription>
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
    </>
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
  const title = isIndex ? referenceIndexTitle : page.data.title;
  const description = isIndex ? referenceIndexDescription : page.data.description;

  return {
    title,
    description,
    alternates: alternates(page.url),
    openGraph: {
      url: canonicalUrl(page.url),
      siteName: "Tysel",
      locale: "en_US",
      type: "article",
      images: getPageImageUrl(page).url,
    },
  };
}
