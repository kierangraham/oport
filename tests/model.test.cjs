const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const model = require('../plugin/Model.js');

// The plugin decodes whatever the real scanner emits.
const raw = execFileSync('./target/release/oport', ['watch', '--interval', '0.25', '--count', '2'], {encoding: 'utf8'});
const snapshots = raw.trim().split('\n').map(model.decode);
assert.equal(snapshots.length, 2);
const last = snapshots[1];
assert.ok(last.memory.total > 0 && last.memory.available <= last.memory.total);
assert.equal(last.memory.servers, last.servers.reduce((sum, s) => sum + s.memory, 0));

assert.throws(() => model.decode('garbage'));
assert.throws(() => model.decode('{"schema_version":2,"servers":[]}'));
const server = {
  id: '10-1', pid: 10, ports: [5173, 9229], url: 'http://localhost:5173', name: 'web', branch: 'main',
  framework: 'Vite', runtime: 'node', cwd: '/home/u/web', memory: 3 * 1024 ** 3, cpu: 12.34, uptime: 3720,
  processes: [], history: {memory: [], cpu: []}, agent: {label: 'Claude Code'}, leak: null, idle: false,
  idle_seconds: 0, cwd_missing: false, cleanup: false, protected: false
};
const snapshot = {...model.emptySnapshot(), servers: [server], memory: {total: 8 * 1024 ** 3, available: 2 * 1024 ** 3, servers: 3 * 1024 ** 3}};
assert.equal(model.decode(JSON.stringify(snapshot)).servers.length, 1);
assert.throws(() => model.decode(JSON.stringify({...snapshot, servers: [{...server, ports: []}]})));
assert.throws(() => model.decode(JSON.stringify({...snapshot, servers: [{...server, memory: -1}]})));

assert.equal(model.bytes(3 * 1024 ** 3), '3.0 GiB');
assert.equal(model.duration(3720), '1h 2m');
assert.equal(model.cpu(12.34), '12%');
assert.equal(model.cpu(0.25), '0.3%');
assert.equal(model.ports(server), ':5173 :9229');
assert.equal(model.portColor(5173), model.portColor(5173));
assert.equal(model.home('/home/u/web/app', '/home/u'), '~/web/app');
assert.equal(model.home('/srv/x', '/home/u'), '/srv/x');
assert.equal(model.detail(server), 'Vite  ·  Claude Code  ·  up 1h 2m');
assert.equal(model.detail({...server, leak: 'Grew 600 MiB in 10 min'}).split('  ·  ')[0], 'Grew 600 MiB in 10 min');
assert.equal(model.detail({...server, cwd_missing: true}).split('  ·  ')[0], 'folder deleted');

const shares = model.memoryShares(snapshot.memory);
assert.ok(Math.abs(shares.servers + shares.other + shares.free - 1) < 1e-9);
assert.equal(shares.free, 0.25);
assert.deepEqual(model.memoryShares({total: 0}), {servers: 0, other: 0, free: 1});

const stale = {...server, id: '11-1', cleanup: true};
const locked = {...server, id: '12-1', cleanup: true, protected: true};
const selected = model.cleanupSelection([server, stale, locked]);
assert.equal(model.selectedCount(selected, [server, stale, locked]), 1);

assert.match(model.tooltip(snapshot, false), /^1 dev server · 3.0 GiB\n:5173 :9229  web$/);
assert.equal(model.tooltip(snapshot, true), 'oport · waiting for the scanner');
console.log('model tests passed');
