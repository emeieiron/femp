import { readFileSync } from 'node:fs';
import { defineConfig } from 'astro/config';
import { satteri } from '@astrojs/markdown-satteri';
import { docLinks } from './src/lib/doc-links.mjs';

const fe = JSON.parse(readFileSync(new URL('./src/lib/fe.tmLanguage.json', import.meta.url), 'utf8'));

// GitHub Pages serves the project under /femp/. SITE_BASE=/ builds for a site root.
const base = process.env.SITE_BASE ?? '/femp/';
const repo = 'https://github.com/emeieiron/femp';

export default defineConfig({
  site: process.env.SITE_URL ?? 'https://emeieiron.github.io',
  base,
  trailingSlash: 'always',
  markdown: {
    processor: satteri({ hastPlugins: [docLinks({ base, repo })] }),
    shikiConfig: { theme: 'css-variables', langs: [fe] },
  },
});
