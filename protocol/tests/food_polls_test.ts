import { assert, assertEquals, assertRejects, assertThrows } from "@std/assert";
import { findFoodPlaces } from "../src/food_places.ts";
import {
  createFoodPollService,
  defaultFoodPoll,
  foodPollConfig,
} from "../src/food_polls.ts";

function googleMock(): typeof fetch {
  return ((input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    if (url.includes("geocode")) {
      assert(
        new URL(url).searchParams.get("address")?.includes("Dương Quang Đông"),
      );
      return Promise.resolve(Response.json({
        status: "OK",
        results: [{
          formatted_address: "Địa chỉ đã xác định",
          geometry: { location: { lat: 10.7, lng: 106.6 } },
        }],
      }));
    }
    const body = JSON.parse(String(init?.body));
    assertEquals(body.locationRestriction.circle.radius, 5000);
    assertEquals(body.locationRestriction.circle.center, {
      latitude: 10.7,
      longitude: 106.6,
    });
    assertEquals(
      new Headers(init?.headers).get("X-Goog-Api-Key"),
      "test-secret",
    );
    const place = (id: string, lat: number, status = "OPERATIONAL") => ({
      id,
      displayName: { text: `Quán ${id}` },
      formattedAddress: "Địa chỉ quán",
      googleMapsUri: "https://maps.google.com/?cid=1",
      location: { latitude: lat, longitude: 106.6 },
      businessStatus: status,
    });
    return Promise.resolve(Response.json({
      places: [
        place("near", 10.701),
        place("near", 10.701),
        place("second", 10.702),
        place("far", 11),
        place("closed", 10.7, "CLOSED_PERMANENTLY"),
      ],
    }));
  }) as typeof fetch;
}

Deno.test("Google discovery restricts radius, removes duplicate/outside/closed places", async () => {
  const result = await findFoodPlaces("test-secret", undefined, googleMock());
  assertEquals(result.restaurants.map((r) => r.id), ["near", "second"]);
  assert(result.restaurants[0].distanceMeters > 100);
  assert(result.restaurants[1].distanceMeters < 230);
});

Deno.test("ambiguous geocoding stops before querying restaurants; errors never expose API key", async () => {
  const ambiguous = (() =>
    Promise.resolve(
      Response.json({ status: "OK", results: [{ partial_match: true }] }),
    )) as typeof fetch;
  await assertRejects(
    () => findFoodPlaces("test-secret", undefined, ambiguous),
    Error,
    "Chưa xác định chính xác",
  );
  const broken = (() =>
    Promise.reject(
      new Error("https://example.com?key=test-secret"),
    )) as typeof fetch;
  const error = await assertRejects(() =>
    findFoodPlaces("test-secret", undefined, broken)
  );
  assert(!String(error).includes("test-secret"));
});

Deno.test("configuration validates time, weekdays and option limits", () => {
  for (
    const patch of [{ time: "24:00" }, { weekdays: [] }, { weekdays: [7] }, {
      optionCount: 1,
    }, { title: " " }]
  ) {
    assertThrows(() => foodPollConfig({ ...defaultFoodPoll(), ...patch }));
  }
  assertEquals(
    foodPollConfig({ ...defaultFoodPoll(), weekdays: [1, 1, 3] }).weekdays,
    [1, 3],
  );
});

Deno.test("Geocoding permission failures explain API setup without exposing provider errors", async () => {
  const denied = (() =>
    Promise.resolve(
      Response.json({
        status: "REQUEST_DENIED",
        error_message: "test-secret",
      }),
    )) as typeof fetch;
  const error = await assertRejects(
    () => findFoodPlaces("test-secret", undefined, denied),
    Error,
    "quyền Geocoding API",
  );
  assert(!String(error).includes("test-secret"));
});

Deno.test("group flag persists privately per account; previews do not save or send", async () => {
  const directory = await Deno.makeTempDir();
  let account = "u-first";
  const service = createFoodPollService({
    directory,
    account: () => account,
    isGroup: (mid) => mid === "c-group",
    request: googleMock(),
  });
  try {
    const config = { ...defaultFoodPoll(), flagged: true };
    const saved = await service.save("c-group", config, "test-secret");
    assertEquals(saved.config.flagged, true);
    assert(!JSON.stringify(saved).includes("test-secret"));
    const stat = await Deno.stat(`${directory}/food-polls-u-first.json`);
    if (Deno.build.os !== "windows") assertEquals(stat.mode! & 0o777, 0o600);
    await service.save("c-group", config);
    const preview = await service.preview("c-group", config);
    assertEquals(preview.options.length, 2);
    assert(preview.text.includes("Google Maps"));
    account = "u-second";
    assertEquals((await service.read("c-group")).config.flagged, false);
    assertEquals((await service.read("c-group")).hasApiKey, false);
    await assertRejects(
      () => service.read("u-friend"),
      Error,
      "Chỉ cấu hình cho nhóm",
    );
    await assertRejects(
      () => service.read("c-other"),
      Error,
      "Chỉ cấu hình cho nhóm",
    );
  } finally {
    await Deno.remove(directory, { recursive: true });
  }
});
