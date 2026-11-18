/** Formats a base-unit amount with thousands separators and trimmed zeros. */
export function formatUnits(value: bigint, decimals: number): string {
  const negative = value < 0n;
  const abs = negative ? -value : value;
  const base = 10n ** BigInt(decimals);
  const whole = (abs / base).toString().replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const frac = (abs % base)
    .toString()
    .padStart(decimals, "0")
    .replace(/0+$/, "");
  return `${negative ? "-" : ""}${whole}${frac === "" ? "" : `.${frac}`}`;
}

/** Plain decimal string without separators, for filling an input. */
export function toInputString(value: bigint, decimals: number): string {
  return formatUnits(value, decimals).replaceAll(",", "");
}

/** Parses a decimal string into base units. Returns null when invalid. */
export function parseUnits(text: string, decimals: number): bigint | null {
  const t = text.trim();
  if (!/^\d*\.?\d*$/.test(t) || t === "" || t === ".") return null;
  const [whole = "", frac = ""] = t.split(".");
  if (frac.length > decimals) return null;
  return (
    BigInt(whole === "" ? "0" : whole) * 10n ** BigInt(decimals) +
    BigInt(frac.padEnd(decimals, "0") === "" ? "0" : frac.padEnd(decimals, "0"))
  );
}

export function shortAddress(address: string): string {
  return `${address.slice(0, 4)}...${address.slice(-4)}`;
}
