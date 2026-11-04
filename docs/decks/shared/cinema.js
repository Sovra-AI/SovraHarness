/* Sovra Cinema deck engine · 1920×1080.
   → / Space / PageDown next · ← / PageUp back · Home / End · F fullscreen ·
   O overview · #7 opens slide 7 · swipe on touch.
   Canvas scenes (<canvas data-fx="globe|net|train|mesh">) run only while their
   slide is active, so a 30-slide deck costs one animation at a time. */
(function () {
  "use strict";
  var deck = document.getElementById("deck");
  var slides = Array.prototype.slice.call(document.querySelectorAll(".slide"));
  if (!deck || !slides.length) return;
  var total = slides.length, index = -1, overview = false;
  var reduce = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  var progress = document.createElement("div"); progress.id = "progress"; document.body.appendChild(progress);
  var hint = document.createElement("div"); hint.id = "hint"; hint.textContent = "← →  ·  F FULLSCREEN  ·  O OVERVIEW"; document.body.appendChild(hint);
  setTimeout(function () { hint.style.opacity = "0"; }, 6000);

  var logo = document.body.getAttribute("data-logo") || "";
  var title = document.body.getAttribute("data-title") || "";
  slides.forEach(function (s, n) {
    if (!s.hasAttribute("data-bare")) {
      var ch = document.createElement("div"); ch.className = "chrome";
      ch.innerHTML = '<div class="brand"><img src="' + logo + '" alt="Sovra"><span><b>SOVRA</b> · ' + title + '</span></div><div>' + (s.getAttribute("data-section") || "") + "</div>";
      s.insertBefore(ch, s.firstChild);
      var ft = document.createElement("div"); ft.className = "foot";
      ft.innerHTML = "<span>" + (s.getAttribute("data-note") || "") + '</span><span data-pg></span>';
      s.appendChild(ft);
    }
    var pg = s.querySelector("[data-pg]");
    if (pg) pg.textContent = pad(n + 1) + " / " + pad(total);
    // stagger reveal children automatically when no explicit --i
    var i = 0;
    s.querySelectorAll(".rv").forEach(function (el) { if (!el.style.getPropertyValue("--i")) el.style.setProperty("--i", i++); });
    // measure svg strokes for the draw-on effect
    s.querySelectorAll(".draw path,.draw line,.draw polyline,.draw circle.stroke").forEach(function (p) {
      try { p.style.setProperty("--len", Math.ceil(p.getTotalLength() + 2)); } catch (e) { /* not rendered yet */ }
    });
  });
  function pad(n) { return n < 10 ? "0" + n : String(n); }

  function fit() {
    if (overview) {
      var s = (window.innerWidth - 40) / (5 * 1920 + 4 * 80 + 160);
      deck.style.transform = "scale(" + s + ")";
      return;
    }
    var k = Math.min(window.innerWidth / 1920, window.innerHeight / 1080);
    deck.style.transform = "scale(" + k + ")";
  }

  function show(i) {
    i = Math.max(0, Math.min(total - 1, i));
    if (i === index) return;
    if (index >= 0) stopFx(slides[index]);
    index = i;
    slides.forEach(function (s, n) { s.classList.toggle("active", n === index); });
    progress.style.width = ((index + 1) / total * 100) + "%";
    var h = String(index + 1);
    if (location.hash.replace("#", "") !== h) history.replaceState(null, "", "#" + h);
    startFx(slides[index]);
    countUp(slides[index]);
  }

  function fromHash() {
    var n = parseInt(location.hash.replace("#", ""), 10);
    return Number.isFinite(n) && n >= 1 && n <= total ? n - 1 : 0;
  }

  function toggleOverview() {
    overview = !overview;
    document.body.classList.toggle("overview", overview);
    fit();
    if (overview) slides[index].scrollIntoView({ block: "center" });
  }

  window.addEventListener("resize", fit);
  window.addEventListener("hashchange", function () { show(fromHash()); });
  document.addEventListener("keydown", function (e) {
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    var k = e.key;
    if (k === "ArrowRight" || k === " " || k === "PageDown" || k === "ArrowDown") { e.preventDefault(); show(index + 1); }
    else if (k === "ArrowLeft" || k === "PageUp" || k === "ArrowUp") { e.preventDefault(); show(index - 1); }
    else if (k === "Home") { e.preventDefault(); show(0); }
    else if (k === "End") { e.preventDefault(); show(total - 1); }
    else if (k === "f" || k === "F") { if (!document.fullscreenElement) document.documentElement.requestFullscreen(); else document.exitFullscreen(); }
    else if (k === "o" || k === "O" || k === "Escape") { if (k !== "Escape" || overview) toggleOverview(); }
  });
  document.addEventListener("click", function (e) {
    if (e.target.closest("a,button")) return;
    if (overview) {
      var s = e.target.closest(".slide");
      if (s) { toggleOverview(); show(slides.indexOf(s)); }
      return;
    }
    show(index + (e.clientX >= window.innerWidth / 2 ? 1 : -1));
  });
  var tx = null;
  document.addEventListener("touchstart", function (e) { tx = e.touches[0].clientX; }, { passive: true });
  document.addEventListener("touchend", function (e) {
    if (tx === null) return;
    var dx = e.changedTouches[0].clientX - tx; tx = null;
    if (Math.abs(dx) > 40) show(index + (dx < 0 ? 1 : -1));
  }, { passive: true });

  /* ----------------------------------------------------------- count up */
  function countUp(slide) {
    slide.querySelectorAll("[data-count]").forEach(function (el) {
      var target = parseFloat(el.getAttribute("data-count"));
      var suffix = el.getAttribute("data-suffix") || "";
      var sep = el.hasAttribute("data-sep");
      if (reduce) { el.textContent = fmt(target) + suffix; return; }
      var t0 = null, dur = 1600;
      function fmt(v) { return sep ? v.toLocaleString("en-US") : String(v); }
      // timer-driven so it also completes where animation frames are throttled
      function step() {
        if (!slide.classList.contains("active")) { el.textContent = fmt(target) + suffix; return; }
        var t = performance.now();
        if (t0 === null) t0 = t;
        var p = Math.min(1, (t - t0) / dur), e = 1 - Math.pow(1 - p, 4);
        el.textContent = fmt(Math.round(target * e)) + suffix;
        if (p < 1) setTimeout(step, 16);
      }
      setTimeout(step, 350);
    });
  }

  /* ------------------------------------------------------- canvas scenes */
  var running = [];
  function startFx(slide) {
    slide.querySelectorAll("canvas[data-fx]").forEach(function (c) {
      var fx = FX[c.getAttribute("data-fx")];
      if (!fx) return;
      if (!c._scene) c._scene = fx(c);
      c._live = true;
      var last = performance.now();
      (function loop(t) {
        if (!c._live) return;
        var dt = Math.min(0.05, (t - last) / 1000); last = t;
        c._scene(reduce ? 0 : dt, t / 1000);
        c._raf = requestAnimationFrame(loop);
      })(last);
      running.push(c);
    });
  }
  function stopFx() {
    running.forEach(function (c) { c._live = false; cancelAnimationFrame(c._raf); });
    running = [];
  }

  function setup(c) {
    var w = c.width = c.clientWidth || 1920, h = c.height = c.clientHeight || 1080;
    return { ctx: c.getContext("2d"), w: w, h: h };
  }
  function rng(seed) { return function () { seed = (seed * 16807) % 2147483647; return (seed - 1) / 2147483646; }; }

  var FX = {
    /* A slowly turning sphere of computers. Links are peers; pulses are
       memory and lessons moving between them — no centre. */
    globe: function (c) {
      var o = setup(c), ctx = o.ctx, W = o.w, H = o.h;
      var cx = W * parseFloat(c.dataset.x || 0.5), cy = H * parseFloat(c.dataset.y || 0.5);
      var R = Math.min(W, H) * parseFloat(c.dataset.r || 0.42);
      var N = parseInt(c.dataset.n || 220, 10), cool = c.dataset.tone === "cool";
      var pts = [], links = [], pulses = [], r = rng(7);
      for (var i = 0; i < N; i++) {
        var y = 1 - (i / (N - 1)) * 2, rad = Math.sqrt(1 - y * y), th = i * 2.399963;
        pts.push([Math.cos(th) * rad, y, Math.sin(th) * rad, r() < 0.12]);
      }
      for (var a = 0; a < N; a++) for (var b = a + 1; b < N; b++) {
        var d = Math.hypot(pts[a][0] - pts[b][0], pts[a][1] - pts[b][1], pts[a][2] - pts[b][2]);
        if (d < 0.26) links.push([a, b]);
      }
      var rot = 0, tilt = 0.38;
      function proj(p) {
        var x = p[0] * Math.cos(rot) - p[2] * Math.sin(rot), z = p[0] * Math.sin(rot) + p[2] * Math.cos(rot);
        var y = p[1] * Math.cos(tilt) - z * Math.sin(tilt); z = p[1] * Math.sin(tilt) + z * Math.cos(tilt);
        var s = 1.9 / (2.6 - z);
        return [cx + x * R * s, cy + y * R * s, z, s];
      }
      return function (dt, t) {
        rot += dt * 0.09;
        ctx.clearRect(0, 0, W, H);
        var g = ctx.createRadialGradient(cx, cy, R * 0.1, cx, cy, R * 1.35);
        g.addColorStop(0, cool ? "rgba(100,116,139,.07)" : "rgba(168,85,247,.09)"); g.addColorStop(1, "rgba(0,0,0,0)");
        ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
        var P = pts.map(proj);
        ctx.lineWidth = 1;
        for (var k = 0; k < links.length; k++) {
          var p = P[links[k][0]], q = P[links[k][1]], z = (p[2] + q[2]) / 2;
          var al = 0.07 + 0.30 * (z + 1) / 2;
          ctx.strokeStyle = (cool ? "rgba(100,116,139," : "rgba(124,58,237,") + al + ")";
          ctx.beginPath(); ctx.moveTo(p[0], p[1]); ctx.lineTo(q[0], q[1]); ctx.stroke();
        }
        if (Math.random() < 0.35 && pulses.length < 60) pulses.push({ l: links[(Math.random() * links.length) | 0], t: 0 });
        pulses = pulses.filter(function (u) {
          u.t += dt * 1.4; if (u.t > 1) return false;
          var p = P[u.l[0]], q = P[u.l[1]];
          var x = p[0] + (q[0] - p[0]) * u.t, y = p[1] + (q[1] - p[1]) * u.t;
          ctx.fillStyle = cool ? "rgba(100,116,139,.95)" : "rgba(91,33,182,.95)";
          ctx.shadowBlur = 0;
          ctx.beginPath(); ctx.arc(x, y, 2.4, 0, 6.283); ctx.fill(); ctx.shadowBlur = 0;
          return true;
        });
        for (var n = 0; n < N; n++) {
          var s = P[n], al2 = 0.25 + 0.75 * (s[2] + 1) / 2, big = pts[n][3];
          var rr = (big ? 4.2 : 1.9) * s[3];
          if (big) {
            ctx.fillStyle = "rgba(91,33,182," + (al2 * 0.25) + ")";
            ctx.beginPath(); ctx.arc(s[0], s[1], rr * (3.2 + Math.sin(t * 2 + n) * 0.8), 0, 6.283); ctx.fill();
          }
          ctx.fillStyle = big ? "rgba(91,33,182," + al2 + ")" : (cool ? "rgba(100,116,139," : "rgba(124,58,237,") + al2 + ")";
          ctx.beginPath(); ctx.arc(s[0], s[1], rr, 0, 6.283); ctx.fill();
        }
      };
    },

    /* Quiet drifting constellation for text-heavy slides. */
    net: function (c) {
      var o = setup(c), ctx = o.ctx, W = o.w, H = o.h, r = rng(11), P = [];
      for (var i = 0; i < 70; i++) P.push({ x: r() * W, y: r() * H, vx: (r() - .5) * 14, vy: (r() - .5) * 14 });
      return function (dt) {
        ctx.clearRect(0, 0, W, H);
        P.forEach(function (p) { p.x = (p.x + p.vx * dt + W) % W; p.y = (p.y + p.vy * dt + H) % H; });
        for (var a = 0; a < P.length; a++) for (var b = a + 1; b < P.length; b++) {
          var d = Math.hypot(P[a].x - P[b].x, P[a].y - P[b].y);
          if (d < 190) { ctx.strokeStyle = "rgba(124,58,237," + (0.16 * (1 - d / 190)) + ")"; ctx.beginPath(); ctx.moveTo(P[a].x, P[a].y); ctx.lineTo(P[b].x, P[b].y); ctx.stroke(); }
        }
        ctx.fillStyle = "rgba(124,58,237,.55)";
        P.forEach(function (p) { ctx.beginPath(); ctx.arc(p.x, p.y, 1.8, 0, 6.283); ctx.fill(); });
      };
    },

    /* Federated training: computers around a ring each train on their own
       files (the file glyph never moves). Only a blurred lesson — a cloud of
       noisy particles — flies to the merge, and the shared model ring grows. */
    train: function (c) {
      var o = setup(c), ctx = o.ctx, W = o.w, H = o.h, cx = W / 2, cy = H / 2;
      var K = 9, R = Math.min(W, H) * 0.38, nodes = [], packets = [], round = 0, glow = 0, acc = 0;
      var labels = ["Finance", "Legal", "Sales", "Eng", "Support", "HR", "Ops", "Design", "Research"];
      for (var i = 0; i < K; i++) { var a = -Math.PI / 2 + i * 2 * Math.PI / K; nodes.push({ x: cx + Math.cos(a) * R, y: cy + Math.sin(a) * R, phase: Math.random(), l: labels[i] }); }
      return function (dt, t) {
        ctx.clearRect(0, 0, W, H);
        acc += dt;
        // ring
        ctx.strokeStyle = "rgba(124,58,237,.14)"; ctx.setLineDash([4, 10]); ctx.lineWidth = 1.5;
        ctx.beginPath(); ctx.arc(cx, cy, R, 0, 6.283); ctx.stroke(); ctx.setLineDash([]);
        nodes.forEach(function (n, i) {
          ctx.strokeStyle = "rgba(100,116,139,.10)"; ctx.beginPath(); ctx.moveTo(n.x, n.y); ctx.lineTo(cx, cy); ctx.stroke();
          n.phase += dt * (0.35 + i * 0.013);
          if (n.phase >= 1) { n.phase = 0; for (var q = 0; q < 14; q++) packets.push({ n: n, t: 0, ox: (Math.random() - .5) * 40, oy: (Math.random() - .5) * 40, s: 0.55 + Math.random() * 0.5 }); }
          // local training arc
          ctx.strokeStyle = "rgba(91,33,182,.9)"; ctx.lineWidth = 3;
          ctx.beginPath(); ctx.arc(n.x, n.y, 34, -Math.PI / 2, -Math.PI / 2 + n.phase * 6.283); ctx.stroke();
          ctx.fillStyle = "#ffffff"; ctx.strokeStyle = "rgba(124,58,237,.45)"; ctx.lineWidth = 1.5;
          ctx.beginPath(); ctx.arc(n.x, n.y, 26, 0, 6.283); ctx.fill(); ctx.stroke();
          // file glyph — stays home
          ctx.strokeStyle = "rgba(59,7,100,.85)"; ctx.lineWidth = 1.6;
          ctx.strokeRect(n.x - 7, n.y - 9, 14, 18);
          ctx.beginPath(); ctx.moveTo(n.x - 4, n.y - 3); ctx.lineTo(n.x + 4, n.y - 3); ctx.moveTo(n.x - 4, n.y + 2); ctx.lineTo(n.x + 4, n.y + 2); ctx.stroke();
          ctx.fillStyle = "rgba(88,28,135,.8)"; ctx.font = "500 17px 'JetBrains Mono', monospace"; ctx.textAlign = "center";
          var lx = n.x + (n.x - cx) / R * 70, ly = n.y + (n.y - cy) / R * 62 + 6;
          ctx.fillText(n.l.toUpperCase(), lx, ly);
        });
        packets = packets.filter(function (p) {
          p.t += dt * p.s * 0.8;
          if (p.t >= 1) { glow = Math.min(1, glow + 0.012); return false; }
          var e = p.t * p.t * (3 - 2 * p.t), fade = 1 - e;
          var x = p.n.x + (cx - p.n.x) * e + p.ox * fade + Math.sin(t * 9 + p.ox) * 6 * fade;
          var y = p.n.y + (cy - p.n.y) * e + p.oy * fade + Math.cos(t * 7 + p.oy) * 6 * fade;
          ctx.fillStyle = "rgba(100,116,139," + (0.25 + 0.6 * e) + ")";
          ctx.beginPath(); ctx.arc(x, y, 2.2, 0, 6.283); ctx.fill();
          return true;
        });
        glow = Math.max(0.25, glow - dt * 0.02);
        if (acc > 3) { acc = 0; round++; }
        // shared model
        var pr = 72 + Math.sin(t * 2) * 3;
        var g = ctx.createRadialGradient(cx, cy, 0, cx, cy, pr * 2.6);
        g.addColorStop(0, "rgba(124,58,237," + (0.06 + glow * 0.08) + ")"); g.addColorStop(1, "rgba(168,85,247,0)");
        ctx.fillStyle = g; ctx.beginPath(); ctx.arc(cx, cy, pr * 2.6, 0, 6.283); ctx.fill();
        ctx.fillStyle = "#ffffff"; ctx.strokeStyle = "rgba(91,33,182,.8)"; ctx.lineWidth = 2;
        ctx.beginPath(); ctx.arc(cx, cy, pr, 0, 6.283); ctx.fill(); ctx.stroke();
        for (var k = 0; k < 3; k++) {
          ctx.strokeStyle = "rgba(124,58,237," + (0.5 - k * 0.14) + ")"; ctx.lineWidth = 1.2;
          ctx.beginPath(); ctx.arc(cx, cy, pr - 14 - k * 13, t * (0.6 + k * 0.3), t * (0.6 + k * 0.3) + 4.4); ctx.stroke();
        }
        ctx.fillStyle = "#3b0764"; ctx.textAlign = "center"; ctx.font = "700 21px 'Inter Tight', sans-serif";
        ctx.fillText("Company model", cx, cy - 2);
        ctx.fillStyle = "rgba(71,85,105,.95)"; ctx.font = "500 15px 'JetBrains Mono', monospace";
        ctx.fillText("ROUND " + (1000 + round), cx, cy + 24);
      };
    },

    /* Mesh wave for divider slides. */
    mesh: function (c) {
      var o = setup(c), ctx = o.ctx, W = o.w, H = o.h, cols = 60, rows = 26;
      return function (dt, t) {
        ctx.clearRect(0, 0, W, H);
        for (var j = 0; j < rows; j++) {
          ctx.beginPath();
          for (var i = 0; i <= cols; i++) {
            var x = i / cols * W, z = j / rows;
            var y = H * 0.48 + z * H * 0.55 + Math.sin(i * 0.22 + t * 0.8 + j * 0.35) * 26 * (0.4 + z) + Math.cos(i * 0.09 - t * 0.5) * 18;
            if (i === 0) ctx.moveTo(x, y); else ctx.lineTo(x, y);
          }
          ctx.strokeStyle = "rgba(" + (c.dataset.tone === "cool" ? "8,145,178" : "124,58,237") + "," + (0.05 + 0.2 * (j / rows)) + ")";
          ctx.lineWidth = 1; ctx.stroke();
        }
      };
    }
  };

  /* Optional three.js hero: a glowing icosahedral shell of computers. Falls
     back to the 2D globe when the CDN is unreachable. */
  document.querySelectorAll("canvas[data-three]").forEach(function (c) {
    if (!window.THREE) { c.setAttribute("data-fx", "globe"); return; }
    FX.three = function (cv) {
      var T = window.THREE, W = cv.clientWidth || 1920, H = cv.clientHeight || 1080;
      var r = new T.WebGLRenderer({ canvas: cv, antialias: true, alpha: true });
      r.setPixelRatio(Math.min(2, window.devicePixelRatio || 1)); r.setSize(W, H, false);
      var scene = new T.Scene(), cam = new T.PerspectiveCamera(42, W / H, 0.1, 100);
      cam.position.set(0, 0, 9.5);
      var group = new T.Group(); scene.add(group);
      group.position.x = parseFloat(cv.dataset.ox || 0);
      var geo = new T.IcosahedronGeometry(3, 3);
      var wire = new T.LineSegments(new T.WireframeGeometry(geo), new T.LineBasicMaterial({ color: 0x6d28d9, transparent: true, opacity: 0.22 }));
      group.add(wire);
      var pg = new T.BufferGeometry(); pg.setAttribute("position", geo.getAttribute("position"));
      var dots = new T.Points(pg, new T.PointsMaterial({ color: 0x5b21b6, size: 0.06, transparent: true, opacity: 0.9 }));
      group.add(dots);
      var inner = new T.Mesh(new T.IcosahedronGeometry(1.25, 1), new T.MeshBasicMaterial({ color: 0x94a3b8, wireframe: true, transparent: true, opacity: 0.55 }));
      group.add(inner);
      var halo = new T.Mesh(new T.SphereGeometry(2.9, 48, 48), new T.MeshBasicMaterial({ color: 0xc4b5fd, transparent: true, opacity: 0.05 }));
      group.add(halo);
      var star = new T.BufferGeometry(), sp = [];
      for (var i = 0; i < 900; i++) sp.push((Math.random() - .5) * 40, (Math.random() - .5) * 24, -Math.random() * 20 - 4);
      star.setAttribute("position", new T.Float32BufferAttribute(sp, 3));
      scene.add(new T.Points(star, new T.PointsMaterial({ color: 0x94a3b8, size: 0.03, transparent: true, opacity: 0.30 })));
      return function (dt, t) {
        group.rotation.y += dt * 0.12; group.rotation.x = 0.35 + Math.sin(t * 0.2) * 0.08;
        inner.rotation.y -= dt * 0.4; inner.rotation.z += dt * 0.25;
        dots.material.size = 0.065 + Math.sin(t * 2) * 0.012;
        r.render(scene, cam);
      };
    };
    c.setAttribute("data-fx", "three");
  });

  fit();
  show(fromHash());
})();
