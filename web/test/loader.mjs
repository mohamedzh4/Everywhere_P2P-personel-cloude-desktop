// Node resolve hook: maps the page's import-map prefixes to web/vendor, so
// the browser modules run unchanged under `node`.
import { pathToFileURL } from 'node:url';
import { resolve as resolvePath, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const vendor = resolvePath(dirname(fileURLToPath(import.meta.url)), '../vendor');
const MAP = {
  '@noble/curves/': pathToFileURL(vendor + '/noble-curves/').href,
  '@noble/hashes/': pathToFileURL(vendor + '/noble-hashes/').href,
};

export async function resolve(specifier, context, next) {
  for (const [prefix, target] of Object.entries(MAP)) {
    if (specifier.startsWith(prefix)) return next(target + specifier.slice(prefix.length), context);
  }
  return next(specifier, context);
}
