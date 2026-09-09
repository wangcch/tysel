import { referenceSource } from "@/lib/source";
import { DocsLayout } from "fumadocs-ui/layouts/docs";
import { baseOptions } from "@/lib/layout.shared";

export default function Layout({ children }: LayoutProps<"/reference">) {
  return (
    <DocsLayout
      tree={referenceSource.getPageTree()}
      {...baseOptions()}
      nav={{ enabled: false, title: null }}
    >
      {children}
    </DocsLayout>
  );
}
