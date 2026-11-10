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

var LOCKED = [
  "Own the sovereign stack.",
  "That\u2019s the play.",
  "The nation owns every layer. Sovra is the desk where the work happens.",
  "The desk \u00b7 Sovra",
  "Helpers, memory, files and approvals on computers the nation owns.",
  "Talent and policy",
  "Chief AI Officers \u00b7 AI Charter",
  "National models",
  "Falcon \u00b7 Jais \u00b7 K2 Think",
  "National compute",
  "Stargate UAE \u00b7 Khazna \u00b7 sovereign clouds"
];
LOCKED.forEach(function (line) {
  assert.ok(slide4.indexOf(line) >= 0, "slide 4 keeps locked line: " + line);
});

function hasContentPicture(slide) {
  if (/<svg\b/.test(slide)) return true;
  var tags = slide.match(/<canvas\b[^>]*>/g) || [];
  return tags.some(function (tag) {
    return !/\b(?:bgcanvas|background)\b/i.test(tag);
  });
}
assert.ok(
  hasContentPicture(nthSlide(html, 4)),
  "slide 4 contains a content picture (svg or non-background canvas)"
);

var HEADLINES = [
  "AI-native. On UAE terms.",
  "of UAE government work is to run on AI that acts, within two years.",
  "If AI does half the work, whose AI is it?",
  "Own the sovereign stack. That\u2019s the play.",
  "Rented AI makes a nation AI-native on someone else\u2019s terms.",
  "Assistants in your own office.",
  "Five things stay yours.",
  "One ask. A team answers.",
  "Nothing is ever secretly rewritten.",
  "From one desk to a sovereign nation.",
  "A desk. A team. A nation.",
  "Prove where every record lives.",
  "Your own AI, trained at home.",
  "Proof, not promises.",
  "Two questions sort the whole market.",
  "A real product, on real computers.",
  "The right answer, at the right time.",
  "Arabic. Teams. Certified. The ledger.",
  "Keep it in the UAE. Prove it on the desk."
];
function headlineText(slide) {
  var m = slide.match(/<h1\b[^>]*>[\s\S]*?<\/h1>|<div\b[^>]*class="[^"]*\b(?:giant|mega)\b[^>]*>[\s\S]*?<\/div>/);
  return m ? visibleText(m[0]) : "";
}
slides.forEach(function (block, n) {
  assert.strictEqual(headlineText(block), HEADLINES[n], "slide " + (n + 1) + " headline changed");
});

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

function tagAttr(tag, name) {
  var m = tag.match(new RegExp("\\b" + name + "\\s*=\\s*([\"'])([\\s\\S]*?)\\1", "i"));
  return m ? m[2] : "";
}
function extraCdnAssets(doc) {
  var assets = [];
  var scriptRe = /<script\b[^>]*>/gi;
  var found;
  while ((found = scriptRe.exec(doc))) {
    var script = found[0];
    if (/\btype\s*=\s*["']module["']/i.test(script)) continue;
    var src = tagAttr(script, "src");
    if (!/^https:\/\//i.test(src)) continue;
    if (/three(?:\.min)?\.js/i.test(src) && /r128/.test(src)) continue;
    if (/fonts\.googleapis\.com/i.test(src)) continue;
    assets.push(src);
  }
  return assets;
}
function graphicHosts(slide) {
  var hosts = [];
  var re = /<(?:img|use|object|svg)\b[^>]*>/gi;
  var found;
  while ((found = re.exec(slide))) {
    var tag = found[0];
    var url = tagAttr(tag, "src") || tagAttr(tag, "href") || tagAttr(tag, "data") || tagAttr(tag, "xlink:href");
    if (!url) continue;
    var abs;
    try { abs = new URL(url, "https://placeholder.invalid/"); } catch (err) { continue; }
    if (abs.host && abs.host !== "placeholder.invalid") hosts.push(abs.host);
  }
  return hosts;
}
function pageStyle(doc) {
  var m = doc.match(/<style>([\s\S]*?)<\/style>/);
  return m ? m[1] : "";
}
function fillOf(slide, css) {
  var cls = (slide.match(/<section[^>]*\bclass="([^"]*)"/) || ["", ""])[1].split(/\s+/);
  var foundFill = "";
  cls.forEach(function (name) {
    if (!name || name === "slide") return;
    var rule = css.match(new RegExp("#deck\\s+\\.slide\\." + name + "\\s*\\{([^}]*)\\}"));
    if (!rule) return;
    var bg = rule[1].match(/background\s*:\s*(#[0-9a-fA-F]{6})\b/);
    if (bg) foundFill = bg[1].toLowerCase();
  });
  return foundFill;
}
function luminance(hex) {
  function chan(pair) {
    var c = parseInt(pair, 16) / 255;
    return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  }
  return 0.2126 * chan(hex.slice(1, 3)) + 0.7152 * chan(hex.slice(3, 5)) + 0.0722 * chan(hex.slice(5, 7));
}

assert.ok(!/type\s*=\s*["']module["']/i.test(html), "pitch stays on classic scripts");
var cdnAssets = extraCdnAssets(html);
assert.strictEqual(cdnAssets.length, 1, "one extra CDN stylesheet or classic script, got " + cdnAssets.join(" | "));
var cdnHost = new URL(cdnAssets[0]).host;
var cdnSlides = slides.filter(function (block) {
  return graphicHosts(block).indexOf(cdnHost) >= 0;
}).length;
assert.ok(cdnSlides >= 8, "at least eight slides use a " + cdnHost + " graphic, got " + cdnSlides);

var css = pageStyle(html);
var fills = slides.map(function (block, n) {
  var fill = fillOf(block, css);
  assert.ok(fill, "slide " + (n + 1) + " has a page-local light fill");
  assert.ok(luminance(fill) >= 0.8, "slide " + (n + 1) + " fill is light (" + fill + ")");
  return fill;
});
var fillSet = {};
fills.forEach(function (fill) { fillSet[fill] = true; });
assert.ok(Object.keys(fillSet).length >= 4, "at least four fills, got " + Object.keys(fillSet).join(", "));
for (var f = 1; f < fills.length; f++) {
  assert.notStrictEqual(fills[f], fills[f - 1], "slides " + f + " and " + (f + 1) + " share fill " + fills[f]);
}
var treatments = slides.map(function (block, n) {
  var art = graphicHosts(block).indexOf(cdnHost) >= 0 ? "cdn" : "plain";
  return arrangements[n] + "+" + art;
});
var treatmentSet = {};
treatments.forEach(function (name) { treatmentSet[name] = true; });
assert.ok(Object.keys(treatmentSet).length >= 6, "at least six treatments: " + treatments.join(" | "));
for (var t = 1; t < treatments.length; t++) {
  assert.notStrictEqual(
    treatments[t],
    treatments[t - 1],
    "slides " + t + " and " + (t + 1) + " share treatment " + treatments[t]
  );
}
console.log("fills: " + fills.join(" | "));
console.log("treatments: " + treatments.join(" | "));

console.log("pitch.test.js: ok");
