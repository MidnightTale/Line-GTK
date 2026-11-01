import type { Client } from "@evex/linejs";

export type UserProfile = {
  mid: string;
  name: string;
  statusMessage: string;
  picturePath: string | null;
  avatarPath: string | null;
  kind: string;
  isSelf: boolean;
};

type Runtime = {
  getClient: () => Client | null;
  selfMid: () => string;
  avatarPathFor: (
    mid: string,
    picturePath?: string | null,
  ) => Promise<string | null>;
  kindOf: (contact: unknown) => string;
  emit: (event: string, payload: Record<string, unknown>) => void;
  now?: () => number;
};

const TTL_MS = 600_000;
const MAX_ENTRIES = 500;

export function isProfileMid(value: unknown): value is string {
  return typeof value === "string" && /^u[a-zA-Z0-9_-]{1,128}$/.test(value);
}

function text(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

export function createUserProfiles(runtime: Runtime) {
  const now = runtime.now ?? Date.now;
  let owner: Client | null = null;
  let ownerMid = "";
  const cache = new Map<string, { profile: UserProfile; until: number }>();
  const inflight = new Map<string, Promise<UserProfile>>();

  function sync() {
    const client = runtime.getClient();
    const mid = runtime.selfMid();
    if (client !== owner || mid !== ownerMid) {
      owner = client;
      ownerMid = mid;
      cache.clear();
      inflight.clear();
    }
    if (!client || !mid) throw new Error("user_profile_not_logged_in");
    return { client, self: mid };
  }

  async function load(
    client: Client,
    self: string,
    mid: string,
  ): Promise<UserProfile> {
    const contact = await client.base.talk.getContact({ mid });
    const picturePath = contact.picturePath || null;
    const profile: UserProfile = {
      mid,
      name: text(contact.displayNameOverridden) || text(contact.displayName) ||
        mid,
      statusMessage: text(contact.statusMessage),
      picturePath,
      avatarPath: null,
      kind: runtime.kindOf(contact),
      isSelf: mid === self,
    };
    try {
      profile.avatarPath = await runtime.avatarPathFor(mid, picturePath);
    } catch {
      profile.avatarPath = null;
    }
    if (profile.avatarPath) {
      runtime.emit("avatar_ready", { mid, avatarPath: profile.avatarPath });
    }
    return profile;
  }

  async function get(mid: unknown, force = false): Promise<UserProfile> {
    if (!isProfileMid(mid)) throw new Error("user_profile_invalid_user");
    const { client, self } = sync();
    if (!force) {
      const hit = cache.get(mid);
      if (hit && hit.until > now()) return hit.profile;
      const pending = inflight.get(mid);
      if (pending) return await pending;
    }
    const task = load(client, self, mid).then((profile) => {
      if (owner === client && ownerMid === self) {
        cache.delete(mid);
        cache.set(mid, { profile, until: now() + TTL_MS });
        while (cache.size > MAX_ENTRIES) {
          cache.delete(cache.keys().next().value!);
        }
      }
      return profile;
    }).finally(() => {
      if (inflight.get(mid) === task) inflight.delete(mid);
    });
    inflight.set(mid, task);
    return await task;
  }

  return { get };
}
