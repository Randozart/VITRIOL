// PROVENANCE: original test oracle using this repository's Caduceus
// (Apache-2.0 OR MIT). Executes the canonical JavaScript policy without copying it.
import { writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import {
  candidateOrder, shouldRetry, cappedParkMs, pruneParked, giveUpMessage,
} from '../../officina/scripts/caduceus-router-lib.mjs';

const orders = [];
function order(requested, fallbacks, parked, nowMs = 1000) {
  const input = { requested, fallbacks, state: { parked }, nowMs };
  orders.push({ requested, fallbacks, parked, nowMs, expected: candidateOrder(input) });
}
order(' a ', ['b', 'a', '', ' c ', 'b '], {});
order('a', ['b', 'c', 'd'], { a: 1001, b: 1000, c: 999, d: 1002 });
order(' a ', ['a', ' b '], { ' a ': 9000 });
order('模型/α', ['模型/α', 'β', '😀'], {});
for (const code of [9, 10, 11, 12, 13, 32, 160, 0x1680, ...Array.from({ length: 11 }, (_, i) => 0x2000 + i), 0x2028, 0x2029, 0x202f, 0x205f, 0x3000, 0xfeff, 0x85, 0x180e, 0x200b]) {
  const c = String.fromCodePoint(code);
  order(`${c}a${c}`, ['a', `${c}`, 'b'], {});
}
let seed = 7301;
function next() {
  seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
  return seed;
}
const names = ['a', ' b ', 'a ', '', '模型', '\ufeffc\ufeff', '\u0085a', '?'];
for (let i = 0; i < 64; i++) {
  const requested = names[next() % names.length];
  const fallbacks = Array.from({ length: 8 }, () => names[next() % names.length]);
  const parked = Object.fromEntries(['a', 'b', 'c', '模型', '?'].map(id => [id, 999 + next() % 3]));
  order(requested, fallbacks, parked);
}
const retries = Array.from({ length: 600 }, (_, status) => ({
  status, expected: shouldRetry({ status, forwarded: false, timedOut: false }),
}));
const caps = [];
for (const hint of [null, -1, 0, 999.9, 1000.9, 2000.5, 10000000]) {
  for (const max of [null, -1, 0, 500, 900000]) {
    caps.push({ hint, max, expected: cappedParkMs(hint, max) ?? null });
  }
}
const states = [
  { parked: {}, reasons: {} },
  { parked: { '?': 9000, a: 1001, b: 1000, c: 999 }, reasons: { a: 'quota', c: 'expired', orphan: 'unused' } },
  { parked: { ' a ': 1002, 模型: 1003 }, reasons: { 模型: 'limit' } },
];
const prunes = states.map(state => ({ state, nowMs: 1000, expected: pruneParked(state, 1000) }));
const reports = [];
for (const parked of [{}, { '?': 99999 }, { a: 1000 }, { a: 1001 }, { a: 2499 }, { a: 2500 }, { a: 4000, b: 2300 }, { '?': 1500, a: 10000 }]) {
  reports.push({ tried: 2, parked, nowMs: 1000, expected: giveUpMessage({ tried: 2, parked, nowMs: 1000 }) });
}
// Preserve the source helper's zero-timestamp truthiness boundary.
reports.push({ tried: 2, parked: { a: 0 }, nowMs: -1, expected: giveUpMessage({ tried: 2, parked: { a: 0 }, nowMs: -1 }) });
const output = fileURLToPath(new URL('../testdata/caduceus-policy.json', import.meta.url));
writeFileSync(output, `${JSON.stringify({ schema: 1, orders, retries, caps, prunes, reports }, null, 2)}\n`);
console.log(JSON.stringify({ orders: orders.length, retries: retries.length, caps: caps.length, prunes: prunes.length, reports: reports.length }));
