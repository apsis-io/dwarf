// Types for the `webcrypto` polyfill (`--polyfill webcrypto`) - `crypto.subtle`
// backed by @noble/hashes, @noble/curves, @noble/ciphers (see NOTICES).
//
// This is a deliberate SUBSET of the real Web Crypto API, not full spec
// parity:
//   - digest: SHA-1, SHA-256, SHA-384, SHA-512
//   - HMAC: sign/verify/generateKey/importKey/exportKey (raw, jwk)
//   - ECDSA (P-256 with SHA-256, P-384 with SHA-384 only - fixed pairing,
//     matching @noble/curves): generateKey/importKey/exportKey/sign/verify
//     (raw, jwk)
//   - ECDH (P-256, P-384): generateKey/importKey/exportKey/deriveBits/
//     deriveKey (raw, jwk)
//   - HKDF: importKey (raw only, per spec)/deriveBits/deriveKey
//   - AES-GCM: generateKey/importKey/exportKey/encrypt/decrypt (raw, jwk)
// NOT implemented: RSA (no pure-JS RSA in @noble), AES-CBC/CTR/CCM, PBKDF2,
// "spki"/"pkcs8" DER import/export (only "raw" and "jwk"), Ed25519/X25519.
//
// `crypto.getRandomValues` is NOT part of this polyfill - it's always
// generated separately (wired to `wasi:random/random`, see the always-on
// globals in `builtins.d.ts`/the CLI cheat sheet), independent of
// `--polyfill webcrypto`.

type WebCryptoHashName = "SHA-1" | "SHA-256" | "SHA-384" | "SHA-512";
type WebCryptoNamedCurve = "P-256" | "P-384";
type WebCryptoKeyUsage = "sign" | "verify" | "encrypt" | "decrypt" | "deriveBits" | "deriveKey";

interface HmacKeyAlgorithm {
  name: "HMAC";
  hash: WebCryptoHashName | { name: WebCryptoHashName };
  length?: number;
}

interface AesKeyAlgorithm {
  name: "AES-GCM";
  length?: 128 | 192 | 256;
}

interface EcKeyAlgorithm {
  name: "ECDSA" | "ECDH";
  namedCurve: WebCryptoNamedCurve;
}

type WebCryptoAlgorithm = HmacKeyAlgorithm | AesKeyAlgorithm | EcKeyAlgorithm | { name: "HKDF" };

declare class CryptoKey {
  readonly type: "secret" | "private" | "public";
  readonly extractable: boolean;
  readonly algorithm: Readonly<Record<string, unknown>>;
  readonly usages: ReadonlyArray<WebCryptoKeyUsage>;
}

interface CryptoKeyPair {
  privateKey: CryptoKey;
  publicKey: CryptoKey;
}

interface AesGcmParams {
  name: "AES-GCM";
  iv: BufferSource;
  additionalData?: BufferSource;
}

interface EcdsaParams {
  name: "ECDSA";
  hash: WebCryptoHashName | { name: WebCryptoHashName };
}

interface EcdhKeyDeriveParams {
  name: "ECDH";
  public: CryptoKey;
}

interface HkdfParams {
  name: "HKDF";
  hash: WebCryptoHashName | { name: WebCryptoHashName };
  salt: BufferSource;
  info: BufferSource;
}

type JsonWebKey = Record<string, unknown>;

declare const subtle: {
  digest(algorithm: WebCryptoHashName | { name: WebCryptoHashName }, data: BufferSource): Promise<ArrayBuffer>;

  generateKey(
    algorithm: HmacKeyAlgorithm | AesKeyAlgorithm,
    extractable: boolean,
    keyUsages: WebCryptoKeyUsage[],
  ): Promise<CryptoKey>;
  generateKey(algorithm: EcKeyAlgorithm, extractable: boolean, keyUsages: WebCryptoKeyUsage[]): Promise<CryptoKeyPair>;

  importKey(
    format: "raw",
    keyData: BufferSource,
    algorithm: WebCryptoAlgorithm,
    extractable: boolean,
    keyUsages: WebCryptoKeyUsage[],
  ): Promise<CryptoKey>;
  importKey(
    format: "jwk",
    keyData: JsonWebKey,
    algorithm: WebCryptoAlgorithm,
    extractable: boolean,
    keyUsages: WebCryptoKeyUsage[],
  ): Promise<CryptoKey>;

  exportKey(format: "raw", key: CryptoKey): Promise<ArrayBuffer>;
  exportKey(format: "jwk", key: CryptoKey): Promise<JsonWebKey>;

  sign(algorithm: "HMAC" | EcdsaParams, key: CryptoKey, data: BufferSource): Promise<ArrayBuffer>;
  verify(
    algorithm: "HMAC" | EcdsaParams,
    key: CryptoKey,
    signature: BufferSource,
    data: BufferSource,
  ): Promise<boolean>;

  encrypt(algorithm: AesGcmParams, key: CryptoKey, data: BufferSource): Promise<ArrayBuffer>;
  decrypt(algorithm: AesGcmParams, key: CryptoKey, data: BufferSource): Promise<ArrayBuffer>;

  deriveBits(algorithm: EcdhKeyDeriveParams | HkdfParams, baseKey: CryptoKey, length: number): Promise<ArrayBuffer>;
  deriveKey(
    algorithm: EcdhKeyDeriveParams | HkdfParams,
    baseKey: CryptoKey,
    derivedKeyAlgorithm: HmacKeyAlgorithm | AesKeyAlgorithm,
    extractable: boolean,
    keyUsages: WebCryptoKeyUsage[],
  ): Promise<CryptoKey>;
};
