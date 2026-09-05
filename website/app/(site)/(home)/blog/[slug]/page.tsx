import { notFound } from "next/navigation";
import { blog } from "@/lib/source";
import BlogArticle, { blogMetadata } from "@/components/blog/article";
import { alternates } from "@/lib/i18n/seo";
export default async function Page(props: PageProps<"/blog/[slug]">) {
  const page = blog.getPage([(await props.params).slug]);
  if (!page) notFound();
  return <BlogArticle page={page} />;
}
export function generateStaticParams() { return blog.getPages().map(page => ({ slug: page.slugs[0] })); }
export async function generateMetadata(props: PageProps<"/blog/[slug]">) {
  const page = blog.getPage([(await props.params).slug]);
  if (!page) notFound();
  const metadata = blogMetadata(page);
  return { ...metadata, alternates: { ...metadata.alternates, ...alternates(page.url) } };
}
