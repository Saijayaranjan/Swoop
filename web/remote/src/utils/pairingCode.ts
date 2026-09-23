// Pairing codes are 8 unambiguous alphanumeric characters, displayed/typed as "ABCD-EFGH".

const UNAMBIGUOUS_ALPHABET = /[^0-9A-Z]/g;

/**
 * Auto-format free-typed pairing-code input into "ABCD-EFGH": uppercases, strips anything
 * that isn't alphanumeric, inserts the dash after the 4th character, and truncates to 8 chars.
 * Safe to call on every keystroke (idempotent on already-formatted input).
 */
export function formatPairingCode(input: string): string {
  const cleaned = input.toUpperCase().replace(UNAMBIGUOUS_ALPHABET, "").slice(0, 8);
  if (cleaned.length <= 4) {
    return cleaned;
  }
  return `${cleaned.slice(0, 4)}-${cleaned.slice(4)}`;
}

/** True when `code` is a well-formed "ABCD-EFGH" pairing code (8 chars either side of the dash). */
export function isValidPairingCode(code: string): boolean {
  return /^[0-9A-Z]{4}-[0-9A-Z]{4}$/.test(code);
}

/** Strip the dash for submission if the server expects the raw code; here we keep it, matching docs. */
export function normalizePairingCode(code: string): string {
  return formatPairingCode(code);
}
