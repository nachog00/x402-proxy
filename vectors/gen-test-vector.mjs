// Generates a fixed EIP-3009 test vector with viem, for cross-validating
// the Rust signer. Throwaway key = 0x...01 (publicly known, never funded).
// Run (no native node): podman run --rm -v ./proxies/x402:/app:Z -w /app docker.io/library/node:24-slim \
//   sh -c "npm config set ignore-scripts true && npm install --omit=dev >/dev/null 2>&1 && node gen-test-vector.mjs"
import { privateKeyToAccount } from "viem/accounts";

const account = privateKeyToAccount(
  "0x0000000000000000000000000000000000000000000000000000000000000001"
);

const signature = await account.signTypedData({
  domain: {
    name: "USD Coin",
    version: "2",
    chainId: 8453,
    verifyingContract: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
  },
  types: {
    TransferWithAuthorization: [
      { name: "from", type: "address" },
      { name: "to", type: "address" },
      { name: "value", type: "uint256" },
      { name: "validAfter", type: "uint256" },
      { name: "validBefore", type: "uint256" },
      { name: "nonce", type: "bytes32" },
    ],
  },
  primaryType: "TransferWithAuthorization",
  message: {
    from: account.address,
    to: "0x4aAbE17C239eF71c3A26bA7C2b3e0AeBbfC1DF26",
    value: 1000000n,
    validAfter: 1700000000n,
    validBefore: 1700000060n,
    nonce: "0x0000000000000000000000000000000000000000000000000000000000000001",
  },
});

console.log("address:", account.address);
console.log("signature:", signature);
