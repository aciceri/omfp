/**
 * Runtime decoders mirroring how `serde` decodes omp-rpc's closed wire structs: required fields must be present and
 * of the right type, `Option` fields may be absent or `null` (both decode to "absent"), unknown keys are dropped.
 * A decoder throws {@link Invalid}; {@link decodeOrUndefined} turns that into `undefined`.
 */

import { isObject, type JsonObject, type JsonValue } from "./json.ts";

/** A value that does not decode; caught by the reducer, never escapes it. */
export class Invalid extends Error {}

export type Decoder<T> = (value: unknown) => T;

/** Throws {@link Invalid}: the value does not decode. */
export function fail(): never {
  throw new Invalid("invalid wire value");
}

/** Runs `decoder`; `undefined` when the value does not decode. */
export function decodeOrUndefined<T>(decoder: Decoder<T>, value: unknown): T | undefined {
  try {
    return decoder(value);
  } catch (error) {
    if (error instanceof Invalid) {
      return undefined;
    }
    throw error;
  }
}

export const str: Decoder<string> = (value) => (typeof value === "string" ? value : fail());

export const bool: Decoder<boolean> = (value) => (typeof value === "boolean" ? value : fail());

/** An integer (`i64`); JSON cannot tell `1.0` from `1`, so both decode. */
export const int: Decoder<number> = (value) =>
  Number.isSafeInteger(value) && typeof value === "number" ? value : fail();

/** An `i32`. */
export const i32: Decoder<number> = (value) => {
  const decoded = int(value);
  return decoded >= -0x8000_0000 && decoded <= 0x7fff_ffff ? decoded : fail();
};

/** A finite number (`f64`). */
export const num: Decoder<number> = (value) => (typeof value === "number" && Number.isFinite(value) ? value : fail());

/** Any JSON value (`serde_json::Value`); `undefined` means the field is missing. */
export const json: Decoder<JsonValue> = (value) => (value === undefined ? fail() : (value as JsonValue));

/** A JSON object (`serde_json::Map<String, Value>`). */
export const object: Decoder<JsonObject> = (value) => (isObject(value) ? (value as JsonObject) : fail());

/** One of the string `values` (a unit-variant enum). */
export function oneOf<const T extends string>(values: readonly T[]): Decoder<T> {
  return (value) => values.find((candidate) => candidate === value) ?? fail();
}

export function arrayOf<T>(item: Decoder<T>): Decoder<T[]> {
  return (value) => (Array.isArray(value) ? value.map((entry) => item(entry)) : fail());
}

/** An `Option` field: absent or `null` leaves the key out. */
export interface Optional<T> {
  readonly optional: Decoder<T>;
}

export function optional<T>(decoder: Decoder<T>): Optional<T> {
  return { optional: decoder };
}

/** A field with a `serde(default = …)`: absent takes `fallback`. */
export interface Defaulted<T> {
  readonly decoder: Decoder<T>;
  readonly fallback: T;
}

export function withDefault<T>(decoder: Decoder<T>, fallback: T): Defaulted<T> {
  return { decoder, fallback };
}

type OptionalKeys<T> = { [K in keyof T]-?: object extends Pick<T, K> ? K : never }[keyof T];

/** How to decode each field of `T`: optional properties take {@link optional}, required ones a decoder or default. */
export type Spec<T> = {
  [K in keyof T]-?: K extends OptionalKeys<T> ? Optional<Exclude<T[K], undefined>> : Decoder<T[K]> | Defaulted<T[K]>;
};

/** A closed struct: the declared fields of `spec`, decoded; every other key dropped. */
export function record<T>(spec: Spec<T>): Decoder<T> {
  const fields: [string, Decoder<unknown> | Optional<unknown> | Defaulted<unknown>][] = Object.entries(spec);
  return (value) => {
    if (!isObject(value)) {
      return fail();
    }
    const out: Record<string, unknown> = {};
    for (const [key, field] of fields) {
      const raw = Object.hasOwn(value, key) ? value[key] : undefined;
      if (typeof field === "function") {
        out[key] = field(raw);
      } else if ("optional" in field) {
        if (raw !== undefined && raw !== null) {
          out[key] = field.optional(raw);
        }
      } else {
        out[key] = raw === undefined ? field.fallback : field.decoder(raw);
      }
    }
    // The spec's type ties every key of `T` to its decoder, so `out` holds a `T`.
    return out as T;
  };
}
