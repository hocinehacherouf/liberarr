/** Lowercased without accents, to compare "reunion" with "Réunion". */
export const fold = (s: string) => s.normalize('NFD').replace(/\p{Diacritic}/gu, '').toLowerCase()
