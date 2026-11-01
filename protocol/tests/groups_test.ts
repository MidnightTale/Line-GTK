import { assert, assertEquals, assertRejects } from "@std/assert";
import {
  fetchGroupProfiles,
  groupProfile,
  setGroupMuted,
} from "../src/groups.ts";
import { conversationMid } from "../src/messages.ts";

Deno.test("115グループを上限以下のバッチで取得し、名前と画像を保持する", async () => {
  const mids = Array.from({ length: 115 }, (_, i) => `c-group-${i}`);
  const batches: string[][] = [];
  const groups = await fetchGroupProfiles(
    {
      getAllChatMids: () =>
        Promise.resolve({ memberChatMids: mids, invitedChatMids: [] }),
      getChats: ({ chatMids, withMembers, withInvitees }) => {
        assert(chatMids.length <= 5);
        assertEquals(withMembers, false);
        assertEquals(withInvitees, false);
        batches.push(chatMids);
        return Promise.resolve(
          {
            chats: chatMids.map((mid) => ({
              chatMid: mid,
              chatName: `グループ${mid.slice(8)}`,
              picturePath: "/画像",
              notificationDisabled: true,
            })),
          } as never,
        );
      },
    },
    [mids[0], "u-friend"],
    () => {
      throw new Error("正常な取得が失敗しました");
    },
  );
  assertEquals(groups.length, 115);
  assertEquals(batches.length, 23);
  assertEquals(groups[0], {
    mid: mids[0],
    name: "グループ0",
    picturePath: "/画像",
    kind: "group",
    muted: true,
  });
});

Deno.test("一覧の取得失敗や退会済みグループが、正常なグループ名を巻き込まない", async () => {
  let failures = 0;
  const groups = await fetchGroupProfiles(
    {
      getAllChatMids: () => Promise.reject(new Error("一覧取得不可")),
      getChats: ({ chatMids }) => {
        if (chatMids.includes("c-removed")) {
          return Promise.reject(
            new Error("退会済み"),
          );
        }
        return Promise.resolve(
          {
            chats: chatMids.map((mid) => ({
              chatMid: mid,
              chatName: "取得できた名前",
            })),
          } as never,
        );
      },
    },
    ["c-good", "c-removed"],
    () => {
      failures++;
    },
  );
  assertEquals(groups.map((group) => group.name), ["取得できた名前"]);
  assertEquals(failures, 2);
  assertEquals(groupProfile({ chatMid: "u-friend", chatName: "個人" }), null);
});

Deno.test("グループの受信・送信を同じトークに振り分け、個人のDMと分ける", () => {
  assertEquals(
    conversationMid({ mine: false, from: "u-friend", to: "c-group" }),
    "c-group",
  );
  assertEquals(
    conversationMid({ mine: true, from: "u-self", to: "c-group" }),
    "c-group",
  );
  assertEquals(
    conversationMid({ mine: false, from: "u-friend", to: "u-self" }),
    "u-friend",
  );
  assertEquals(
    conversationMid({ mine: true, from: "u-self", to: "u-friend" }),
    "u-friend",
  );
});

Deno.test("group notification toggle sends the chat setting and propagates failure", async () => {
  const requests: unknown[] = [];
  const talk = {
    updateChat: (request: unknown) => {
      requests.push(request);
      return Promise.resolve({} as never);
    },
  };
  await setGroupMuted(talk, "c-group", true, 17);
  await setGroupMuted(talk, "c-group", false, 18);
  assertEquals(requests, [
    {
      request: {
        reqSeq: 17,
        chat: { chatMid: "c-group", notificationDisabled: true },
        updatedAttribute: "NOTIFICATION_SETTING",
      },
    },
    {
      request: {
        reqSeq: 18,
        chat: { chatMid: "c-group", notificationDisabled: false },
        updatedAttribute: "NOTIFICATION_SETTING",
      },
    },
  ]);
  await assertRejects(
    () =>
      setGroupMuted(
        { updateChat: () => Promise.reject(new Error("server rejected")) },
        "c-group",
        true,
        19,
      ),
    Error,
    "server rejected",
  );
});
