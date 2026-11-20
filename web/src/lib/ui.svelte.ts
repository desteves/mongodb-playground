// Shared UI state: theme, screen size, the mobile drawer, and a tiny client-side router.

export type Theme = 'auto' | 'light' | 'dark';

function load<T extends string>(key: string): T | null {
  try {
    return localStorage.getItem(key) as T | null;
  } catch {
    return null;
  }
}

function save(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* private mode: the choice just will not persist */
  }
}

const smallQuery = typeof matchMedia === 'function' ? matchMedia('(max-width: 860px)') : null;
const mediumQuery = typeof matchMedia === 'function' ? matchMedia('(max-width: 1240px)') : null;
const darkQuery = typeof matchMedia === 'function' ? matchMedia('(prefers-color-scheme: dark)') : null;

export const ui = $state({
  theme: (load<Theme>('theme') ?? 'auto') as Theme,
  prefersDark: darkQuery?.matches ?? true,
  small: smallQuery?.matches ?? false,
  /** Narrow desktop: three side-by-side panels get cramped next to the nav, so stack them by default. */
  medium: mediumQuery?.matches ?? false,
  drawer: false,
});

export const isDark = () => ui.theme === 'dark' || (ui.theme === 'auto' && ui.prefersDark);

export function setTheme(t: Theme) {
  ui.theme = t;
  save('theme', t);
}

/** Auto -> light -> dark -> auto, for the one-button toggle. */
export function cycleTheme() {
  setTheme(ui.theme === 'auto' ? 'light' : ui.theme === 'light' ? 'dark' : 'auto');
}

export function watchScreen(): () => void {
  const onSmall = () => (ui.small = smallQuery!.matches);
  const onDark = () => (ui.prefersDark = darkQuery!.matches);
  const onMedium = () => (ui.medium = mediumQuery!.matches);
  mediumQuery?.addEventListener('change', onMedium);
  smallQuery?.addEventListener('change', onSmall);
  darkQuery?.addEventListener('change', onDark);
  return () => {
    mediumQuery?.removeEventListener('change', onMedium);
    smallQuery?.removeEventListener('change', onSmall);
    darkQuery?.removeEventListener('change', onDark);
  };
}

// ---- routing ---------------------------------------------------------------------------------------------

export const route = $state({ path: typeof location === 'undefined' ? '/' : location.pathname });

const PAGES = ['/', '/docs/quickstart', '/docs/api', '/docs/mcp', '/docs/run-it-yourself', '/about'];

export const isPlayground = (p: string) => p === '/' || /^\/p\/[0-9A-Za-z-]+$/.test(p);
export const isKnownPath = (p: string) => PAGES.includes(p) || isPlayground(p);

export function navigate(to: string, push = true) {
  const url = new URL(to, location.origin);
  if (push) history.pushState({}, '', url.pathname + url.search + url.hash);
  route.path = url.pathname;
  ui.drawer = false;
  window.scrollTo(0, 0);
}

/** One click handler for the whole app: same-site links to known pages navigate without a reload. */
export function interceptLinks(e: MouseEvent) {
  if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
  const a = (e.target as Element | null)?.closest?.('a[href]') as HTMLAnchorElement | null;
  if (!a || a.target === '_blank' || a.hasAttribute('download')) return;
  const url = new URL(a.href, location.origin);
  if (url.origin !== location.origin || !isKnownPath(url.pathname)) return;
  e.preventDefault();
  navigate(url.pathname + url.search + url.hash);
}

if (typeof window !== 'undefined') {
  window.addEventListener('popstate', () => (route.path = location.pathname));
}
