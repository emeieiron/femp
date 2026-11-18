import { defineCollection } from 'astro:content';
import { glob } from 'astro/loaders';

// The pages are the Markdown files in the repository's docs/, so they read the same on GitHub.
const docs = defineCollection({
  loader: glob({ pattern: '*.md', base: '../docs' }),
});

export const collections = { docs };
