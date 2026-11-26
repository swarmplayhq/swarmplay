// FitGirl's repacks as a catalog the launcher imports like RuTracker's PC forums (src/catalog.rs):
// the same report shape, a topic per repack, with its magnet, sizes, languages and cover.
//
// Every post is listed in the site's sitemaps with when it last changed, so a run reads only the
// pages that are new or changed since the catalog it starts from (the published one): the first
// run reads them all (about 9000), then a few a day. Politely: two pages at a time.
//
//   node fitgirl.mjs [--catalog dist/repacks/fitgirl-topics.json] [--limit 4000] [--minutes 100]
//
// Steam art and ProtonDB ratings are added after, by ../rutracker-list/steam.mjs --catalog.
import { readFile, rename, writeFile, mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const SITE = 'https://fitgirl-repacks.site/';
const AGENT = 'swarmplay-catalog (github.com/swarmplayhq/swarmplay)';
/** Topic IDs of their own (the launcher keys releases by ID): FitGirl's post ID plus this. */
export const ID_BASE = 9_000_000_000;
/** How pages are read: a catalog read differently is read again, whole. */
const PARSER = 2;

const option = (name, fallback) => { const i = process.argv.indexOf(name); return i < 0 ? fallback : process.argv[i + 1]; };

async function get(url, tries = 3) {
  for (let attempt = 1; ; attempt++) {
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(30_000), headers: { 'User-Agent': AGENT } });
      if (response.ok) return await response.text();
      if (response.status === 404) return null;
      throw new Error(`HTTP ${response.status}`);
    } catch (error) {
      if (attempt >= tries) throw new Error(`${url}: ${error.message}`);
      await new Promise(r => setTimeout(r, 2000 * attempt));
    }
  }
}

const decode = s => s.replace(/&#(\d+);/g, (_, n) => String.fromCodePoint(+n)).replace(/&#x([0-9a-f]+);/gi, (_, n) => String.fromCodePoint(parseInt(n, 16)))
  .replace(/&amp;/g, '&').replace(/&quot;/g, '"').replace(/&#039;|&apos;/g, "'").replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&nbsp;/g, ' ');
const strip = s => decode(s.replace(/<[^>]+>/g, '')).replace(/\s+/g, ' ').trim();

/** Every post URL with when it last changed, from the sitemap index. */
export async function listPosts() {
  const index = await get(`${SITE}sitemap_index.xml`);
  const maps = [...index.matchAll(/<loc>([^<]*post-sitemap\d*\.xml)<\/loc>/g)].map(m => m[1]);
  const posts = new Map();
  for (const map of maps) {
    const xml = await get(map);
    for (const m of xml.matchAll(/<loc>([^<]+)<\/loc>\s*<lastmod>([^<]+)<\/lastmod>/g)) {
      if (m[1] !== SITE) posts.set(m[1], m[2]);
    }
  }
  return posts;
}

/** "God of War – v1.0.12 (…) + Bonus OST" → "God of War"; "Stellaris: Ultimate Bundle, v4.6.0" →
 * "Stellaris". The game's name, as other catalogs and Steam call it; the edition, version and
 * extras go in `version`. */
export function splitTitle(title) {
  const cut = title.search(/\s+[–—]\s+|,\s*(?:v\s?[a-z]?\d|build\b|update\b|r\d)|\s+\+\s+|\s+\((?:v\s?\d|build)/i);
  let name = (cut < 0 ? title : title.slice(0, cut)).trim();
  const rest = (cut < 0 ? '' : title.slice(cut).replace(/^\s*[–—,]\s*/, '')).trim();
  const edition = name.match(/:?\s+(?:(?:Digital\s+)?(?:Ultimate|Deluxe|Complete|Gold|GOTY|Game of the Year|Definitive|Premium|Collector'?s|Enhanced|Anniversary|Standard|Special|Supporter|Legendary|Royal|Champion'?s|Super Deluxe)(?:\s+(?:Edition|Bundle|Pack|Collection))?|(?:Edition|Bundle))$/i);
  if (edition && edition.index > 0) name = name.slice(0, edition.index).trim();
  return { name, version: [edition?.[0].replace(/^:?\s+/, ''), rest].filter(Boolean).join(', ') };
}

/** "from 26 GB" / "1.2 GB" / "850 MB" → "26 GB". */
const size = s => (s.match(/[\d.,]+\s*(?:TB|GB|MB)/i) || [''])[0].replace(',', '.');

/** One repack's page as a topic, or null when it isn't one (news, digests, a page without a magnet). */
export function parsePost(html, url) {
  const id = +(html.match(/<article id="post-(\d+)"/) || [])[1];
  const magnet = (html.match(/href="(magnet:\?xt=urn:btih:[^"]+)"/i) || [])[1];
  const repackSize = (html.match(/Repack Size:\s*<strong>(.*?)<\/strong>/s) || [])[1];
  if (!id || !magnet || !repackSize) return null;
  const title = strip((html.match(/<h1 class="entry-title">(.*?)<\/h1>/s) || [])[1] || '');
  if (!title) return null;
  // A field's value: the bold text right after its label, or (Genres/Tags: links) what's on its
  // line; never more than a line's worth (a label with no line break after it would take the page).
  const field = name => {
    const bold = html.match(new RegExp(`${name}:\\s*<strong>([^<]{0,300})</strong>`));
    const line = bold || html.match(new RegExp(`${name}:\\s*((?:(?!<br|</p>|</div>)[\\s\\S]){0,600})`));
    return strip((line || [])[1] || '').slice(0, 300);
  };
  const { name, version } = splitTitle(title);
  const languages = field('Languages');
  // Which languages come as their own optional files (the "selective" download).
  const selective = [...new Set([...html.matchAll(/fg-selective-([a-z]+)\.bin/gi)].map(m => m[1].toLowerCase()))].sort();
  const cover = (html.match(/"thumbnailUrl":"([^"]+)"/) || [])[1] || (html.match(/<img[^>]+src="(https:\/\/[^"]+\.(?:jpe?g|png|webp))"/i) || [])[1];
  const published = (html.match(/<time class="entry-date" datetime="([^"]+)"/) || [])[1] || '';
  const companies = field('Compan(?:y|ies)');
  return {
    id: String(ID_BASE + id),
    // The name first, in the shape the launcher's name cleaner knows; the maker after it.
    title: `${name} [FitGirl Repack]${version ? ` [${version}]` : ''}`,
    url,
    size: size(decode(repackSize)),
    seeders: null, leechers: null,
    author: 'FitGirl',
    maker: 'FitGirl',
    magnet: decode(magnet),
    magnet_status: 'available',
    published_at: published,
    game_info: {
      source_url: url,
      name,
      version,
      genre: field('Genres/Tags').split(',')[0]?.trim() || '',
      developer: companies.split(',')[0]?.trim() || '',
      publisher: companies.split(',').slice(-1)[0]?.trim() || '',
      interface_languages: languages,
      audio_languages: languages,
      selective_languages: selective,
      original_size: size(field('Original Size')),
      image_urls: cover ? [cover.replace(/\\\//g, '/')] : [],
      platform: 'Windows',
      title_id: null,
    },
  };
}

async function main() {
  const path = resolve(option('--catalog', fileURLToPath(new URL('../../dist/repacks/fitgirl-topics.json', import.meta.url))));
  const limit = +option('--limit', 4000);
  const deadline = Date.now() + +option('--minutes', 100) * 60_000;
  let previous = { topics: [], seen: {} };
  try { previous = JSON.parse(await readFile(path, 'utf8')); } catch { /* first run */ }
  // Read with an older parser: everything again (its fields may be wrong).
  const fresh = previous.parser === PARSER;
  const byUrl = new Map(previous.topics.map(t => [t.url, fresh ? t : { ...t, lastmod: '' }]));
  // Pages already read and found not to be repacks, with their date then.
  const seen = fresh ? (previous.seen || {}) : {};
  const posts = await listPosts();
  const due = [...posts].filter(([url, mod]) => (byUrl.get(url)?.lastmod || seen[url]) !== mod)
    .sort((a, b) => b[1].localeCompare(a[1])).slice(0, limit);
  console.log(`${posts.size} posts, ${due.length} to read`);
  let done = 0, failed = 0;
  const worker = async () => {
    while (due.length && Date.now() < deadline) {
      const [url, mod] = due.shift();
      try {
        const html = await get(url);
        const topic = html && parsePost(html, url);
        if (topic) {
          // Steam's match stays unless the name changed.
          const old = byUrl.get(url);
          if (old?.steam && old.game_info?.name === topic.game_info.name) topic.steam = old.steam;
          topic.lastmod = mod;
          byUrl.set(url, topic);
          delete seen[url];
        } else {
          byUrl.delete(url);
          seen[url] = mod;
        }
      } catch (error) {
        failed++;
        console.warn(error.message);
      }
      if (++done % 200 === 0) console.log(`${done} read, ${byUrl.size} repacks`);
      await new Promise(r => setTimeout(r, 300));
    }
  };
  await Promise.all([worker(), worker()]);
  // Gone from the site: gone from the catalog.
  for (const url of byUrl.keys()) if (!posts.has(url)) byUrl.delete(url);
  const topics = [...byUrl.values()].sort((a, b) => a.id.localeCompare(b.id));
  const report = {
    source: SITE, platform: 'pc', collected_at: new Date().toISOString(), complete: true,
    scope: 'fitgirl', parser: PARSER, pages_visited: done, pages_remaining: due.length, failures: failed,
    topic_count: topics.length, translation: { language: 'en' }, topics, seen,
  };
  await mkdir(dirname(path), { recursive: true });
  await writeFile(`${path}.tmp`, JSON.stringify(report, null, 1) + '\n');
  await rename(`${path}.tmp`, path);
  console.log(`${topics.length} repacks in ${path} (${due.length} pages left for the next run, ${failed} failed)`);
}

if (import.meta.url === `file://${process.argv[1]}`) main().catch(error => { console.error(error); process.exit(1); });
