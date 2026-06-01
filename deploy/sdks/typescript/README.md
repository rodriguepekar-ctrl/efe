# EFE Client — TypeScript / Node.js

Thin gRPC client for the EFE service.

## Install

```
npm install @efe-project/efe-client
```

## Quickstart

```ts
import * as grpc from '@grpc/grpc-js';
import * as fs from 'fs';
import { Mode1ServiceClient, ClassifyRequest } from '@efe-project/efe-client';

const endpoint = process.env.EFE_ENDPOINT ?? 'efe.platform.svc:50051';
const token    = process.env.EFE_JWT ?? '';
const caPath   = process.env.EFE_TLS_CA ?? '';

const creds = grpc.credentials.combineChannelCredentials(
  grpc.credentials.createSsl(fs.readFileSync(caPath)),
  grpc.credentials.createFromMetadataGenerator((_, cb) => {
    const md = new grpc.Metadata();
    md.set('authorization', `Bearer ${token}`);
    cb(null, md);
  }),
);

const client = new Mode1ServiceClient(endpoint, creds);

const req: ClassifyRequest = {
  text: 'My credit card was charged without consent',
  catalog: 'support_triage',
  tenantId: 'team-support',
  requestId: `example-${Date.now()}`,
};

client.classify(req, (err, resp) => {
  if (err) {
    console.error(`classify failed: code=${err.code} msg=${err.message}`);
    process.exit(1);
  }
  console.log(`label=${resp!.label} confidence=${resp!.confidence} zone=${resp!.zone}`);
});
```

## Regenerating stubs

```
cd deploy/sdks/typescript
make gen
```

## Browser / edge support

This package targets Node.js (`@grpc/grpc-js`). For browsers or edge
runtimes (Cloudflare Workers, Vercel Edge), use the HTTP/JSON gateway
via `@efe-project/efe-client-http` (generated from the same proto via
`protoc-gen-openapiv2`).
