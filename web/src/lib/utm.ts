/** The UTM parameters on every link back to mongodb.com (any subdomain), in this order.
 *  The server sends the same set in `GET /api/v1/guide` (atlasRegister.utm); these are the fallback. */
export const UTM_KEYS = ['utm_campaign', 'utm_source', 'utm_medium', 'utm_content', 'utm_term'] as const;

export const UTM_DEFAULT: Record<string, string> = {
  utm_campaign: 'devrel',
  utm_source: 'third-party-content',
  utm_medium: 'cta',
  utm_content: 'mongodb-playground',
  utm_term: 'diana.esteves',
};

/** Adds the UTM parameters to a link to mongodb.com (`mongodb.com` or any `*.mongodb.com`).
 *  Other links come back unchanged; other query parameters and the fragment are kept. */
export function withUtm(href: string, utm: Record<string, string> = UTM_DEFAULT): string {
  let url: URL;
  try {
    url = new URL(href);
  } catch {
    return href;
  }
  const host = url.hostname.toLowerCase();
  if (host !== 'mongodb.com' && !host.endsWith('.mongodb.com')) return href;
  for (const k of UTM_KEYS) url.searchParams.delete(k);
  for (const k of UTM_KEYS) url.searchParams.set(k, utm[k] ?? UTM_DEFAULT[k]);
  return url.toString();
}
