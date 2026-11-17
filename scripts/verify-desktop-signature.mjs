import { createHash, createPublicKey, verify } from "node:crypto";

function base64(value) {
  const text = value.trim();
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(text) || text.length % 4 !== 0) throw new Error("Invalid signature base64");
  const bytes = Buffer.from(text, "base64");
  if (bytes.toString("base64") !== text) throw new Error("Noncanonical signature base64");
  return bytes;
}
// Tauri wraps the four-line Minisign envelope in base64. This follows the
// installed minisign-verify verifier: ED = BLAKE2b-512 prehash, Ed = legacy raw
// Ed25519; the second signature authenticates signature bytes + trusted comment.
export function verifyDesktopSignature(bytes, signature, publicKey) {
  const keyLines = base64(publicKey).toString("utf8").trimEnd().split(/\r?\n/);
  const lines = base64(signature).toString("utf8").trimEnd().split(/\r?\n/);
  if (keyLines.length !== 2 || lines.length !== 4 || !lines[2].startsWith("trusted comment: "))
    throw new Error("Invalid Minisign envelope");
  const key = base64(keyLines[1]),
    signed = base64(lines[1]),
    global = base64(lines[3]);
  if (key.length !== 42 || signed.length !== 74 || global.length !== 64) throw new Error("Invalid Minisign length");
  if (!["Ed", "ED"].includes(key.subarray(0, 2).toString())) throw new Error("Unsupported public key algorithm");
  if (!key.subarray(2, 10).equals(signed.subarray(2, 10))) throw new Error("Unexpected signing key");
  const algorithm = signed.subarray(0, 2).toString();
  if (!["Ed", "ED"].includes(algorithm)) throw new Error("Unsupported signature algorithm");
  const message = algorithm === "ED" ? createHash("blake2b512").update(bytes).digest() : bytes;
  const ed25519 = createPublicKey({
    key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), key.subarray(10)]),
    format: "der",
    type: "spki",
  });
  const signatureBytes = signed.subarray(10);
  if (
    !verify(null, message, ed25519, signatureBytes) ||
    !verify(null, Buffer.concat([signatureBytes, Buffer.from(lines[2].slice(17))]), ed25519, global)
  )
    throw new Error("Invalid updater signature");
}
