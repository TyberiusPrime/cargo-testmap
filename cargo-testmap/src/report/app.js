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
