// Seeders and leechers of every repack in a catalog, from a public tracker's scrape (BEP 15, UDP):
// the launcher ranks releases and says how easy they are to get by them.
//
//   node seeders.mjs [--catalog dist/repacks/fitgirl-topics.json]
import { readFile, rename, writeFile } from 'node:fs/promises';
import { createSocket } from 'node:dgram';
import { randomBytes } from 'node:crypto';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const TRACKERS = [['tracker.opentrackr.org', 1337], ['open.stealth.si', 80], ['tracker.torrent.eu.org', 451]];
const BATCH = 70; // what fits a scrape packet

const option = (name, fallback) => { const i = process.argv.indexOf(name); return i < 0 ? fallback : process.argv[i + 1]; };

function exchange(socket, host, port, packet, txid, timeout = 8000) {
  return new Promise((done, fail) => {
    const timer = setTimeout(() => { socket.off('message', on); fail(new Error('timeout')); }, timeout);
    const on = msg => {
      if (msg.length >= 8 && msg.readUInt32BE(4) === txid) { clearTimeout(timer); socket.off('message', on); done(msg); }
    };
    socket.on('message', on);
    socket.send(packet, port, host);
  });
}

/** Scrape `hashes` (hex) from one tracker: hash → [seeders, leechers]. */
async function scrape(host, port, hashes) {
  const socket = createSocket('udp4');
  try {
    let tx = randomBytes(4).readUInt32BE();
    const connect = Buffer.alloc(16);
    connect.writeBigUInt64BE(0x41727101980n, 0); connect.writeUInt32BE(0, 8); connect.writeUInt32BE(tx, 12);
    const reply = await exchange(socket, host, port, connect, tx);
    const id = reply.readBigUInt64BE(8);
    const out = new Map();
    for (let i = 0; i < hashes.length; i += BATCH) {
      const batch = hashes.slice(i, i + BATCH);
      tx = randomBytes(4).readUInt32BE();
      const packet = Buffer.alloc(16 + 20 * batch.length);
      packet.writeBigUInt64BE(id, 0); packet.writeUInt32BE(2, 8); packet.writeUInt32BE(tx, 12);
      batch.forEach((h, j) => Buffer.from(h, 'hex').copy(packet, 16 + 20 * j));
      const res = await exchange(socket, host, port, packet, tx);
      if (res.readUInt32BE(0) !== 2) throw new Error('scrape refused');
      batch.forEach((h, j) => {
        const at = 8 + 12 * j;
        if (res.length >= at + 12) out.set(h, [res.readUInt32BE(at), res.readUInt32BE(at + 8)]);
      });
    }
    return out;
  } finally { socket.close(); }
}

async function main() {
  const path = resolve(option('--catalog', fileURLToPath(new URL('../../dist/repacks/fitgirl-topics.json', import.meta.url))));
  const report = JSON.parse(await readFile(path, 'utf8'));
  const hashOf = t => (t.magnet.match(/btih:([a-f0-9]{40})/i) || [])[1]?.toLowerCase();
  const hashes = [...new Set(report.topics.map(hashOf).filter(Boolean))];
  // The most anyone reports: trackers each see part of the swarm.
  const best = new Map();
  for (const [host, port] of TRACKERS) {
    try {
      for (const [h, [s, l]] of await scrape(host, port, hashes)) {
        const [bs, bl] = best.get(h) || [0, 0];
        best.set(h, [Math.max(bs, s), Math.max(bl, l)]);
      }
      console.log(`${host}: ${hashes.length} scraped`);
    } catch (error) { console.warn(`${host}: ${error.message}`); }
  }
  if (!best.size) throw new Error('no tracker answered');
  const now = new Date().toISOString();
  for (const t of report.topics) {
    const counts = best.get(hashOf(t));
    if (counts) { [t.seeders, t.leechers] = counts; t.peers_observed = now; }
  }
  report.refreshed_at = now;
  await writeFile(`${path}.tmp`, JSON.stringify(report, null, 1) + '\n');
  await rename(`${path}.tmp`, path);
  const seeded = report.topics.filter(t => t.seeders > 0).length;
  console.log(`${seeded} of ${report.topics.length} repacks seeded`);
}

main().catch(error => { console.error(error.message); process.exit(1); });
