// Preserve route tokens and link targets, while allowing visible labels to change.
export function navigationIdentity(entry) {
  if (typeof entry !== 'string') return entry;
  if (/^---.+---$/.test(entry)) return '---separator---';
  const link = entry.match(/^\[([^\]]+)\]\(([^)]+)\)$/);
  return link ? `[](${link[2]})` : entry;
}
export function localizeNavigation(metadata, locale, paths) {
  return { ...metadata, ...(metadata.pages && { pages: metadata.pages.map(entry => {
    const link = typeof entry === 'string' && entry.match(/^\[([^\]]+)\]\(([^)]+)\)$/);
    if (!link || !paths.has(link[2].split(/[?#]/)[0].replace(/\/$/, '') || '/')) return entry;
    return `[${link[1]}](/${locale}${link[2]})`;
  }) }) };
}
