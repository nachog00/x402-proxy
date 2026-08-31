// Generates a fixed Permit2 `PermitWitnessTransferFrom` signature with viem,
// to cross-validate the Rust `upto` signer. Throwaway key 0x…01 (never funded).
// Container-run only (podman + node:24-slim, ignore-scripts). Inputs MUST match
// the Rust test in payment/upto.rs (signs_at fixed nonce/deadline/validAfter).
import { privateKeyToAccount } from "viem/accounts";

const account = privateKeyToAccount(
  "0x0000000000000000000000000000000000000000000000000000000000000001"
);

const signature = await account.signTypedData({
  // Permit2 domain — note: NO version field.
  domain: {
    name: "Permit2",
    chainId: 8453,
    verifyingContract: "0x000000000022D473030F116dDEE9F6B43aC78BA3",
  },
  types: {
    PermitWitnessTransferFrom: [
      { name: "permitted", type: "TokenPermissions" },
      { name: "spender", type: "address" },
      { name: "nonce", type: "uint256" },
      { name: "deadline", type: "uint256" },
      { name: "witness", type: "Witness" },
    ],
    TokenPermissions: [
      { name: "token", type: "address" },
      { name: "amount", type: "uint256" },
    ],
    Witness: [
      { name: "to", type: "address" },
      { name: "facilitator", type: "address" },
      { name: "validAfter", type: "uint256" },
    ],
  },
  primaryType: "PermitWitnessTransferFrom",
  message: {
    permitted: {
      token: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
      amount: 1000000n,
    },
    spender: "0x4020A4f3b7b90ccA423B9fabCc0CE57C6C240002",
    nonce: 1n,
    deadline: 1700000060n,
    witness: {
      to: "0x4aAbE17C239eF71c3A26bA7C2b3e0AeBbfC1DF26",
      facilitator: "0x14fDa13953Fc30428938E6BF950d036e77214e52",
      validAfter: 1700000000n,
    },
  },
});

console.log("address:", account.address);
console.log("signature:", signature);
