"use client";

import { usePathname } from "next/navigation";
import { useEffect, useLayoutEffect, useRef } from "react";

/**
 * Next.js keeps scroll when it thinks the new page segment is already in view.
 * Docs layouts with sticky sidebar/TOC often trip that heuristic, so article
 * navigations leave you mid-page. Force the document to the top on path changes.
 * Hash deep-links and browser back/forward keep native scroll restoration.
 */
export function ScrollToTop() {
  const pathname = usePathname();
  const fromHistory = useRef(false);

  useEffect(() => {
    const onPopState = () => {
      fromHistory.current = true;
    };
    window.addEventListener("popstate", onPopState);
    return () => window.removeEventListener("popstate", onPopState);
  }, []);

  useLayoutEffect(() => {
    if (fromHistory.current) {
      fromHistory.current = false;
      return;
    }
    if (window.location.hash) return;
    window.scrollTo({ top: 0, left: 0, behavior: "auto" });
  }, [pathname]);

  return null;
}
