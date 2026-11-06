#!/usr/bin/env node
/* Drive the shipped cinema.js engine (classic script, no ES modules).
   Asserts hash #2, clamp, 1920×1080 fit, and a single .slide.active. */
"use strict";

var fs = require("fs");
var path = require("path");
var vm = require("vm");
var assert = require("assert");

var ENGINE = path.join(__dirname, "cinema.js");

function classList(el) {
  function tokens() {
    return String(el.className || "").split(/\s+/).filter(Boolean);
  }
  return {
    contains: function (name) { return tokens().indexOf(name) >= 0; },
    add: function (name) {
      if (!this.contains(name)) el.className = tokens().concat([name]).join(" ");
    },
    remove: function (name) {
      el.className = tokens().filter(function (c) { return c !== name; }).join(" ");
    },
    toggle: function (name, force) {
      var on = force === undefined ? !this.contains(name) : !!force;
      if (on) this.add(name); else this.remove(name);
      return on;
    }
  };
}

function createEl(tag) {
  var el = {
    tagName: String(tag).toUpperCase(),
    id: "",
    className: "",
    childNodes: [],
    parentNode: null,
    textContent: "",
    _innerHTML: "",
    _attrs: {},
    style: {
      transform: "",
      width: "",
      opacity: "",
      setProperty: function (k, v) { this[k] = v; },
      getPropertyValue: function () { return ""; }
    },
    dataset: {},
    disabled: false
  };
  el.classList = classList(el);
  el.getAttribute = function (n) {
    if (n === "id") return el.id;
    if (n === "class") return el.className;
    return Object.prototype.hasOwnProperty.call(el._attrs, n) ? el._attrs[n] : null;
  };
  el.setAttribute = function (n, v) {
    v = String(v);
    if (n === "id") el.id = v;
    else if (n === "class") el.className = v;
    else el._attrs[n] = v;
  };
  el.hasAttribute = function (n) { return el.getAttribute(n) != null; };
  el.appendChild = function (child) {
    child.parentNode = el;
    el.childNodes.push(child);
    return child;
  };
  el.insertBefore = function (child, ref) {
    child.parentNode = el;
    var i = el.childNodes.indexOf(ref);
    if (i < 0) el.childNodes.push(child);
    else el.childNodes.splice(i, 0, child);
    return child;
  };
  el.querySelector = function (sel) { return queryAll(el, sel)[0] || null; };
  el.querySelectorAll = function (sel) { return queryAll(el, sel); };
  el.closest = function () { return null; };
  el.addEventListener = function () {};
  el.removeEventListener = function () {};
  Object.defineProperty(el, "innerHTML", {
    get: function () { return el._innerHTML; },
    set: function (v) { el._innerHTML = String(v); }
  });
  el.forEach = undefined;
  return el;
}

function walk(node, acc) {
  (node.childNodes || []).forEach(function (c) {
    acc.push(c);
    walk(c, acc);
  });
  return acc;
}

function matchSel(el, sel) {
  sel = sel.trim();
  if (!sel) return false;
  if (sel.indexOf(",") >= 0) {
    return sel.split(",").some(function (s) { return matchSel(el, s); });
  }
  if (sel.charAt(0) === "#") return el.id === sel.slice(1);
  if (sel.charAt(0) === ".") return el.classList.contains(sel.slice(1));
  if (sel.charAt(0) === "[") {
    var attr = sel.slice(1, -1);
    return el.hasAttribute(attr);
  }
  var m = sel.match(/^([a-zA-Z0-9-]+)?(?:\.([a-zA-Z0-9_-]+))?(?:\[([^\]]+)\])?$/);
  if (!m) return false;
  if (m[1] && el.tagName !== m[1].toUpperCase()) return false;
  if (m[2] && !el.classList.contains(m[2])) return false;
  if (m[3] && !el.hasAttribute(m[3])) return false;
  return true;
}

function queryAll(root, sel) {
  var list = walk(root, []);
  var out = list.filter(function (el) { return matchSel(el, sel); });
  out.forEach = Array.prototype.forEach;
  return out;
}

function makeWindow(slideCount, hash) {
  var deck = createEl("div");
  deck.id = "deck";
  var slides = [];
  for (var i = 0; i < slideCount; i++) {
    var s = createEl("section");
    s.className = "slide";
    s.setAttribute("data-section", "S" + (i + 1));
    s.setAttribute("data-note", "note");
    deck.appendChild(s);
    slides.push(s);
  }
  var body = createEl("body");
  body.setAttribute("data-logo", "../../../crates/sovra-ui/assets/branding/sovra-logo.svg");
  body.setAttribute("data-title", "For a family or a small business");
  var stage = createEl("div");
  stage.id = "stage";
  stage.appendChild(deck);
  body.appendChild(stage);

  var loc = { hash: hash || "" };
  var hist = {
    replaceState: function (_s, _t, url) {
      var m = String(url).match(/#.*$/);
      loc.hash = m ? m[0] : "";
    }
  };

  var listeners = { resize: [], hashchange: [], keydown: [], click: [], touchstart: [], touchend: [] };
  function on(type, fn) {
    if (!listeners[type]) listeners[type] = [];
    listeners[type].push(fn);
  }

  var win = {
    innerWidth: 1920,
    innerHeight: 1080,
    location: loc,
    history: hist,
    matchMedia: function () { return { matches: false }; },
    addEventListener: on,
    requestAnimationFrame: function () { return 0; },
    cancelAnimationFrame: function () {},
    setTimeout: function () { return 0; },
    clearTimeout: function () {},
    performance: { now: function () { return 0; } },
    document: null
  };

  var document = {
    body: body,
    documentElement: createEl("html"),
    fullscreenElement: null,
    getElementById: function (id) {
      if (id === "deck") return deck;
      if (id === "stage") return stage;
      return null;
    },
    querySelectorAll: function (sel) {
      if (sel === ".slide") {
        var list = slides.slice();
        list.forEach = Array.prototype.forEach;
        return list;
      }
      if (sel === ".slide.active") {
        var active = slides.filter(function (s) { return s.classList.contains("active"); });
        active.forEach = Array.prototype.forEach;
        return active;
      }
      if (sel === "canvas[data-three]") {
        var empty = [];
        empty.forEach = Array.prototype.forEach;
        return empty;
      }
      return queryAll(body, sel);
    },
    querySelector: function (sel) { return document.querySelectorAll(sel)[0] || null; },
    createElement: createEl,
    addEventListener: on
  };
  win.document = document;
  win.window = win;
  return { window: win, deck: deck, slides: slides, location: loc };
}

function loadEngine(env) {
  var code = fs.readFileSync(ENGINE, "utf8");
  assert.equal(code.indexOf("import "), -1, "cinema.js must not use ES import");
  assert.equal(/^\s*import\s/m.test(code), false);
  assert.equal(code.indexOf("type=\"module\""), -1);
  var sandbox = env.window;
  sandbox.window = sandbox;
  sandbox.console = console;
  sandbox.SovraCinema = undefined;
  vm.runInNewContext(code, sandbox, { filename: "cinema.js" });
  return sandbox.SovraCinema;
}

function run() {
  var src = fs.readFileSync(ENGINE, "utf8");
  assert.ok(src.indexOf("SovraCinema") >= 0, "shipped engine exposes SovraCinema");
  assert.ok(!/\brequire\s*\(/.test(src), "no unguarded Node require");

  var C0 = loadEngine(makeWindow(3, ""));
  assert.strictEqual(C0.DESIGN_W, 1920);
  assert.strictEqual(C0.DESIGN_H, 1080);
  assert.strictEqual(C0.pad(3), "03");
  assert.strictEqual(C0.pad(12), "12");
  assert.strictEqual(C0.fromHash("#2", 14), 1);
  assert.strictEqual(C0.fromHash("#1", 14), 0);
  assert.strictEqual(C0.fromHash("#99", 14), 0);
  assert.strictEqual(C0.fromHash("", 14), 0);
  assert.strictEqual(C0.clamp(-1, 14), 0);
  assert.strictEqual(C0.clamp(99, 14), 13);
  assert.strictEqual(C0.clamp(3, 14), 3);
  assert.strictEqual(C0.fitScale(1920, 1080, false), 1);
  assert.ok(C0.fitScale(960, 540, false) === 0.5);

  var env = makeWindow(4, "#2");
  var C = loadEngine(env);
  var engine = C.boot();
  assert.ok(engine, "boot returns an instance when #deck exists");
  assert.strictEqual(engine.getTotal(), 4);
  assert.strictEqual(engine.getIndex(), 1, "hash #2 opens slide 2 (index 1)");
  var active = env.slides.filter(function (s) { return s.classList.contains("active"); });
  assert.strictEqual(active.length, 1, "only one .slide.active");
  assert.ok(env.slides[1].classList.contains("active"));

  engine.show(99);
  assert.strictEqual(engine.getIndex(), 3, "next clamps at the last slide");
  engine.show(-4);
  assert.strictEqual(engine.getIndex(), 0, "prev clamps at the first slide");
  active = env.slides.filter(function (s) { return s.classList.contains("active"); });
  assert.strictEqual(active.length, 1);

  var k = engine.fit();
  assert.strictEqual(k, 1, "1920×1080 window fits at scale 1");
  assert.strictEqual(env.deck.style.transform, "scale(1)");

  env.window.innerWidth = 960;
  env.window.innerHeight = 1080;
  assert.strictEqual(engine.fit(), 0.5);

  var empty = makeWindow(0, "");
  empty.deck.id = "not-deck";
  empty.window.document.getElementById = function () { return null; };
  empty.window.document.querySelectorAll = function () {
    var list = [];
    list.forEach = Array.prototype.forEach;
    return list;
  };
  var Cempty = loadEngine(empty);
  assert.strictEqual(Cempty.boot(), null, "boot is a no-op without #deck");

  console.log("cinema-engine.test.js: ok");
}

run();
