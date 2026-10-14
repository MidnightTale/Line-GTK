import type { Client } from "@evex/linejs";

type GroupApi = Pick<Client["base"]["talk"], "getChats" | "getAllChatMids">;
export type GroupProfile = {
  mid: string;
  name: string;
  picturePath: string | null;
  kind: "group";
  muted: boolean;
};

export function groupProfile(raw: {
  chatMid?: string;
  chatName?: string;
  picturePath?: string;
  notificationDisabled?: boolean;
}): GroupProfile | null {
  const mid = raw.chatMid ?? "";
  if (!mid.startsWith("c")) return null;
  return {
    mid,
    name: raw.chatName?.trim() ? raw.chatName : mid,
    picturePath: raw.picturePath || null,
    kind: "group",
    muted: !!raw.notificationDisabled,
  };
}

export async function fetchGroupProfiles(
  talk: GroupApi,
  recentMids: string[],
  onFailure: () => void,
): Promise<GroupProfile[]> {
  const mids = new Set(recentMids.filter((mid) => mid.startsWith("c")));
  try {
    const joined = await talk.getAllChatMids({
      request: { withMemberChats: true },
      syncReason: "INTERNAL",
    });
    for (const mid of joined.memberChatMids ?? []) {
      if (mid.startsWith("c")) mids.add(mid);
    }
  } catch {
    // 全件の一覧が取れなくても、最近のトークの名前を解決する。
    onFailure();
  }

  const profiles: GroupProfile[] = [];
  const readBatch = async (chatMids: string[]) => {
    const result = await talk.getChats({
      chatMids,
      withMembers: false,
      withInvitees: false,
    });
    for (const raw of result.chats ?? []) {
      const profile = groupProfile(raw);
      if (profile) profiles.push(profile);
    }
  };
  const ids = [...mids];
  // 大量のIDを一度に渡すと全件失敗するため、小分けにして取得する。
  for (let index = 0; index < ids.length; index += 5) {
    const batch = ids.slice(index, index + 5);
    try {
      await readBatch(batch);
    } catch {
      onFailure();
      // 退会済みなどの1件が、同じバッチの正常なグループを巻き込まないようにする。
      for (const mid of batch) {
        try {
          await readBatch([mid]);
        } catch { /* 次のグループへ進む */ }
      }
    }
  }
  return profiles;
}
