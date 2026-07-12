// Types for the `unstorage` polyfill (`--polyfill unstorage`) - a universal
// key-value storage API from unjs/unstorage. Only the core plus its
// zero-config default (in-memory) driver are bundled here - every other
// driver (fs, redis, cloudflare-kv, etc., all Node/host-specific) is NOT
// available, even though `unstorage.builtinDrivers` still lists their
// module specifiers (a plain lookup table, not functional code - none of
// those modules are actually bundled, so referencing one would fail).
// `createStorage()` with no options already works out of the box.
//
// Storage is in-process memory only - not persisted across restarts, and
// (per dwarf's WASI 0.3 async task-cancellation semantics, same caveat as
// setTimeout) any state set is naturally scoped to the lifetime of however
// long the component instance itself stays alive.
//
// Simplified signatures (not unstorage's own generic `StorageDefinition`
// item-type mapping - a TS nicety not needed for this being genuinely
// useful at runtime).

type StorageValue = null | string | number | boolean | object;
type WatchEvent = "update" | "remove";
type WatchCallback = (event: WatchEvent, key: string) => unknown;
type Unwatch = () => void | Promise<void>;
type TransactionOptions = Record<string, unknown>;
type GetKeysOptions = TransactionOptions & { maxDepth?: number };

interface StorageMeta {
  atime?: Date;
  mtime?: Date;
  ttl?: number;
  [key: string]: StorageValue | Date | undefined;
}

interface Driver<OptionsT = unknown, InstanceT = unknown> {
  name?: string;
  options?: OptionsT;
  getInstance?: () => InstanceT;
  hasItem(key: string, opts?: TransactionOptions): boolean | Promise<boolean>;
  getItem(key: string, opts?: TransactionOptions): StorageValue | Promise<StorageValue>;
  setItem?(key: string, value: string, opts?: TransactionOptions): void | Promise<void>;
  removeItem?(key: string, opts?: TransactionOptions): void | Promise<void>;
  getKeys(base?: string, opts?: GetKeysOptions): string[] | Promise<string[]>;
  clear?(base?: string, opts?: TransactionOptions): void | Promise<void>;
  dispose?(): void | Promise<void>;
  watch?(callback: WatchCallback): Unwatch | Promise<Unwatch>;
}

interface Storage<T extends StorageValue = StorageValue> {
  hasItem(key: string, opts?: TransactionOptions): Promise<boolean>;
  getItem<R = T>(key: string, opts?: TransactionOptions): Promise<R | null>;
  getItems<U = T>(
    items: (string | { key: string; options?: TransactionOptions })[],
    commonOptions?: TransactionOptions,
  ): Promise<{ key: string; value: U }[]>;
  getItemRaw<R = unknown>(key: string, opts?: TransactionOptions): Promise<R | null>;
  setItem<U = T>(key: string, value: U, opts?: TransactionOptions): Promise<void>;
  setItems<U = T>(items: { key: string; value: U; options?: TransactionOptions }[], commonOptions?: TransactionOptions): Promise<void>;
  setItemRaw<T = unknown>(key: string, value: T, opts?: TransactionOptions): Promise<void>;
  removeItem(key: string, opts?: TransactionOptions | boolean): Promise<void>;
  getMeta(key: string, opts?: TransactionOptions | boolean): Promise<StorageMeta>;
  setMeta(key: string, value: StorageMeta, opts?: TransactionOptions): Promise<void>;
  removeMeta(key: string, opts?: TransactionOptions): Promise<void>;
  getKeys(base?: string, opts?: GetKeysOptions): Promise<string[]>;
  clear(base?: string, opts?: TransactionOptions): Promise<void>;
  dispose(): Promise<void>;
  watch(callback: WatchCallback): Promise<Unwatch>;
  unwatch(): Promise<void>;
  mount(base: string, driver: Driver): Storage<T>;
  unmount(base: string, dispose?: boolean): Promise<void>;
  getMount(key?: string): { base: string; driver: Driver };
  getMounts(base?: string, opts?: { parents?: boolean }): { base: string; driver: Driver }[];
  // Node.js Map-like alias methods
  keys: Storage<T>["getKeys"];
  get: Storage<T>["getItem"];
  set: Storage<T>["setItem"];
  has: Storage<T>["hasItem"];
  del: Storage<T>["removeItem"];
  remove: Storage<T>["removeItem"];
}

interface CreateStorageOptions {
  driver?: Driver;
}

declare const unstorage: {
  createStorage<T extends StorageValue = StorageValue>(options?: CreateStorageOptions): Storage<T>;
  prefixStorage<T extends StorageValue = StorageValue>(storage: Storage<T>, base: string): Storage<T>;
  snapshot(storage: Storage, base: string): Promise<Record<string, StorageValue>>;
  restoreSnapshot(storage: Storage, snapshot: Record<string, StorageValue>, base?: string): Promise<void>;
  defineDriver<OptionsT = unknown, InstanceT = unknown>(factory: (opts: OptionsT) => Driver<OptionsT, InstanceT>): (opts: OptionsT) => Driver<OptionsT, InstanceT>;
  normalizeKey(key?: string): string;
  joinKeys(...keys: string[]): string;
  normalizeBaseKey(base?: string): string;
  filterKeyByDepth(key: string, depth: number | undefined): boolean;
  filterKeyByBase(key: string, base: string | undefined): boolean;
  /** A lookup table of driver names to module specifiers - none of these
   * other than the zero-config default are actually bundled/available. */
  builtinDrivers: Record<string, string>;
};
