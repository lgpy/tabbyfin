// Runs in the head, before the first paint: marks the page as having JavaScript (the styles show
// the theme switch and the screens as tabs only then), and sets the saved theme so it never flashes
// the other one.
document.documentElement.classList.add('js');
try {
  var saved = localStorage.getItem('tabbyfin-theme');
  if (saved === 'light' || saved === 'dark') document.documentElement.dataset.theme = saved;
} catch (e) {}
