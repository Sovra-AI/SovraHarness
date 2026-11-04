/* Present a 1920×1200 Sovra deck: one slide, scaled to the window.
   Arrow / Space / PageDown next · ArrowLeft / PageUp previous · Home / End · F fullscreen.
   Hash #3 opens slide 3. Print media stacks every slide. */
(function () {
  const deck = document.getElementById("deck");
  const slides = Array.from(document.querySelectorAll(".slide"));
  if (!deck || slides.length === 0) return;

  const total = slides.length;
  let index = 0;

  function fit() {
    const s = Math.min(window.innerWidth / 1920, window.innerHeight / 1200);
    deck.style.transform = "scale(" + s + ")";
  }

  function show(i) {
    index = Math.max(0, Math.min(total - 1, i));
    slides.forEach(function (slide, n) {
      slide.classList.toggle("active", n === index);
    });
    const hash = String(index + 1);
    if (location.hash.replace("#", "") !== hash) {
      history.replaceState(null, "", "#" + hash);
    }
  }

  function fromHash() {
    const n = parseInt(location.hash.replace("#", ""), 10);
    if (Number.isFinite(n) && n >= 1 && n <= total) return n - 1;
    return 0;
  }

  window.addEventListener("resize", fit);
  window.addEventListener("hashchange", function () {
    show(fromHash());
  });
  document.addEventListener("keydown", function (e) {
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    if (e.key === "ArrowRight" || e.key === " " || e.key === "PageDown") {
      e.preventDefault();
      show(index + 1);
    } else if (e.key === "ArrowLeft" || e.key === "PageUp") {
      e.preventDefault();
      show(index - 1);
    } else if (e.key === "Home") {
      e.preventDefault();
      show(0);
    } else if (e.key === "End") {
      e.preventDefault();
      show(total - 1);
    } else if (e.key === "f" || e.key === "F") {
      if (!document.fullscreenElement) document.documentElement.requestFullscreen();
      else document.exitFullscreen();
    }
  });

  document.addEventListener("click", function (e) {
    if (e.target.closest("a,button")) return;
    const mid = window.innerWidth / 2;
    if (e.clientX >= mid) show(index + 1);
    else show(index - 1);
  });

  fit();
  show(fromHash());
})();
