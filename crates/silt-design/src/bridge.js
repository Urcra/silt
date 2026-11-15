// The frame's side of the bridge (ADR 0072): `silt.use(name)` asks the page
// holding the frame for a capability, and every call goes there by message,
// where it's checked against what the version declared and what the viewer
// allowed. Runs inside the first script, after `to` (the holding page's
// origin) and `boot` (what the holding page kept for this viewer) are set.
var seq = 0, waiting = {}, heard = {}, snapshot = null;
function ask(cap, method, args) {
  return new Promise(function (ok, no) {
    var id = ++seq;
    waiting[id] = [ok, no];
    parent.postMessage({ silt: 'call', id: id, cap: cap, method: method, args: args === undefined ? null : args }, to);
  });
}
function refusal(error) {
  var e = new Error((error && error.message) || 'refused');
  e.code = (error && error.code) || 'refused';
  return e;
}
function listen(cap, topic, fn) {
  var key = cap + ':' + topic;
  (heard[key] = heard[key] || []).push(fn);
  return function () {
    heard[key] = (heard[key] || []).filter(function (other) { return other !== fn; });
  };
}
addEventListener('message', function (e) {
  var d = e.data;
  if (e.source !== parent || !d) return;
  if (d.silt === 'reply' && waiting[d.id]) {
    var w = waiting[d.id];
    delete waiting[d.id];
    if (d.ok) w[0](d.value); else w[1](refusal(d.error));
  }
  if (d.silt === 'event') {
    var list = heard[d.cap + ':' + d.topic] || [];
    for (var i = 0; i < list.length; i++) { try { list[i](d.data); } catch (x) {} }
  }
  if (d.silt === 'snapshot') {
    var data = null;
    try { data = snapshot ? snapshot() : null; } catch (x) {}
    try { parent.postMessage({ silt: 'snapshotted', data: data }, to); } catch (x) {}
  }
});

// Per-viewer storage: the holding page's own storage keeps it for this
// viewer and artifact, as the frame's opaque origin has none.
function storage(kind, initial) {
  var items = {};
  for (var k in initial || {}) if (Object.prototype.hasOwnProperty.call(initial, k)) items[k] = String(initial[k]);
  function save() { parent.postMessage({ silt: 'storage', kind: kind, items: items }, to); }
  var api = {
    getItem: function (key) { key = String(key); return Object.prototype.hasOwnProperty.call(items, key) ? items[key] : null; },
    setItem: function (key, value) { items[String(key)] = String(value); save(); },
    removeItem: function (key) { delete items[String(key)]; save(); },
    clear: function () { items = {}; save(); },
    key: function (n) { var keys = Object.keys(items); return n >= 0 && n < keys.length ? keys[n] : null; }
  };
  Object.defineProperty(api, 'length', { get: function () { return Object.keys(items).length; } });
  return new Proxy(api, {
    get: function (target, name) { return name in target ? target[name] : target.getItem(name); },
    set: function (target, name, value) { target.setItem(name, value); return true; },
    deleteProperty: function (target, name) { target.removeItem(name); return true; },
    has: function (target, name) { return name in target || Object.prototype.hasOwnProperty.call(items, name); },
    ownKeys: function () { return Object.keys(items); },
    getOwnPropertyDescriptor: function (target, name) {
      return Object.prototype.hasOwnProperty.call(items, name) ? { value: items[name], enumerable: true, configurable: true } : undefined;
    }
  });
}
try {
  var kept = boot.storage || {};
  Object.defineProperty(window, 'localStorage', { value: storage('local', kept.local), configurable: true });
  Object.defineProperty(window, 'sessionStorage', { value: storage('session', kept.session), configurable: true });
} catch (x) {}

function namespace(cap, methods) {
  var ns = {};
  for (var i = 0; i < methods.length; i++) {
    (function (method) { ns[method] = function (args) { return ask(cap, method, args); }; })(methods[i]);
  }
  ns.on = function (topic, fn) { return listen(cap, topic, fn); };
  return Object.freeze(ns);
}
// `db` as a page writes it (ADR 0072): documents at even paths, in
// collections at odd ones, read once or watched.
function database(ns) {
  function watch(request, fn) {
    var id = null, gone = false;
    var off = ns.on('snapshot', function (d) { if (d && d.watch === id) fn(d.data); });
    ns.watch({ request: request }).then(function (answer) {
      if (gone) { ns.unwatch({ watch: answer.watch }); return; }
      id = answer.watch;
      fn(answer.data);
    }, function () {});
    return function () { gone = true; off(); if (id !== null) ns.unwatch({ watch: id }); };
  }
  function doc(path) {
    return Object.freeze({
      path: path,
      id: path.split('/').pop(),
      get: function () { return ns.get({ path: path }); },
      set: function (value) { return ns.set({ path: path, value: value }); },
      update: function (fields) { return ns.update({ path: path, fields: fields }); },
      delete: function () { return ns['delete']({ path: path }); },
      strReplace: function (field, old, now) { return ns.str_replace({ path: path, field: field, old: old, new: now }); },
      onSnapshot: function (fn) { return watch({ op: 'get', path: path }, fn); }
    });
  }
  function collection(path, query) {
    query = query || { where: [], order_by: null, descending: false, limit: null };
    function also(change) {
      var next = { where: query.where.slice(), order_by: query.order_by, descending: query.descending, limit: query.limit };
      change(next);
      return collection(path, next);
    }
    return Object.freeze({
      path: path,
      doc: function (id) { return doc(path + '/' + id); },
      add: function (value) {
        var id = Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
        return doc(path + '/' + id).set(value).then(function () { return doc(path + '/' + id); });
      },
      where: function (field, op, value) { return also(function (q) { q.where.push({ field: field, op: op, value: value }); }); },
      orderBy: function (field, direction) { return also(function (q) { q.order_by = field; q.descending = direction === 'desc'; }); },
      limit: function (n) { return also(function (q) { q.limit = n; }); },
      get: function () { return ns.query({ collection: path, query: query }); },
      onSnapshot: function (fn) { return watch({ op: 'query', collection: path, query: query }, fn); }
    });
  }
  return Object.freeze({
    doc: doc,
    collection: function (path) { return collection(path); },
    batch: function (writes) { return ns.batch({ writes: writes }); }
  });
}
// The rest as a page writes them, each call one message to the holding page.
function artifactClient(ns) {
  return Object.freeze({
    publish: function (html) { return ns.publish({ html: String(html) }); },
    version: function () { return ns.version().then(function (v) { return v.version; }); }
  });
}
function userClient(ns) {
  return Object.freeze({
    id: function () { return ns.id(); },
    me: function () { return ns.me(); },
    isOwner: function () { return ns.isOwner(); },
    canEdit: function () { return ns.canEdit(); },
    can: function (name) { return ns.can({ name: String(name) }); },
    profiles: function (ids) { return ns.profiles({ ids: Array.prototype.slice.call(ids || []) }); },
    search: function (query) { return ns.search({ query: String(query || '') }); }
  });
}
function sampleClient(ns) {
  function sample(input, opts) {
    opts = opts || {};
    var ask = 's' + Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
    var off = typeof opts.onText === 'function'
      ? ns.on('text', function (d) {
          if (!d || d.ask !== ask) return;
          var before = sample.last[ask] || '';
          sample.last[ask] = d.text;
          try { opts.onText({ text: d.text, delta: d.text.slice(before.length) }); } catch (x) {}
        })
      : function () {};
    // The page's tools run here, when the agent calls one; only their names,
    // descriptions and input schemas leave the page.
    var tools = Array.isArray(opts.tools) ? opts.tools : [];
    var runs = {};
    tools.forEach(function (tool) { runs[tool.name] = tool.execute; });
    var offTools = tools.length
      ? ns.on('tool', function (d) {
          if (!d || d.ask !== ask) return;
          var run = runs[d.name];
          Promise.resolve()
            .then(function () {
              if (typeof run !== 'function') throw new Error('no tool is named ' + d.name);
              return run(d.input);
            })
            .then(
              function (value) { return ns.toolResult({ ask: ask, call: d.call, ok: true, value: value === undefined ? null : value }); },
              function (e) { return ns.toolResult({ ask: ask, call: d.call, ok: false, error: String((e && e.message) || e) }); }
            )
            .catch(function () {});
        })
      : function () {};
    var done = function () { off(); offTools(); delete sample.last[ask]; };
    var asked = { input: input, ask: ask };
    if (opts.images) asked.images = Array.prototype.slice.call(opts.images);
    if (opts.modelTier || opts.model) asked.model = { model_tier: opts.modelTier || null, model: opts.model || null };
    if (tools.length) {
      asked.tools = tools.map(function (tool) {
        return { name: String(tool.name), description: String(tool.description || ''), inputSchema: tool.inputSchema };
      });
    }
    return ns.ask(asked).then(function (answer) { done(); return answer; }, function (e) { done(); throw e; });
  }
  sample.last = {};
  sample.json = function (input, opts) {
    return sample(input, opts).then(function (answer) {
      var text = String(answer.text).trim().replace(/^```(?:json)?\s*/, '').replace(/```$/, '').trim();
      return JSON.parse(text);
    });
  };
  sample.limits = function () { return ns.limits(); };
  return Object.freeze(sample);
}
function assetsClient(ns) {
  return Object.freeze({
    upload: function (blob, name) { return ns.upload({ blob: blob, name: name }); },
    list: function () { return ns.list(); },
    delete: function (id) { return ns['delete']({ id: String(id) }); }
  });
}
function filesClient(ns) {
  function list(opts) { opts = opts || {}; return ns.list({ path: opts.path || '', recursive: !!opts.recursive, cursor: opts.cursor || null }); }
  function read(path, opts) { opts = opts || {}; return ns.read({ path: String(path), at: opts.at || null }); }
  return Object.freeze({
    list: list,
    read: read,
    download: function (path) { return ns.download({ path: String(path) }); },
    // Read again every few seconds, as the hosted app hears of no change;
    // `fn` hears of the file whenever its revision changes.
    watch: function (path, fn, opts) {
      var every = Math.max(2000, (opts && opts.every) || 5000);
      var last = null;
      var stopped = false;
      function look() {
        if (stopped) return;
        read(path).then(function (now) {
          if (stopped || now.revision === last) return;
          last = now.revision;
          try { fn(now); } catch (x) {}
        }, function () {});
      }
      look();
      var timer = setInterval(look, every);
      return function () { stopped = true; clearInterval(timer); };
    },
    // The thread's files as tools for `sample`, so the agent answering a
    // page's question can look through them itself.
    asSampleTools: function () {
      return Promise.resolve([
        {
          name: 'list_files',
          description: "Lists the thread's files under a folder, the whole tree when no path is given",
          inputSchema: { type: 'object', properties: { path: { type: 'string' }, recursive: { type: 'boolean' } } },
          execute: function (input) { return list({ path: input && input.path, recursive: input && input.recursive }); }
        },
        {
          name: 'read_file',
          description: "Reads one of the thread's files as text",
          inputSchema: { type: 'object', properties: { path: { type: 'string' } }, required: ['path'] },
          execute: function (input) {
            return read(String((input && input.path) || '')).then(function (file) {
              return { path: input.path, text: String(file.text).slice(0, 60000), size: file.size };
            });
          }
        }
      ]);
    }
  });
}
function dictationClient(ns) {
  return Object.freeze({
    start: function () { return ns.start(); },
    stop: function () { return ns.stop(); },
    onPartial: function (fn) { return ns.on('partial', fn); },
    onFinal: function (fn) { return ns.on('final', fn); },
    onState: function (fn) { return ns.on('state', fn); }
  });
}
function ttsClient(ns) {
  function lines(text, opts) {
    var given = Array.isArray(text) ? text : [Object.assign({ text: String(text) }, opts || {})];
    return given.map(function (line) { return typeof line === 'string' ? { text: line } : line; });
  }
  return Object.freeze({
    speak: function (text, opts) { return ns.speak({ lines: lines(text, opts) }); },
    add: function (text, opts) { return ns.add({ lines: lines(text, opts) }); },
    stop: function () { return ns.stop(); },
    pause: function () { return ns.pause(); },
    resume: function () { return ns.resume(); },
    setRate: function (rate) { return ns.setRate({ rate: rate }); },
    onState: function (fn) { return ns.on('state', fn); },
    onLine: function (fn) { return ns.on('line', fn); }
  });
}
function mcpClient(ns) {
  function call(server, tool, input) { return ns.callTool({ server: server, tool: tool, input: input || {} }); }
  return Object.freeze({
    callTool: call,
    describeTool: function (server, tool) { return ns.describeTool({ server: server, tool: tool }); },
    listTools: function (server) { return ns.listTools({ server: server }); },
    // Reads a tool that only reads now and then again, no faster than every 30 s.
    watchTool: function (server, tool, input, handler, opts) {
      var every = Math.max(30000, (opts && opts.refetchInterval) || 0), timer = null, gone = false;
      function read() {
        ns.callTool({ server: server, tool: tool, input: input || {}, read: true }).then(function (result) {
          if (!gone) handler({ type: 'data', data: result, cache: { storedAt: Date.now() } });
        }, function (error) { if (!gone) handler({ type: 'error', error: error }); });
        if (!gone && opts && opts.refetchInterval) timer = setTimeout(read, every);
      }
      read();
      return function () { gone = true; if (timer) clearTimeout(timer); };
    }
  });
}
// A room's moments and presence (ADR 0081); `join(name)` gives a sub-room
// with the same functions. Presence is sent whole, as the latest state.
function roomClient(ns) {
  function room(name) {
    var state = {};
    ns.presence({ room: name, state: null });
    return Object.freeze({
      emit: function (topic, data) { return ns.emit({ room: name, topic: String(topic), data: data === undefined ? null : data }); },
      on: function (topic, fn) {
        return ns.on('moment', function (d) { if (d && d.room === name && d.topic === topic) fn(d.data, d.from); });
      },
      presence: function (patch) {
        for (var k in patch || {}) if (Object.prototype.hasOwnProperty.call(patch, k)) state[k] = patch[k];
        return ns.presence({ room: name, state: state });
      },
      onPeers: function (fn) { return ns.on('peers', function (d) { if (d && d.room === name) fn(d.peers); }); },
      join: function (sub) { return room(String(sub)); },
      leave: function () { return ns.leave({ room: name }); }
    });
  }
  return room('');
}
var clients = {
  db: database, artifact: artifactClient, user: userClient, sample: sampleClient, assets: assetsClient, files: filesClient,
  dictation: dictationClient, tts: ttsClient, mcp: mcpClient, room: roomClient,
  permissions: function (ns) {
    return Object.freeze({
      state: function (name) { return ns.state({ name: name }); },
      request: function (names) { return ns.request({ names: names === undefined ? undefined : [].concat(names) }); },
      manage: function () { return ns.manage(); }
    });
  }
};
var made = {};
window.silt = Object.freeze({
  // The runtime contract the version was made with (ADR 0072).
  version: (boot && typeof boot.contract === 'string' && boot.contract) || '1',
  // A capability's functions, or null where this view can't run it: not
  // declared, not granted, not served here, or failed. Null after 10 s
  // when nothing answers.
  use: function (name) {
    name = String(name);
    if (made[name]) return made[name];
    var answered = ask('silt', 'use', { name: name }).then(function (answer) {
      if (!answer || !answer.available) return null;
      var ns = namespace(name, answer.methods || []);
      return clients[name] ? clients[name](ns) : ns;
    }, function () { return null; });
    var late = new Promise(function (ok) { setTimeout(function () { ok(null); }, 10000); });
    made[name] = Promise.race([answered, late]);
    return made[name];
  },
  // State kept when a new version arrives while the page is open.
  hot: Object.freeze({
    data: boot.hot === undefined ? null : boot.hot,
    snapshot: function (fn) {
      snapshot = typeof fn === 'function' ? fn : null;
      parent.postMessage({ silt: 'hot', keeps: snapshot !== null }, to);
    },
    ready: function (start) { start(boot.hot || {}); }
  })
});
