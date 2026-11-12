// SPDX-License-Identifier: AGPL-3.0-or-later
// Dictaphone : adaptation d'un texte dicté à l'endroit du curseur où il s'insère.

/** Mots qui ne prennent pas de majuscule au milieu d'une phrase (insertion au curseur). */
const LOWER_IN_SENTENCE = new Set(
  (
    "je j tu il elle on nous vous ils elles le la les l un une des de du d au aux à et ou mais donc car or ni " +
    "que qu qui ce c cet cette ces en dans pour par sur sous avec sans si puis alors ainsi est sont a ont " +
    "mon ma mes ton ta tes son sa ses notre nos votre vos leur leurs ne n se s me m y comme lorsque quand"
  ).split(" "),
);

/** Adapte le texte dicté à l'endroit où il s'insère : espace avant, majuscule ou non. */
export function fitInsertion(before: string, after: string, dictated: string): string {
  let text = dictated.trim();
  if (!text) return "";
  const prev = before.replace(/[ \t]+$/, "");
  const lastChar = prev.slice(-1);
  const sentenceStart = prev === "" || /[.!?…\n]$/.test(prev);
  if (!sentenceStart) {
    const first = text.split(/[\s,;:.!?]/)[0];
    const head = first.toLowerCase().split(/['’]/)[0];
    if (LOWER_IN_SENTENCE.has(head)) text = text.charAt(0).toLowerCase() + text.slice(1);
    // Une phrase complète insérée au milieu : son point final n'a pas lieu d'être.
    if (after.trim() !== "" && !/^[\n]/.test(after) && /[^.]\.$/.test(text)) text = text.slice(0, -1);
  }
  const needsSpaceBefore = before !== "" && !/\s$/.test(before) && !/^[,.;:!?…)\]»]/.test(text) && lastChar !== "(";
  const needsSpaceAfter = after !== "" && !/^[\s,.;:!?…)\]]/.test(after);
  return (needsSpaceBefore ? " " : "") + text + (needsSpaceAfter ? " " : "");
}
