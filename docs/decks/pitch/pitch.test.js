#!/usr/bin/env node
/* Drive the shipped pitch HTML (not a copy). The investor pitch keeps 19
   slides and the locked sovereignty facts: the play, the five things that
   stay yours, the cited figures, and the built counts. No new market size
   and no funding amount. */
"use strict";

var fs = require("fs");
var path = require("path");
var assert = require("assert");

var PITCH = path.join(__dirname, "index.html");

function nthSlide(html, n) {
  var rest = html;
  var found = 0;
  while (true) {
    var start = rest.indexOf("<section");
    if (start < 0) return null;
    var after = rest.slice(start);
    var end = after.indexOf("</section>");
    if (end < 0) return null;
    end += "</section>".length;
    var block = after.slice(0, end);
    rest = after.slice(end);
    if (/class=["']slide/.test(block)) {
      found += 1;
      if (found === n) return block;
    }
  }
}

function eachSlide(html) {
  var rest = html;
  var slides = [];
  while (true) {
    var start = rest.indexOf("<section");
    if (start < 0) break;
    var after = rest.slice(start);
    var end = after.indexOf("</section>");
    if (end < 0) break;
    end += "</section>".length;
    var block = after.slice(0, end);
    rest = after.slice(end);
    if (/class=["']slide/.test(block)) slides.push(block);
  }
  return slides;
}

function visibleText(html) {
  return html
    .replace(/<[^>]+>/g, " ")
    .replace(/&nbsp;/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/\s+/g, " ")
    .trim();
}

var html = fs.readFileSync(PITCH, "utf8");
var slides = eachSlide(html);
assert.strictEqual(slides.length, 19, "pitch has 19 slides, got " + slides.length);

var slide4 = visibleText(nthSlide(html, 4));
var slide4Lower = slide4.toLowerCase();
assert.ok(slide4Lower.indexOf("sovereign stack") >= 0, "slide 4 names the sovereign stack");
assert.ok(/\bown(ing|s|ed)?\b/i.test(slide4), "slide 4 says own/owning: " + slide4);
assert.ok(
  slide4Lower.indexOf("the play") >= 0 ||
    slide4Lower.indexOf("this is the bet") >= 0 ||
    slide4Lower.indexOf("the bet after") >= 0,
  "slide 4 calls it the play: " + slide4
);
["compute", "models", "talent", "policy", "desk"].forEach(function (layer) {
  assert.ok(slide4Lower.indexOf(layer) >= 0, "slide 4 names layer " + layer);
});
assert.ok(slide4.indexOf("Sovra") >= 0, "slide 4 names Sovra");

var sovereign = slides.filter(function (slide) {
  return visibleText(slide).indexOf("100% sovereign") >= 0;
});
assert.strictEqual(sovereign.length, 1, "one slide is titled in copy as 100% sovereign");
var five = visibleText(sovereign[0]);
["Data", "Knowledge", "The AI", "Decisions", "Learning"].forEach(function (name) {
  assert.ok(five.indexOf(name) >= 0, "100% sovereign slide names " + name + ": " + five);
});

var deck = visibleText(html);
["50%", "2027", "70%", "1 GW", "95%"].forEach(function (figure) {
  assert.ok(deck.indexOf(figure) >= 0, "pitch still cites " + figure);
});

var built = slides.filter(function (slide) {
  return slide.indexOf('data-section="Built"') >= 0;
});
assert.strictEqual(built.length, 1, "one Built slide");
["12", "49", "3", "11"].forEach(function (count) {
  assert.ok(
    built[0].indexOf('data-count="' + count + '"') >= 0,
    "built count " + count + " stays on the Built slide"
  );
  assert.ok(visibleText(built[0]).indexOf(count) >= 0, "built count " + count + " is visible");
});

assert.ok(!/\$\s?\d/.test(deck), "no funding dollar amount in visible copy: " + deck);
assert.ok(!/\bTAM\b/i.test(deck), "no TAM in visible copy");
assert.ok(!/billion/i.test(deck), "no billion-scale market figure");
assert.ok(!/million/i.test(deck), "no million-scale market figure");
assert.ok(
  html.indexOf("The funding amount is for the founder to add") >= 0,
  "the ask still leaves the amount for the founder"
);

/* Criterion 2. Classify each slide from its markup. A free-floating label
   is not enough: side-by-side means a .pair or .split that holds both a
   headline and a picture; stacked means the headline and then a picture;
   picture-over means the picture comes first; a centered figure is a
   .figure that holds the picture after the headline. */
function sectionClass(slide) {
  var m = slide.match(/<section[^>]*\bclass="([^"]*)"/);
  return m ? m[1] : "";
}

function headlineAt(slide) {
  var m = /<h1\b|class="[^"]*\b(?:giant|mega)\b/.exec(slide);
  return m ? m.index : -1;
}

function pictureAt(slide) {
  var re = /<svg\b|class="[^"]*\b(?:scene|mock|fill)\b/g;
  var m;
  var at = -1;
  while ((m = re.exec(slide))) {
    if (at < 0 || m.index < at) at = m.index;
  }
  return at;
}

function arrangement(slide) {
  var sec = sectionClass(slide);
  if (/\bdivider\b/.test(sec)) return "divider";
  if (/\bdata-hero\b/.test(slide)) return "hero";
  if (/class="[^"]*\bbigstat\b/.test(slide) || /class="[^"]*\bhuge\b/.test(slide)) return "giant number";
  if (/class="[^"]*\bed\b/.test(slide)) return "numbered list";
  if (/class="[^"]*\bsteps\b/.test(slide)) return "step row";
  var head = headlineAt(slide);
  var picture = pictureAt(slide);
  var side = /class="[^"]*\b(?:pair|split)\b/.test(slide);
  if (side && picture >= 0 && head >= 0) return "side-by-side headline-and-picture";
  if (picture >= 0 && head >= 0 && picture < head) return "picture over type";
  if (/class="[^"]*\bfigure\b/.test(slide) && picture >= 0 && head >= 0 && head < picture) {
    return "centered figure";
  }
  if (picture >= 0 && head >= 0 && head < picture) return "stacked picture";
  if (/class="[^"]*\b(?:layers|quint)\b/.test(slide)) return "full-width band";
  return "type-only";
}

var arrangements = slides.map(arrangement);
var allowed = {
  "side-by-side headline-and-picture": true,
  "stacked picture": true,
  "picture over type": true,
  "centered figure": true,
  "full-width band": true,
  "type-only": true,
  "giant number": true,
  "numbered list": true,
  "step row": true,
  "divider": true,
  hero: true
};
arrangements.forEach(function (name, i) {
  assert.ok(allowed[name], "slide " + (i + 1) + " has an unknown arrangement: " + name);
});
var sideBySide = arrangements.filter(function (name) {
  return name === "side-by-side headline-and-picture";
});
assert.ok(
  sideBySide.length <= 2,
  "at most two side-by-side headline-and-picture slides, got " + sideBySide.length + ": " + arrangements.join(" | ")
);
for (var i = 0; i < arrangements.length - 2; i++) {
  assert.ok(
    !(arrangements[i] === arrangements[i + 1] && arrangements[i + 1] === arrangements[i + 2]),
    "slides " + (i + 1) + "-" + (i + 3) + " share one arrangement (" + arrangements[i] + "): " + arrangements.join(" | ")
  );
}
console.log("arrangements: " + arrangements.join(" | "));

console.log("pitch.test.js: ok");
