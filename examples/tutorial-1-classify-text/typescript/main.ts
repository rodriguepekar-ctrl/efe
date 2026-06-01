// Tutorial 1 — Classify a single text via the TypeScript SDK.
//
// Sprint Φ-C-SDK Completion (D32) — artefact #25.
//
// Run :
//   EFE_HOST=efe.example.com:443 npx tsx main.ts

// Once the TS SDK is published, this becomes :
// import { EfeClient } from "@efe-ai/sdk";

async function main(): Promise<void> {
  const host = process.env.EFE_HOST ?? "efe.example.com:443";

  // Post-Φ-C-SDK D31 :
  //
  //   const client = new EfeClient({ host });
  //   try {
  //     const verdict = await client.classify({
  //       text: "I want to dispute the charge on my account",
  //       catalogId: "router-v1",
  //     });
  //     console.log(`label       : ${verdict.label}`);
  //     console.log(`confidence  : ${verdict.confidence.toFixed(4)}`);
  //     console.log(`rounds      : ${verdict.roundsExecuted}`);
  //     console.log(`swarm_id    : ${verdict.swarmId}`);
  //     console.log(`catalog_ver : ${verdict.catalogVersion}`);
  //   } finally {
  //     await client.close();
  //   }

  console.log(`tutorial-1 (TS) connecting to ${host}`);
  console.log(
    "Once Φ-C-SDK D31 wires the TS SDK @grpc/grpc-js stubs, this tutorial",
  );
  console.log(
    "will perform an end-to-end classify call. For now it documents the",
  );
  console.log("idiomatic Promise-based API shape per docs/sdk-auth-conventions.md.");
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
