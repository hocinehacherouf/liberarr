/**
 * Lowercased without accents, to compare "reunion" with "Réunion". œ, æ and the curly apostrophes follow the
 * server's rule (liberarr_core::search::fold_char), so the title and transcript searches agree.
 */
export const fold = (s: string) =>
  s
    .normalize('NFD')
    .replace(/\p{Diacritic}/gu, '')
    .toLowerCase()
    .replace(/œ/g, 'oe')
    .replace(/æ/g, 'ae')
    .replace(/[’‘]/g, "'")
