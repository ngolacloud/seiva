// Ctrl+K / ⌘K opens the search, and the placeholder advertises the shortcut —
// matching the djangoproject.com docs "Search (Ctrl+K)" affordance. Material's
// own "/", "s", "f" shortcuts keep working; this only adds Ctrl/Cmd+K.

(function () {
  "use strict";

  var isMac =
    navigator.platform.toUpperCase().indexOf("MAC") >= 0 ||
    /Mac|iPhone|iPad/i.test(navigator.userAgent);
  var hint = isMac ? "⌘K" : "Ctrl+K";
  // European-Portuguese search verb when the page is in Portuguese.
  var isPt = (document.documentElement.lang || "").toLowerCase().indexOf("pt") === 0;
  var verb = isPt ? "Pesquisar" : "Search";

  function setPlaceholder() {
    var input = document.querySelector(".md-search__input");
    if (input) input.setAttribute("placeholder", verb + "  " + hint);
  }

  function openSearch() {
    var toggle = document.querySelector("[data-md-toggle='search']");
    var input = document.querySelector(".md-search__input");
    if (toggle && !toggle.checked) toggle.checked = true;
    if (input) {
      input.focus();
      input.select();
    }
  }

  document.addEventListener("keydown", function (e) {
    if ((e.ctrlKey || e.metaKey) && (e.key === "k" || e.key === "K")) {
      e.preventDefault();
      openSearch();
    }
  });

  if (document.readyState !== "loading") setPlaceholder();
  else document.addEventListener("DOMContentLoaded", setPlaceholder);
})();
