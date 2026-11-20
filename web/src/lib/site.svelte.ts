// Data every page may want, fetched once: limits and capabilities, the setup guide, templates and examples.
import { atlasUrl, examples as fetchExamples, guide as fetchGuide, limits as fetchLimits, type Example, type Guide, type Limits } from './api';
import { startTurnstile } from './turnstile';

export const site = $state<{ limits: Limits | null; guide: Guide | null; examples: Example[]; loaded: boolean }>({
  limits: null,
  guide: null,
  examples: [],
  loaded: false,
});

let started = false;

export function loadSite() {
  if (started) return;
  started = true;
  Promise.all([fetchLimits(), fetchGuide(), fetchExamples().catch(() => [] as Example[])]).then(([l, g, e]) => {
    site.limits = l;
    site.guide = g;
    site.examples = e;
    site.loaded = true;
    if (l?.turnstile?.enabled && l.turnstile.siteKey) startTurnstile(l.turnstile.siteKey);
  });
}

/** The Atlas sign-up link with UTM parameters; `placement` says where it was shown. */
export const registerUrl = (placement: string) => atlasUrl(site.guide, placement);
