// A read-only link's `silt` (ADR 0072): `db` answers reads from `docs`, the
// documents a link's reader may read as they stood where the link is, and
// refuses every write; every other capability is null. Queries follow
// `silt_proto::page::query`. Runs after `docs` is set.
function rank(v) {
  return v === undefined ? 0 : v === null ? 1 : typeof v === 'boolean' ? 2 : typeof v === 'number' ? 3 : typeof v === 'string' ? 4 : 5;
}
function compare(a, b) {
  var ra = rank(a), rb = rank(b);
  if (ra !== rb) return ra - rb;
  if (ra >= 2 && ra <= 4) return a < b ? -1 : a > b ? 1 : 0;
  var x = a === undefined ? '' : JSON.stringify(a), y = b === undefined ? '' : JSON.stringify(b);
  return x < y ? -1 : x > y ? 1 : 0;
}
function same(a, b) {
  if (a === b) return true;
  if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null || Array.isArray(a) !== Array.isArray(b)) return false;
  var ka = Object.keys(a), kb = Object.keys(b);
  if (ka.length !== kb.length) return false;
  for (var i = 0; i < ka.length; i++) {
    if (!Object.prototype.hasOwnProperty.call(b, ka[i]) || !same(a[ka[i]], b[ka[i]])) return false;
  }
  return true;
}
function field(data, name) {
  return data !== null && typeof data === 'object' && !Array.isArray(data) && Object.prototype.hasOwnProperty.call(data, name) ? data[name] : undefined;
}
function matches(data, w) {
  var f = field(data, w.field), order = f === undefined ? null : compare(f, w.value);
  switch (w.op) {
    case '==': return f !== undefined && same(f, w.value);
    case '!=': return !(f !== undefined && same(f, w.value));
    case '<': return order !== null && order < 0;
    case '<=': return order !== null && order <= 0;
    case '>': return order !== null && order > 0;
    case '>=': return order !== null && order >= 0;
    case 'in': return Array.isArray(w.value) && f !== undefined && w.value.some(function (v) { return same(v, f); });
    case 'contains': return Array.isArray(f) && f.some(function (v) { return same(v, w.value); });
    default: return false;
  }
}
function copy(value) { return value === undefined ? null : JSON.parse(JSON.stringify(value)); }
function found(collection, q) {
  var prefix = collection + '/', out = [];
  for (var path in docs) {
    if (!Object.prototype.hasOwnProperty.call(docs, path) || path.indexOf(prefix) !== 0) continue;
    var id = path.slice(prefix.length);
    if (id.indexOf('/') !== -1) continue;
    var data = docs[path];
    if ((q.where || []).every(function (w) { return matches(data, w); })) out.push({ id: id, path: path, data: copy(data) });
  }
  if (q.order_by != null) {
    out.sort(function (a, b) { return compare(field(a.data, q.order_by), field(b.data, q.order_by)); });
    if (q.descending) out.reverse();
  }
  return out.slice(0, Math.min(q.limit == null ? 1000 : q.limit, 1000));
}
function refused() {
  var e = new Error('a read-only link only reads');
  e.code = 'not_granted';
  return Promise.reject(e);
}
function once(read, fn) {
  var gone = false;
  setTimeout(function () { if (!gone) fn(read()); }, 0);
  return function () { gone = true; };
}
function doc(path) {
  var read = function () {
    return Object.prototype.hasOwnProperty.call(docs, path) ? { id: path.split('/').pop(), path: path, data: copy(docs[path]) } : null;
  };
  return Object.freeze({
    path: path,
    id: path.split('/').pop(),
    get: function () { return Promise.resolve(read()); },
    set: refused, update: refused, delete: refused, strReplace: refused,
    onSnapshot: function (fn) { return once(read, fn); }
  });
}
function collection(path, query) {
  query = query || { where: [], order_by: null, descending: false, limit: null };
  function also(change) {
    var next = { where: query.where.slice(), order_by: query.order_by, descending: query.descending, limit: query.limit };
    change(next);
    return collection(path, next);
  }
  var read = function () { return found(path, query); };
  return Object.freeze({
    path: path,
    doc: function (id) { return doc(path + '/' + id); },
    add: refused,
    where: function (f, op, value) { return also(function (q) { q.where.push({ field: f, op: op, value: value }); }); },
    orderBy: function (f, direction) { return also(function (q) { q.order_by = f; q.descending = direction === 'desc'; }); },
    limit: function (n) { return also(function (q) { q.limit = n; }); },
    get: function () { return Promise.resolve(read()); },
    onSnapshot: function (fn) { return once(read, fn); }
  });
}
var db = Object.freeze({ doc: doc, collection: function (path) { return collection(path); }, batch: refused });
window.silt = Object.freeze({
  version: '1',
  use: function (name) { return Promise.resolve(String(name) === 'db' ? db : null); },
  hot: Object.freeze({ data: null, snapshot: function () {}, ready: function (start) { start({}); } })
});
