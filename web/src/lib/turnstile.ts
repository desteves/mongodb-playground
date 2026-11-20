// Cloudflare Turnstile: a browser that passes it gets a one-hour session and a higher rate limit.
// Nothing is blocked without it, so agents and curl keep working on the basic tier.

type Session = { token: string; exp: number };
let session: Session | null = null;
let started = false;

/** Headers to add to API calls: the session, when we have a fresh one. */
export function sessionHeader(): Record<string, string> {
  return session && session.exp * 1000 > Date.now() + 30_000 ? { 'x-playground-session': session.token } : {};
}

declare global {
  interface Window {
    turnstile?: {
      render: (el: HTMLElement, opts: Record<string, unknown>) => string;
      reset: (id?: string) => void;
    };
  }
}

function loadScript(): Promise<void> {
  return new Promise((resolve, reject) => {
    if (window.turnstile) return resolve();
    const s = document.createElement('script');
    s.src = 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit';
    s.async = true;
    s.onload = () => resolve();
    s.onerror = () => reject(new Error('Turnstile script failed to load'));
    document.head.appendChild(s);
  });
}

/** Start the (mostly invisible) challenge. Safe to call more than once. */
export async function startTurnstile(siteKey: string): Promise<void> {
  if (started || typeof window === 'undefined') return;
  started = true;
  try {
    await loadScript();
  } catch {
    started = false;
    return; // blocked by an extension or offline: stay on the basic tier
  }
  const holder = document.createElement('div');
  holder.style.cssText = 'position:fixed;right:8px;bottom:8px;z-index:50';
  document.body.appendChild(holder);
  const widget = window.turnstile!.render(holder, {
    sitekey: siteKey,
    appearance: 'interaction-only', // only visible if Cloudflare needs a click
    callback: async (token: string) => {
      try {
        const res = await fetch('/api/v1/verify', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ token }),
        });
        const data = await res.json();
        if (res.ok && data.session) {
          session = { token: data.session, exp: data.expiresAt };
          // Tokens are single-use: ask for a fresh one shortly before the session ends.
          setTimeout(() => window.turnstile?.reset(widget), Math.max(60_000, (data.expiresInSeconds - 300) * 1000));
        }
      } catch {
        /* stay on the basic tier */
      }
    },
    'error-callback': () => undefined,
  });
}
