export const repo = 'https://github.com/emeieiron/femp';

/** A path under the site's base, which is `/femp/` on GitHub Pages. */
export function url(path = ''): string {
  const base = import.meta.env.BASE_URL.replace(/\/?$/, '/');
  return base + path.replace(/^\//, '');
}

export interface NavItem {
  slug: string;
  label: string;
}

export interface NavGroup {
  label: string;
  items: NavItem[];
}

/** The sidebar, in reading order. Every page in docs/ must appear here. */
export const nav: NavGroup[] = [
  {
    label: 'Start',
    items: [
      { slug: 'overview', label: 'Overview' },
      { slug: 'installation', label: 'Installation' },
      { slug: 'quick-start', label: 'Quick start' },
      { slug: 'fe-and-femp', label: 'FeMP and Fe' },
    ],
  },
  {
    label: 'Build',
    items: [
      { slug: 'project-layout', label: 'Projects and modules' },
      { slug: 'bindings', label: 'Outputs and bindings' },
      { slug: 'source-generators', label: 'Source generators' },
    ],
  },
  {
    label: 'Verify',
    items: [
      { slug: 'testing', label: 'Testing' },
      { slug: 'property-testing', label: 'Property testing' },
      { slug: 'differential-testing', label: 'Differential testing' },
      { slug: 'contracts', label: 'Contracts' },
      { slug: 'proving', label: 'Proving' },
    ],
  },
  {
    label: 'Reference',
    items: [
      { slug: 'cli', label: 'Command line' },
      { slug: 'attributes', label: 'Attributes' },
      { slug: 'architecture', label: 'Architecture' },
      { slug: 'adding-a-platform', label: 'Adding a platform' },
    ],
  },
];

export const pages: NavItem[] = nav.flatMap((group) => group.items);
