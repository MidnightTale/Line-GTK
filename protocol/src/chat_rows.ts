export type ChatRow = {
  mid: string;
  name: string;
  kind: string;
  avatarPath: string | null;
  picturePath?: string | null;
  lastActivity: number;
  unread: number;
  preview: string;
  muted?: boolean;
  contactOnly?: boolean;
};

type ContactInfo = {
  name: string;
  picturePath: string | null;
  kind: string;
  muted: boolean;
};

/** Profile and friend events are not conversation activity. */
export function contactChatRow(
  mid: string,
  info: ContactInfo,
  previous?: ChatRow,
  avatarPath: string | null = null,
): ChatRow {
  return {
    mid,
    lastActivity: 0,
    unread: 0,
    preview: "",
    contactOnly: previous
      ? previous.contactOnly
      : !mid.startsWith("c") && !mid.startsWith("r"),
    avatarPath,
    ...previous,
    name: info.name || previous?.name || mid,
    kind: info.kind || previous?.kind || "dm",
    picturePath: info.picturePath ?? previous?.picturePath ?? null,
    muted: info.muted,
  };
}
