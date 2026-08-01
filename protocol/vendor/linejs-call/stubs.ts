/** Local stubs so we can vendor PlanetTransport without the full linejs tree. */
export type Client = unknown;
export type DeviceDetails = Record<string, unknown>;

export class TypedEventEmitter<T extends Record<string, unknown>> {
  on<K extends keyof T>(_event: K, _listener: (payload: T[K]) => void): this {
    return this;
  }
  off<K extends keyof T>(_event: K, _listener: (payload: T[K]) => void): this {
    return this;
  }
  emit<K extends keyof T>(_event: K, _payload: T[K]): boolean {
    return false;
  }
}
