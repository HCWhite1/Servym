// Thème choisi dans les Paramètres, posé avant le premier affichage (script
// classique chargé dans <head>, pour éviter un flash du mauvais thème).
try {
  const theme = JSON.parse(localStorage.getItem('servynex.theme'));
  if (['light', 'dark', 'solarized-light', 'solarized-dark'].includes(theme)) document.documentElement.dataset.theme = theme;
} catch {}
