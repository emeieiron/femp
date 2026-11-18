// Rewrites links in the Markdown under docs/ so they work on the site, while the files keep
// links that work on GitHub: `formats.md#cbor` becomes the formats page, and a path to any
// other repository file, such as `../tools/README.md`, becomes its page on GitHub.

import path from 'node:path/posix';

export function docHref(href, { base, repo, branch = 'main' }) {
  if (typeof href !== 'string' || /^([a-z]+:|#|\/)/i.test(href)) return undefined;
  const root = base.endsWith('/') ? base : `${base}/`;
  const [target, hash] = href.split('#');
  const fragment = hash ? `#${hash}` : '';
  const resolved = path.normalize(path.join('docs', target));
  if (resolved.startsWith('docs/') && resolved.endsWith('.md')) {
    const slug = resolved.slice('docs/'.length, -'.md'.length);
    return `${root}docs/${slug}/${fragment}`;
  }
  return `${repo}/blob/${branch}/${resolved}${fragment}`;
}

/** A Sätteri hast plugin applying `docHref` to every link. */
export function docLinks(options) {
  return {
    name: 'femp-doc-links',
    element: {
      filter: ['a'],
      visit(node, ctx) {
        const href = docHref(node.properties?.href, options);
        if (href !== undefined) ctx.setProperty(node, 'href', href);
      },
    },
  };
}
