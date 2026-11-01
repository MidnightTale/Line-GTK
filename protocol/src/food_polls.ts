import { join } from "@std/path";
import { atomicWriteJson, readPrivateTextFile } from "./storage.ts";
import { findFoodPlaces, FOOD_ADDRESS } from "./food_places.ts";

export type FoodPollConfig = {
  flagged: boolean;
  address: string;
  title: string;
  time: string;
  weekdays: number[];
  optionCount: number;
};
export const defaultFoodPoll = (): FoodPollConfig => ({
  flagged: false,
  address: FOOD_ADDRESS,
  title: "Hôm nay ăn gì?",
  time: "10:00",
  weekdays: [1, 2, 3, 4, 5],
  optionCount: 6,
});

export function foodPollConfig(value: unknown): FoodPollConfig {
  const v = value as Partial<FoodPollConfig> | null;
  if (
    !v || typeof v !== "object" || typeof v.flagged !== "boolean" ||
    typeof v.address !== "string" || !v.address.trim() ||
    v.address.length > 500 ||
    typeof v.title !== "string" || !v.title.trim() || v.title.length > 200 ||
    typeof v.time !== "string" || !/^([01]\d|2[0-3]):[0-5]\d$/.test(v.time) ||
    !Array.isArray(v.weekdays) || !v.weekdays.length ||
    v.weekdays.some((day) => !Number.isInteger(day) || day < 0 || day > 6) ||
    !Number.isInteger(v.optionCount) || v.optionCount! < 2 ||
    v.optionCount! > 10
  ) {
    throw new Error(
      "Cấu hình không hợp lệ: kiểm tra địa chỉ, tiêu đề, giờ HH:mm, ngày và số quán (2–10).",
    );
  }
  return {
    flagged: v.flagged,
    address: v.address.trim(),
    title: v.title.trim(),
    time: v.time,
    weekdays: [...new Set(v.weekdays)],
    optionCount: v.optionCount!,
  };
}

type Store = { apiKey: string; groups: Record<string, FoodPollConfig> };
type Runtime = {
  directory: string;
  account: () => string;
  isGroup: (mid: string) => boolean;
  request?: typeof fetch;
};

/** Preparation only. Sending is deliberately absent until the poll transport is chosen. */
export function createFoodPollService(runtime: Runtime) {
  let queue: Promise<unknown> = Promise.resolve();
  const serial = <T>(operation: () => Promise<T>): Promise<T> => {
    const task = queue.then(operation);
    queue = task.catch(() => {});
    return task;
  };
  function session(mid: string) {
    const account = runtime.account();
    if (!account || !/^u[a-zA-Z0-9_-]+$/.test(account)) {
      throw new Error("Cần đăng nhập LINE.");
    }
    if (!mid.startsWith("c") || !runtime.isGroup(mid)) {
      throw new Error("Chỉ cấu hình cho nhóm đang tham gia.");
    }
    return {
      path: join(runtime.directory, `food-polls-${account}.json`),
      check() {
        if (runtime.account() !== account) {
          throw new Error("Tài khoản đã thay đổi. Hãy mở lại cấu hình.");
        }
      },
    };
  }
  async function load(path: string): Promise<Store> {
    try {
      const data = JSON.parse(await readPrivateTextFile(path));
      if (
        typeof data.apiKey !== "string" || !data.groups ||
        typeof data.groups !== "object"
      ) throw new Error();
      return data;
    } catch (error) {
      if (error instanceof Deno.errors.NotFound) {
        return { apiKey: "", groups: {} };
      }
      throw new Error(
        "Không đọc được cấu hình poll đã lưu; không ghi đè dữ liệu.",
      );
    }
  }
  async function read(mid: string) {
    const s = session(mid);
    const store = await load(s.path);
    s.check();
    return {
      config: store.groups[mid]
        ? foodPollConfig(store.groups[mid])
        : defaultFoodPoll(),
      hasApiKey: !!store.apiKey,
      sendingSupported: false,
    };
  }
  async function save(mid: string, config: unknown, apiKey?: unknown) {
    const s = session(mid);
    const valid = foodPollConfig(config);
    if (
      apiKey !== undefined &&
      (typeof apiKey !== "string" || apiKey.length > 500)
    ) throw new Error("API key không hợp lệ.");
    return await serial(async () => {
      s.check();
      const store = await load(s.path);
      s.check();
      store.groups[mid] = valid;
      // Omitted key preserves credentials. An explicit empty string deletes them.
      if (typeof apiKey === "string") store.apiKey = apiKey.trim();
      await atomicWriteJson(s.path, store);
      s.check();
      return {
        config: valid,
        hasApiKey: !!store.apiKey,
        sendingSupported: false,
      };
    });
  }
  async function preview(mid: string, config: unknown, apiKey?: unknown) {
    const s = session(mid);
    const valid = foodPollConfig(config);
    const store = await load(s.path);
    s.check();
    const key = typeof apiKey === "string" && apiKey.trim()
      ? apiKey.trim()
      : store.apiKey;
    const places = await findFoodPlaces(key, valid.address, runtime.request);
    s.check();
    const options = places.restaurants.slice(0, valid.optionCount);
    return {
      title: valid.title,
      resolvedAddress: places.resolvedAddress,
      center: places.center,
      radiusMeters: 5000,
      source: "Google Maps",
      options,
      text: `${valid.title}\n\n${
        options.map((r, i) =>
          `${i + 1}. ${r.name} (${
            (r.distanceMeters / 1000).toFixed(1)
          } km)\n${r.address}\n${r.mapsUrl}`
        ).join("\n\n")
      }\n\nNguồn: Google Maps • bán kính 5 km (đường chim bay).`,
    };
  }
  return { read, save, preview };
}
