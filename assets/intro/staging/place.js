// Keeps the word to select (.pick) the same distance from the window's right
// edge on every page, so the card, which opens under the selection with its
// text lined up with it, spills out of the window by the same amount on both
// screenshots. ?edge=220 changes the distance, in CSS pixels, and
// ?theme=dark or ?theme=light overrides Windows' setting.
(() => {
  const params = new URLSearchParams(location.search);
  const edge = Number(params.get("edge")) || 220;
  const theme = params.get("theme");
  if (theme === "dark" || theme === "light") document.documentElement.dataset.theme = theme;
  const page = document.querySelector(".page");
  const main = document.querySelector("main");
  const pick = document.querySelector(".pick");

  function fit() {
    // A hand-set line must never wrap: shrink the type until none do.
    let size = 19;
    page.style.fontSize = size + "px";
    const wraps = () => [...page.querySelectorAll(".l")].some(
      (l) => l.getClientRects().length > 1 || l.offsetHeight > size * 1.7 * 1.5);
    while (wraps() && size > 14) page.style.fontSize = (size -= 0.5) + "px";
  }

  function place() {
    // Every pixel of padding moves the word a pixel further from the edge.
    main.style.paddingRight = "0px";
    const from = document.documentElement.clientWidth - pick.getBoundingClientRect().left;
    main.style.paddingRight = Math.max(24, edge - from) + "px";
  }

  fit();
  place();
  addEventListener("resize", place);
})();
