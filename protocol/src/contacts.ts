export function friendName(user: { mid: string; raw: unknown }): string {
  const raw = user.raw as { contact?: { displayName?: string } };
  return raw.contact?.displayName || user.mid;
}

export function profileUrl(picturePath?: string | null): string | null {
  if (!picturePath) return null;
  if (picturePath.startsWith("http")) return picturePath;
  return `https://profile.line-scdn.net${
    picturePath.startsWith("/") ? "" : "/"
  }${picturePath}`;
}

export function picturePathOf(
  profile: Record<string, unknown> | null | undefined,
): string | null {
  if (!profile) return null;
  const candidates: unknown[] = [
    profile.picturePath,
    profile.pictureStatus,
    (profile as { picture?: { path?: string } }).picture?.path,
    (profile as { raw?: { contact?: { picturePath?: string } } }).raw?.contact
      ?.picturePath,
  ];
  for (const raw of candidates) {
    if (typeof raw === "string" && raw.trim()) return raw.trim();
  }
  return null;
}
