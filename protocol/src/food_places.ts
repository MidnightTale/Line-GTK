/** Google Maps restaurant discovery. No invented coordinates or sample restaurants. */
export const FOOD_ADDRESS =
  "33 Dương Quang Đông, Quận 8, Thành phố Hồ Chí Minh, Việt Nam";
export const FOOD_RADIUS = 5000;

export type Restaurant = {
  id: string;
  name: string;
  address: string;
  mapsUrl: string;
  distanceMeters: number;
};
type Point = { latitude: number; longitude: number };

export function distanceMeters(a: Point, b: Point): number {
  const radians = (n: number) => n * Math.PI / 180;
  const dLat = radians(b.latitude - a.latitude);
  const dLon = radians(b.longitude - a.longitude);
  const h = Math.sin(dLat / 2) ** 2 +
    Math.cos(radians(a.latitude)) * Math.cos(radians(b.latitude)) *
      Math.sin(dLon / 2) ** 2;
  return 6371000 * 2 * Math.asin(Math.sqrt(Math.min(1, h)));
}

export async function findFoodPlaces(
  apiKey: string,
  address = FOOD_ADDRESS,
  request: typeof fetch = fetch,
): Promise<
  { center: Point; resolvedAddress: string; restaurants: Restaurant[] }
> {
  if (!apiKey.trim()) throw new Error("Cần nhập Google Maps API key.");
  if (!address.trim()) throw new Error("Cần nhập địa chỉ tìm quán.");
  const geocode = new URL("https://maps.googleapis.com/maps/api/geocode/json");
  geocode.search = new URLSearchParams({
    address,
    key: apiKey,
    language: "vi",
    components: "country:VN",
  }).toString();
  // Never propagate fetch errors containing a geocoding URL with its API key.
  const json = async (url: string | URL, init?: RequestInit) => {
    let response: Response;
    try {
      response = await request(url, {
        ...init,
        signal: AbortSignal.timeout(10_000),
      });
    } catch {
      throw new Error("Không kết nối được Google Maps. Hãy thử lại.");
    }
    if (!response.ok) {
      throw new Error(
        `Google Maps trả lỗi HTTP ${response.status}. Kiểm tra API key và quota.`,
      );
    }
    try {
      return await response.json();
    } catch {
      throw new Error("Google Maps trả dữ liệu không hợp lệ.");
    }
  };
  const geo = await json(geocode);
  if (geo.status !== "OK" && geo.status !== "ZERO_RESULTS") {
    throw new Error(
      "Google Geocoding từ chối yêu cầu. Kiểm tra API key, quyền Geocoding API, billing và quota.",
    );
  }
  const matches = geo.results ?? [];
  if (geo.status !== "OK" || matches.length !== 1 || matches[0].partial_match) {
    throw new Error(
      "Chưa xác định chính xác địa chỉ. Hãy nhập địa chỉ đầy đủ và tìm lại.",
    );
  }
  const location = matches[0].geometry?.location;
  if (!Number.isFinite(location?.lat) || !Number.isFinite(location?.lng)) {
    throw new Error("Google Maps không trả tọa độ hợp lệ.");
  }
  const center = { latitude: location.lat, longitude: location.lng };
  const data = await json(
    "https://places.googleapis.com/v1/places:searchNearby",
    {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "X-Goog-Api-Key": apiKey,
        "X-Goog-FieldMask":
          "places.id,places.displayName,places.formattedAddress,places.googleMapsUri,places.location,places.businessStatus",
      },
      body: JSON.stringify({
        includedTypes: ["restaurant"],
        maxResultCount: 20,
        languageCode: "vi",
        rankPreference: "DISTANCE",
        locationRestriction: { circle: { center, radius: FOOD_RADIUS } },
      }),
    },
  );
  const restaurants: Restaurant[] = [];
  const seen = new Set<string>();
  for (const place of data.places ?? []) {
    if (!place.id || !place.displayName?.text || seen.has(place.id)) continue;
    if (place.businessStatus !== "OPERATIONAL") continue;
    const point = place.location;
    if (
      !Number.isFinite(point?.latitude) || !Number.isFinite(point?.longitude)
    ) continue;
    const distance = distanceMeters(center, point);
    if (distance > FOOD_RADIUS) continue;
    const url = String(place.googleMapsUri ?? "");
    if (
      !/^https:\/\/(www\.)?google\.com\/maps\b/.test(url) &&
      !/^https:\/\/maps\.google\.com\//.test(url)
    ) continue;
    seen.add(place.id);
    restaurants.push({
      id: place.id,
      name: place.displayName.text,
      address: String(place.formattedAddress ?? ""),
      mapsUrl: url,
      distanceMeters: Math.round(distance),
    });
  }
  restaurants.sort((a, b) => a.distanceMeters - b.distanceMeters);
  if (restaurants.length < 2) {
    throw new Error("Không tìm được ít nhất 2 quán trong bán kính 5 km.");
  }
  return { center, resolvedAddress: matches[0].formatted_address, restaurants };
}
