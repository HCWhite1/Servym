// Lignes de journal « parlantes », pour le filtre du journal. Même motif côté serveur
// (crates/servym-core/src/logs.rs : ligne qui explique un arrêt).
export const ERROR_LINE = /error|erreur|exception|introuvable|n['’]est pas reconnu|is not recognized|EADDRINUSE|EACCES|cannot|impossible|failed|fatal/i;
// Avertissements : affichés avec les erreurs dans le filtre, mais jamais choisis
// comme cause d'un arrêt (une dépréciation après la vraie erreur la masquerait).
export const WARN_LINE = /warn|avertissement/i;
