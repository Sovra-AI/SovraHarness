/* A discreet "All decks" link on every slide, so a deck is one page of a site
   rather than a dead end. Inside the site's viewer it changes the parent
   route; opened on its own it goes to the deck index. Classic script. */
(function () {
  var a = document.createElement("a");
  a.id = "decknav";
  a.textContent = "ALL DECKS";
  a.href = "../index.html#/";
  a.title = "All Sovra decks";
  a.style.cssText = "position:fixed;left:50%;top:14px;transform:translateX(-50%);z-index:50;font:500 12px/1 'JetBrains Mono','IBM Plex Mono',ui-monospace,monospace;letter-spacing:.22em;color:#8a7aa8;padding:8px 14px;border-radius:999px;border:1px solid rgba(124,58,237,.25);background:rgba(255,255,255,.6);text-decoration:none;opacity:.55;transition:opacity .25s";
  a.onmouseenter = function () { a.style.opacity = "1"; };
  a.onmouseleave = function () { a.style.opacity = ".55"; };
  a.onclick = function (e) {
    if (window.top !== window) {
      try { window.top.location.hash = "#/"; e.preventDefault(); } catch (err) { /* not our frame; follow the link */ }
    }
  };
  if (document.body) document.body.appendChild(a);
  else document.addEventListener("DOMContentLoaded", function () { document.body.appendChild(a); });
})();
