// Minimal hover/click interactivity for the testmap report (see DESIGN §5.3).
//
// Page globals (set by the generated HTML):
//   window.__TESTMAP_TESTS  : array of {n,m,b,k,s}
//   window.__TESTMAP_COV    : { [filePath]: { "line": [testIdx, ...] } }
//   window.__TESTMAP_ABOVE  : { [filePath]: { "line": count } }  (>= threshold)
//   window.__TESTMAP_FILE   : the current file path (directory mode only)
//
// Each covered <tr> has a `data-line` (and `data-file` in single-file mode).

(function () {
  "use strict";

  var TESTS = window.__TESTMAP_TESTS || [];
  var COV = window.__TESTMAP_COV || {};
  var ABOVE = window.__TESTMAP_ABOVE || {};
  var CURRENT = window.__TESTMAP_FILE || null;

  var panel = document.getElementById("panel");
  var pinned = null; // the currently pinned <tr>, or null

  function rowFile(tr) {
    return tr.dataset.file || CURRENT;
  }

  function covering(tr) {
    var file = rowFile(tr);
    var line = tr.dataset.line;
    var lines = COV[file];
    if (!lines) return null;
    return lines[line] || null;
  }

  // For above-threshold lines we keep a small sample of the covering tests
  // plus the total count (the report shows the sample + an "above threshold" note).
  function aboveInfo(tr) {
    var file = rowFile(tr);
    var line = tr.dataset.line;
    var lines = ABOVE[file];
    if (!lines) return null;
    return lines[line] || null; // {total, sample:[idx,...]}
  }

  function badgeFor(t) {
    return '<span class="badge">' + esc(t.k) + "/" + esc(t.b) + "</span>";
  }

  function nameFor(t) {
    var path = t.m ? t.m + "::" + t.n : t.n;
    return '<span class="tname">' + esc(path) + "</span>";
  }

  function esc(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
  }

  function showPanel(tr, pinned_) {
    var idxs = covering(tr);
    var hint =
      "click to " + (pinned_ ? "unpin" : "pin");
    if (idxs && idxs.length > 0) {
      var items = idxs
        .map(function (i) {
          var t = TESTS[i];
          if (!t) return "";
          return (
            '<li class="' + (t.s === "failed" ? "failed" : "") + '">' +
            badgeFor(t) +
            nameFor(t) +
            "</li>"
          );
        })
        .join("");
      panel.innerHTML =
        '<div class="panel-head">Tests covering line ' +
        esc(tr.dataset.line) +
        " (" +
        idxs.length +
        ") · " +
        hint +
        "</div><ul>" +
        items +
        "</ul>";
      return;
    }
    var info = aboveInfo(tr);
    if (info) {
      var sample = info.sample || [];
      var items = sample
        .map(function (i) {
          var t = TESTS[i];
          if (!t) return "";
          return (
            '<li class="' + (t.s === "failed" ? "failed" : "") + '">' +
            badgeFor(t) +
            nameFor(t) +
            "</li>"
          );
        })
        .join("");
      var shown = sample.length;
      var total = info.total;
      panel.innerHTML =
        '<div class="panel-head">Line ' +
        esc(tr.dataset.line) +
        " · above threshold — showing " +
        shown +
        " of " +
        total +
        " test" + (total === 1 ? "" : "s") +
        " · " +
        hint +
        "</div>" +
        (items ? "<ul>" + items + "</ul>" : "") +
        '<span class="hint">Covered by too many tests to list fully. Raise --threshold to enumerate them.</span>';
      return;
    }
    panel.innerHTML =
      '<span class="hint">No mapped tests for line ' +
      esc(tr.dataset.line) +
      ".</span>";
  }

  function clearPanel() {
    panel.innerHTML =
      '<span class="hint">Hover a highlighted line to see covering tests · click to pin</span>';
  }

  function setHover(tr, on) {
    if (pinned && pinned === tr) return; // don't override pinned styling
    tr.classList.toggle("hover", on);
  }

  // Wire up only the rows that actually have coverage data.
  var rows = document.querySelectorAll("tr[data-line]");
  rows.forEach(function (tr) {
    if (!covering(tr) && !aboveInfo(tr)) return; // not an annotated line
    tr.addEventListener("mouseenter", function () {
      setHover(tr, true);
      if (!pinned) showPanel(tr, false);
    });
    tr.addEventListener("mouseleave", function () {
      setHover(tr, false);
      if (!pinned) clearPanel();
    });
    tr.addEventListener("click", function (ev) {
      ev.preventDefault();
      if (pinned === tr) {
        // Unpin.
        tr.classList.remove("pinned");
        pinned = null;
        clearPanel();
      } else {
        if (pinned) pinned.classList.remove("pinned");
        pinned = tr;
        tr.classList.add("pinned");
        showPanel(tr, true);
      }
    });
  });
})();

// Click-to-jump: the "N uncovered" / "N ignored" links scroll to the next
// matching line (cycling). Each link carries `data-jump` ("uncovered"/"ignored")
// and, on the single-file report, an optional `data-file-id` scoping it to one
// file's section. Directory index rows navigate via `?jump=` (handled below on
// load) rather than firing this handler.
(function () {
  "use strict";
  var state = {}; // "kind:scope" -> last visited index

  function candidates(kind, scope) {
    var sel = kind === "excluded"
      ? "tr.cov-excluded, tr.cov-excl-covered"
      : "tr.cov-" + kind;
    var root = scope ? document.getElementById("file-" + scope) : document;
    if (!root) return [];
    return Array.prototype.slice.call(root.querySelectorAll(sel));
  }

  function flash(tr) {
    tr.classList.remove("jump-target");
    // Force a reflow so re-adding restarts the animation (repeated jumps).
    void tr.offsetWidth;
    tr.classList.add("jump-target");
    setTimeout(function () { tr.classList.remove("jump-target"); }, 1600);
  }

  // `advance` cycles to the next match; without it we land on the first
  // (used by the `?jump=` deep link on page load).
  function jump(kind, scope, advance) {
    var list = candidates(kind, scope);
    if (!list.length) return;
    var key = kind + ":" + (scope || "");
    var last = state[key] == null ? -1 : state[key];
    var idx = advance ? (last + 1) % list.length : 0;
    state[key] = idx;
    var tr = list[idx];
    tr.scrollIntoView({ block: "center" });
    flash(tr);
  }

  function activate(a) {
    jump(a.dataset.jump, a.dataset.fileId || null, true);
  }

  document.addEventListener("click", function (e) {
    var a = e.target.closest && e.target.closest("a[data-jump]");
    if (!a) return;
    e.preventDefault();
    activate(a);
  });
  document.addEventListener("keydown", function (e) {
    if (e.key !== "Enter" && e.key !== " ") return;
    var a = e.target.closest && e.target.closest("a[data-jump]");
    if (!a) return;
    e.preventDefault();
    activate(a);
  });

  // Deep link from the directory index: ?jump=uncovered / ?jump=ignored lands
  // on the first matching line of this file.
  try {
    var j = new URLSearchParams(window.location.search).get("jump");
    if (j === "uncovered" || j === "excluded" || j === "ignored") {
      requestAnimationFrame(function () { jump(j, null, false); });
    }
  } catch (_) {}
})();

// Click-to-copy: clicking a file path in the toolbar copies it to the clipboard.
(function () {
  "use strict";
  document.querySelectorAll(".toolbar .path").forEach(function (el) {
    el.title = "click to copy path";
    el.addEventListener("click", function () {
      var text = el.textContent;
      navigator.clipboard.writeText(text).then(function () {
        el.classList.add("copied");
        setTimeout(function () { el.classList.remove("copied"); }, 1200);
      });
    });
  });
})();
