export const DAY_MS = 86_400_000;
export const WEEK_MS = 7 * DAY_MS;
export const MONTH_MS = 30 * DAY_MS;
export const FOREVER_MS = Number.MAX_SAFE_INTEGER;

export type CachePolicy = {
  memChat: number;
  memMsg: number;
  diskChat: number;
  diskMsg: number;
  ownedPack: number;
  contactsRefresh: number;
  /** 0 = never expire miss markers. */
  animMiss: number;
};

export function policyFor(retention: string): CachePolicy {
  switch ((retention || "smart").toLowerCase()) {
    case "day":
      return policy(DAY_MS, DAY_MS, 30, 15, 30, DAY_MS);
    case "week":
      return policy(WEEK_MS, WEEK_MS, 60, 30, 120, WEEK_MS);
    case "month":
      return policy(MONTH_MS, MONTH_MS, 120, 60, 360, MONTH_MS);
    case "forever":
      return policy(FOREVER_MS, WEEK_MS, 360, 120, 360, 0);
    default:
      return policy(14 * DAY_MS, DAY_MS, 30, 20, 30, WEEK_MS, 30 * DAY_MS);
  }
}

function policy(
  diskChatTtl: number,
  ownedPackTtl: number,
  memChatMinutes: number,
  memMessageMinutes: number,
  contactsMinutes: number,
  animationMissTtl: number,
  diskMessageTtl = diskChatTtl,
): CachePolicy {
  return {
    memChat: memChatMinutes * 60_000,
    memMsg: memMessageMinutes * 60_000,
    diskChat: diskChatTtl,
    diskMsg: diskMessageTtl,
    ownedPack: ownedPackTtl,
    contactsRefresh: contactsMinutes * 60_000,
    animMiss: animationMissTtl,
  };
}
