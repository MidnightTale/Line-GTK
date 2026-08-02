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

/** Convert an OpenChat OBS image hash into its public CDN object URL. */
export function squareObsUrl(obsHash?: string | null): string | null {
  const value = obsHash?.trim();
  if (!value) return null;
  if (value.startsWith("http")) return value;
  // Appending a profile-style resize suffix makes current Square OBS objects
  // return HTTP 400. The hash itself is the complete public object path.
  return `https://obs.line-scdn.net/${value}`;
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
