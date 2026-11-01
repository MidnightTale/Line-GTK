import { assertEquals, assertRejects, assertStrictEquals } from "@std/assert";
import type { Client } from "@evex/linejs";
import { createUnsendService, unsendOperation } from "../src/unsend.ts";

type Json = Record<string, unknown>;
function fixture(dataDir: string) {
  const original: Json = {
    id: "9007199254740993",
    from: "u-self",
    to: "u-peer",
    mine: true,
    text: "残さない本文",
    createdTime: Date.now(),
    contentType: "IMAGE",
    imagePath: "/秘密の画像.png",
    replyTo: { text: "元の引用" },
  };
  let sequence: () => Promise<number> = () => Promise.resolve(7);
  let request: () => Promise<void> = () => Promise.resolve();
  const calls: unknown[] = [];
  const events: Json[] = [];
  const changes: unknown[] = [];
  let current: Client | null = {
    base: {
      profile: { mid: "u-self" },
      getReqseq: () => sequence(),
      talk: {
        unsendMessage: async (args: unknown) => {
          calls.push(args);
          await request();
        },
      },
    },
  } as unknown as Client;
  const runtime = {
    dataDir,
    getClient: () => current,
    findMessage: () => Promise.resolve(original),
    changed: (mid: string, id: string) => {
      changes.push([mid, id]);
      return Promise.resolve();
    },
    emit: (event: string, payload: Json) => {
      events.push({ event, ...payload });
    },
  };
  return {
    service: createUnsendService(runtime),
    runtime,
    original,
    calls,
    events,
    changes,
    request: (fn: typeof request) => {
      request = fn;
    },
    sequence: (fn: typeof sequence) => {
      sequence = fn;
    },
    account: (mid: string | null) => {
      current = mid ? { base: { profile: { mid } } } as Client : null;
    },
  };
}

Deno.test("自分の送信を正しいIDと順序番号で取り消し、本文と画像と引用を履歴から除く", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    await f.service.unsend(
      "u-peer",
      f.original.id,
      Number(f.original.createdTime),
    );
    assertEquals(f.calls, [{ seq: 7, messageId: "9007199254740993" }]);
    assertEquals(f.events, [{
      event: "message_unsent",
      accountMid: "u-self",
      chatMid: "u-peer",
      messageId: "9007199254740993",
    }]);
    const reply = {
      id: "2",
      from: "u-peer",
      to: "u-self",
      replyTo: {
        messageId: f.original.id,
        text: "残さない本文",
        contentType: "IMAGE",
      },
    };
    const result = await f.service.mask("u-peer", [f.original, reply]);
    assertEquals(result[0].contentType, "UNSENT");
    assertEquals(result[0].imagePath, undefined);
    assertEquals(result[0].replyTo, undefined);
    assertEquals(result[1].replyTo, {
      messageId: f.original.id,
      text: "",
      contentType: "UNAVAILABLE",
    });
    const restored = createUnsendService(f.runtime);
    assertEquals(
      (await restored.mask("u-peer", [f.original]))[0].contentType,
      "UNSENT",
    );
    const disk = await Deno.readTextFile(dir + "/unsent-u-self.json");
    assertEquals(disk.includes("残さない本文") || disk.includes("秘密"), false);
    assertEquals(
      (await restored.mask("u-other", [f.original]))[0].text,
      "残さない本文",
    );
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

for (const kind of ["相手", "別のトーク", "CALL", "CHAT_EVENT", "UNSENT"]) {
  Deno.test(`${kind}のメッセージを送信取消APIへ渡さない`, async () => {
    const dir = await Deno.makeTempDir();
    try {
      const f = fixture(dir);
      if (kind === "相手") f.original.from = "u-peer";
      else if (kind === "別のトーク") f.original.to = "u-other";
      else f.original.contentType = kind;
      await assertRejects(
        () => f.service.unsend("u-peer", f.original.id, 1),
        Error,
        "unsend_not_own_message",
      );
      assertEquals(f.calls, []);
      assertEquals(f.events, []);
    } finally {
      await Deno.remove(dir, { recursive: true });
    }
  });
}

Deno.test("サーバーが期限切れや通信失敗を返した場合は本文を消さず再試行できる", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    f.request(() => Promise.reject(new Error("UNSEND_TIME_LIMIT")));
    await assertRejects(
      () => f.service.unsend("u-peer", f.original.id, 1),
      Error,
      "UNSEND_TIME_LIMIT",
    );
    assertEquals(f.events, []);
    assertEquals(
      (await f.service.mask("u-peer", [f.original]))[0].text,
      "残さない本文",
    );
    f.request(() => Promise.resolve());
    await f.service.unsend("u-peer", f.original.id, 1);
    assertEquals(f.calls.length, 2);
    assertEquals(f.events.length, 1);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("連打を一度のAPI呼び出しにまとめ、取消済みなら再送しない", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    let release!: () => void;
    const ready = Promise.withResolvers<void>();
    f.request(() => {
      ready.resolve();
      return new Promise((resolve) => release = resolve);
    });
    const first = f.service.unsend("u-peer", f.original.id, 1);
    assertStrictEquals(f.service.unsend("u-peer", f.original.id, 1), first);
    await ready.promise;
    release();
    await first;
    await f.service.unsend("u-peer", f.original.id, 1);
    assertEquals(f.calls.length, 1);
    assertEquals(f.events.length, 1);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("別端末の取消通知も保存し、同じ通知を二度反映しない", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    const operation = {
      type: 65,
      param1: "u-peer",
      param2: String(f.original.id),
    };
    assertEquals(f.service.handleOperation(operation), true);
    await f.service.flush();
    assertEquals(f.service.handleOperation(operation), true);
    await f.service.flush();
    assertEquals(f.events.length, 1);
    assertEquals(f.calls, []);
    assertEquals(
      (await f.service.mask("u-peer", [f.original]))[0].contentType,
      "UNSENT",
    );
    for (const type of [64, "DESTROY_MESSAGE", "NOTIFIED_DESTROY_MESSAGE"]) {
      assertEquals(unsendOperation({ ...operation, type }), {
        chatMid: "u-peer",
        id: f.original.id,
      });
    }
    assertEquals(
      unsendOperation({ type: 40, param1: "u-peer", param2: "10" }),
      null,
    );
    assertEquals(
      unsendOperation({ type: 65, param1: "../../path", param2: "10" }),
      null,
    );
    assertEquals(unsendOperation({ ...operation, param2: "pending-1" }), null);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("API呼び出し直前のアカウント変更を拒否する", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    f.sequence(() => {
      f.account("u-other");
      return Promise.resolve(7);
    });
    await assertRejects(
      () => f.service.unsend("u-peer", f.original.id, 1),
      Error,
      "unsend_session_changed",
    );
    assertEquals(f.calls, []);
    assertEquals(f.events, []);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("成功応答が別アカウントへの切替後に届いてもその画面へ通知せず元のアカウントに保存する", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    f.request(() => {
      f.account("u-other");
      return Promise.resolve();
    });
    await f.service.unsend("u-peer", f.original.id, 1);
    assertEquals(f.events, []);
    assertEquals(f.changes, []);
    assertEquals(
      (await f.service.mask("u-peer", [f.original]))[0].text,
      "残さない本文",
    );
    f.account("u-self");
    assertEquals(
      (await f.service.mask("u-peer", [f.original]))[0].contentType,
      "UNSENT",
    );
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});

Deno.test("仮IDや丸められたIDではAPIを呼び出さない", async () => {
  const dir = await Deno.makeTempDir();
  try {
    const f = fixture(dir);
    for (const id of ["pending-1", "0", "../x", 9007199254740993]) {
      await assertRejects(
        () => f.service.unsend("u-peer", id, 1),
        Error,
        "unsend_invalid_message",
      );
    }
    assertEquals(f.calls, []);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});
