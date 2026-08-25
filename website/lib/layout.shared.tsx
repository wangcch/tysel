import type { BaseLayoutProps } from "fumadocs-ui/layouts/shared";
import { Wordmark } from "@/components/wordmark";
import { githubUrl } from "./shared";

/** Shared fumadocs layout props. Site chrome (nav links) lives in SiteHeader. */
export function baseOptions(): BaseLayoutProps {
  return {
    nav: {
      title: <Wordmark className="h-6" />,
      url: "/",
    },
    githubUrl,
    links: [],
  };
}
