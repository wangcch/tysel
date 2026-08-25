export const appName = "Tysel";
export const siteUrl = "https://tysel.dev";
export const docsRoute = "/docs";
export const referenceRoute = "/reference";
export const docsImageRoute = "/og/docs";
export const referenceImageRoute = "/og/reference";
export const docsContentRoute = "/llms.mdx/docs";
export const referenceContentRoute = "/llms.mdx/reference";

export const gitConfig = {
  user: "wangcch",
  repo: "tysel",
  branch: "main",
};

export const githubUrl = `https://github.com/${gitConfig.user}/${gitConfig.repo}`;

export function absoluteUrl(pathname = "/") {
  const path = `/${pathname}`.replace(/\/{2,}/g, "/").replace(/\/$/, "");
  return `${siteUrl}${path || "/"}`;
}

export function canonicalUrl(pathname = "/") {
  const url = absoluteUrl(pathname);
  return url === `${siteUrl}/` ? url : `${url}/`;
}
