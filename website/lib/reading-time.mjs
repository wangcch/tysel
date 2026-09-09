// Approximate prose reading time: 400 Han characters/minute, 220 other words/minute.
export function estimateReadingMinutes(raw) {
  const body = raw.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, '')
    .replace(/```[\s\S]*?```/g, ' ')
    .replace(/!\[[^\]]*\]\([^)]+\)/g, ' ')
    .replace(/\[([^\]]*)\]\([^)]+\)/g, '$1');
  const han = (body.match(/\p{Script=Han}/gu) ?? []).length;
  const words = (body.replace(/\p{Script=Han}/gu, ' ').match(/[\p{L}\p{N}]+(?:['’-][\p{L}\p{N}]+)*/gu) ?? []).length;
  return Math.max(1, Math.round(han / 400 + words / 220));
}
