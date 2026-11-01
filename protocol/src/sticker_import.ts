/** Accept pack IDs and official store links, never arbitrary download URLs. */
export function stickerPackageId(input: string): string | null {
  const value = input.trim();
  if (/^[1-9]\d{0,19}$/.test(value)) return value;
  try {
    const url = new URL(value);
    if (url.protocol !== "https:") return null;
    if (url.username || url.password || url.port) return null;
    const match = url.hostname === "store.line.me"
      ? url.pathname.match(/^\/stickershop\/product\/([1-9]\d{0,19})(?:\/|$)/)
      : url.hostname === "line.me"
      ? url.pathname.match(/^\/S\/sticker\/([1-9]\d{0,19})(?:\/|$)/)
      : null;
    return match?.[1] ?? null;
  } catch {
    return null;
  }
}
