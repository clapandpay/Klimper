// SPDX-License-Identifier: AGPL-3.0-or-later
// Which of the wallet's tickers for a CAT the approval may use (clapandpay TASK-139, audit M2).
//
// Whoever made a CAT chose its ticker, and the token list that gave it to the wallet takes it
// as it comes. So a ticker is used on an approval only when it is a plain ASCII word after
// Unicode compatibility folding (NFKC turns a fullwidth "ＸＣＨ" into "XCH"), and never when
// it reads as the network's coin to a human eye: Cyrillic "ХСН", Greek "ΧCΗ", "XCH.", "X-CH",
// "wXCH" and "CH1A" are all refused. The comparison is on a skeleton: compatibility-folded,
// common look-alike letters and digits mapped to the Latin letter they imitate, separators
// dropped, upper case.

/** A ticker the approval may show: ASCII letters, digits and `. $ _ -`, at most 12. */
const PLAIN_TICKER = /^[A-Za-z0-9.$_-]{1,12}$/;

/**
 * Look-alikes of Latin letters, as pairs of the look-alike and the letter it imitates. Not
 * every confusable in Unicode: the ones a CAT can wear to pass as XCH or Chia, and the common
 * ones beside them.
 */
const LOOKALIKE_PAIRS = [
  // Cyrillic
  'АAаAВBвBСCсCЕEеEНHнHһHІIіIӀIӏIЈJјJКKкKМMмMОOоOРPрPЅSѕSТTтTХXхXУYуYҮYԁDԛQԝW',
  // Greek
  'ΑAαAΒBβBΕEεEΖZΗHΙIιIΚKκKΜMΝNνVΟOοOΡPρPΤTτTΥYυUΧXχXϲCϹC',
  // Latin and symbols that pass for a letter
  'ıIłLøOßB×X¡I|I!I',
  // Digits, and the lower-case l that reads as a capital I
  '0O1IlI3E4A5S6G8B',
].join('');

const LOOKALIKES: ReadonlyMap<string, string> = (() => {
  const characters = Array.from(LOOKALIKE_PAIRS);
  const map = new Map<string, string>();
  for (let index = 0; index + 1 < characters.length; index += 2) {
    map.set(characters[index] ?? '', characters[index + 1] ?? '');
  }
  return map;
})();

/** What a ticker looks like to a reader: folded, look-alikes mapped, separators dropped. */
export function tickerSkeleton(ticker: string): string {
  let out = '';
  for (const character of ticker.normalize('NFKC')) {
    const mapped = LOOKALIKES.get(character) ?? character;
    // Marks and accents that NFKC kept apart from their letter are dropped with the rest.
    for (const folded of mapped.normalize('NFKD').toUpperCase()) {
      if (/[A-Z0-9]/.test(folded)) out += folded;
    }
  }
  return out;
}

/** The ticker reads as the network's coin: its skeleton holds XCH or CHIA anywhere. */
export function impersonatesXch(ticker: string, xchLabel = 'XCH'): boolean {
  const skeleton = tickerSkeleton(ticker);
  const label = tickerSkeleton(xchLabel);
  return (
    skeleton.includes('XCH') ||
    skeleton.includes('CHIA') ||
    (label !== '' && skeleton.includes(label))
  );
}

/**
 * The ticker as the approval may show it (compatibility-folded), or null: when there is none,
 * when it is not a plain ASCII word, or when it passes for the network's coin.
 */
export function safeTicker(
  ticker: string | null | undefined,
  xchLabel = 'XCH',
): string | null {
  if (typeof ticker !== 'string') return null;
  const folded = ticker.trim().normalize('NFKC');
  if (!PLAIN_TICKER.test(folded)) return null;
  if (impersonatesXch(folded, xchLabel)) return null;
  return folded;
}
