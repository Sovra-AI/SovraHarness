#!/usr/bin/env node
/* Drive the shipped pitch HTML (not a copy). Slide 4 must state that owning
   the sovereign stack is the play, and still name the stack layers. */
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

function visibleText(html) {
  return html
    .replace(/<[^>]+>/g, " ")
    .replace(/&nbsp;/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/\s+/g, " ")
    .trim();
}

var html = fs.readFileSync(PITCH, "utf8");
var slide = nthSlide(html, 4);
assert.ok(slide, "fourth <section class=\"slide\"> must exist in " + PITCH);
var visible = visibleText(slide);
var lower = visible.toLowerCase();

assert.ok(lower.indexOf("sovereign stack") >= 0, "slide 4 names the sovereign stack: " + visible);
assert.ok(/\bown(ing|s|ed)?\b/i.test(visible), "slide 4 says own/owning: " + visible);
assert.ok(
  lower.indexOf("the play") >= 0 ||
    lower.indexOf("this is the bet") >= 0 ||
    lower.indexOf("the bet after") >= 0,
  "slide 4 calls it the play (or the bet): " + visible
);
["compute", "models", "talent", "policy", "desk"].forEach(function (layer) {
  assert.ok(lower.indexOf(layer) >= 0, "slide 4 names layer " + layer + ": " + visible);
});
assert.ok(visible.indexOf("Sovra") >= 0, "slide 4 names Sovra: " + visible);

console.log("slide4.test.js: ok");
