// Set the theme before first paint so dark mode does not flash light.
(function () {
  try {
    var t = localStorage.getItem('theme') || 'auto';
    var dark = t === 'dark' || (t === 'auto' && matchMedia('(prefers-color-scheme: dark)').matches);
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  } catch (e) {}
})();
