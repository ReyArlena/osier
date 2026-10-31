// Osier website: hero word rotator, copy buttons, reveal on scroll. No dependencies.
(function () {
  "use strict";

  var reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  // ----- hero word rotator: type, hold, backspace, next -----
  var word = document.querySelector(".rotator .word");
  if (word && !reduceMotion) {
    var words = (word.getAttribute("data-words") || "").split("|").filter(Boolean);
    var index = 0;
    var text = words[0] || "";
    var typing = false;
    var TYPE_MS = 75;
    var DELETE_MS = 38;
    var HOLD_MS = 1900;
    var GAP_MS = 260;

    var step = function () {
      if (document.hidden) {
        setTimeout(step, 500);
        return;
      }
      var target = words[index];
      if (typing) {
        if (text.length < target.length) {
          text = target.slice(0, text.length + 1);
          word.textContent = text;
          setTimeout(step, TYPE_MS);
        } else {
          typing = false;
          setTimeout(step, HOLD_MS);
        }
      } else if (text.length > 0) {
        text = text.slice(0, -1);
        word.textContent = text;
        setTimeout(step, DELETE_MS);
      } else {
        index = (index + 1) % words.length;
        typing = true;
        setTimeout(step, GAP_MS);
      }
    };
    if (words.length > 1) setTimeout(step, HOLD_MS + 600);
  }

  // ----- copy buttons -----
  var copyText = function (value) {
    if (navigator.clipboard && window.isSecureContext) {
      return navigator.clipboard.writeText(value);
    }
    return new Promise(function (resolve, reject) {
      var area = document.createElement("textarea");
      area.value = value;
      area.setAttribute("readonly", "");
      area.style.position = "fixed";
      area.style.opacity = "0";
      document.body.appendChild(area);
      area.select();
      try {
        document.execCommand("copy") ? resolve() : reject(new Error("copy failed"));
      } catch (e) {
        reject(e);
      }
      document.body.removeChild(area);
    });
  };

  Array.prototype.forEach.call(document.querySelectorAll(".copy"), function (button) {
    var label = button.querySelector("span");
    button.addEventListener("click", function () {
      var source = button.parentElement.querySelector("[data-copy-source]");
      if (!source) return;
      copyText(source.textContent.trim()).then(
        function () {
          button.classList.add("done");
          if (label) label.textContent = "Copied";
          setTimeout(function () {
            button.classList.remove("done");
            if (label) label.textContent = "Copy";
          }, 1600);
        },
        function () {
          if (label) label.textContent = "Select it";
        }
      );
    });
  });

  // ----- links for things that do not exist yet -----
  Array.prototype.forEach.call(document.querySelectorAll("a.pending"), function (link) {
    link.addEventListener("click", function (event) {
      event.preventDefault();
    });
  });

  // ----- reveal on scroll -----
  var revealables = document.querySelectorAll(".reveal");
  if (reduceMotion || !("IntersectionObserver" in window)) {
    Array.prototype.forEach.call(revealables, function (el) {
      el.classList.add("in");
    });
    return;
  }
  var observer = new IntersectionObserver(
    function (entries) {
      entries.forEach(function (entry) {
        if (entry.isIntersecting) {
          entry.target.classList.add("in");
          observer.unobserve(entry.target);
        }
      });
    },
    { rootMargin: "0px 0px -8% 0px", threshold: 0.08 }
  );
  Array.prototype.forEach.call(revealables, function (el) {
    observer.observe(el);
  });
})();
