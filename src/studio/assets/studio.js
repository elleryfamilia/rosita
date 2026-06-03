// Minimal htmx-compatible shim for rosita studio (Slice 1).
//
// Reads the htmx attribute subset we use — `hx-post`, `hx-target`,
// `hx-trigger` (event names + optional `delay:Nms` debounce) — and swaps the
// server-rendered fragment into the target. Same-origin fetch sends the session
// cookie automatically. Drop in real htmx later and the markup still works.
(function () {
  "use strict";

  function serialize(form) {
    return new URLSearchParams(new FormData(form)).toString();
  }

  // The swapped fragment is HTML our own server rendered with maud, which
  // escapes every dynamic value; requests are same-origin only (cookie + Host +
  // Origin guards). So innerHTML here is server-trusted, escaped output — the
  // same model htmx uses. Never swap in content from any other origin.
  function swap(selector, html) {
    var el = document.querySelector(selector);
    if (el) el.innerHTML = html;
  }

  function parseTrigger(spec) {
    // e.g. "keyup changed delay:400ms, change" -> { events:[...], delay:400 }
    var delay = 0;
    var m = spec.match(/delay:(\d+)ms/);
    if (m) delay = parseInt(m[1], 10);
    var events = spec.split(",").map(function (clause) {
      return clause.trim().split(/\s+/)[0];
    });
    return { events: events, delay: delay };
  }

  function bind(el) {
    var url = el.getAttribute("hx-post");
    if (!url) return;
    var target = el.getAttribute("hx-target");
    var trig = parseTrigger(el.getAttribute("hx-trigger") || "change");
    var form = el.tagName === "FORM" ? el : el.closest("form");
    var timer;

    function fire() {
      clearTimeout(timer);
      timer = setTimeout(function () {
        fetch(url, {
          method: "POST",
          headers: { "Content-Type": "application/x-www-form-urlencoded" },
          body: form ? serialize(form) : "",
        })
          .then(function (r) { return r.text(); })
          .then(function (t) { if (target) swap(target, t); })
          .catch(function () { /* leave the last good fragment in place */ });
      }, trig.delay);
    }

    trig.events.forEach(function (ev) { el.addEventListener(ev, fire); });
  }

  document.addEventListener("DOMContentLoaded", function () {
    document.querySelectorAll("[hx-post]").forEach(bind);
  });
})();
