// Lignes de journal « parlantes ». Une seule copie, importée par le serveur
// (src/logs.js : ligne qui explique un arrêt) et par l'interface (filtre du journal).
export const ERROR_LINE = /error|erreur|exception|introuvable|n'est pas reconnu|EADDRINUSE|EACCES|cannot|impossible|failed|fatal/i;
// Avertissements : affichés avec les erreurs dans le filtre, mais jamais choisis
// comme cause d'un arrêt (une dépréciation après la vraie erreur la masquerait).
export const WARN_LINE = /warn|avertissement/i;
