import * as grpc from '@grpc/grpc-js';
import * as fs from 'node:fs';

// Generated from the protos by `deploy/proto-gen/scripts/gen-ts.sh`.
// Once the SDK package is published, replace with:
//   import { Mode1ServiceClient, ClassifyRequest } from '@efe-project/efe-client';
import {
  Mode1ServiceClient,
  ClassifyRequest,
} from '../src/generated/efe/mode1/v1/service';

const endpoint = process.env.EFE_ENDPOINT ?? 'localhost:50051';
const token    = process.env.EFE_JWT ?? '';
const caPath   = process.env.EFE_TLS_CA ?? '';
const useTls   = process.env.EFE_TLS === '1' || process.env.EFE_TLS === 'true';

function buildCreds(): grpc.ChannelCredentials {
  if (!useTls) return grpc.credentials.createInsecure();
  const ca = fs.readFileSync(caPath);
  const tls = grpc.credentials.createSsl(ca);
  if (!token) return tls;
  const callCreds = grpc.credentials.createFromMetadataGenerator((_, cb) => {
    const md = new grpc.Metadata();
    md.set('authorization', `Bearer ${token}`);
    md.set('x-request-id', `example-${Date.now()}`);
    cb(null, md);
  });
  return grpc.credentials.combineChannelCredentials(tls, callCreds);
}

const client = new Mode1ServiceClient(endpoint, buildCreds());

const req: ClassifyRequest = {
  text: process.env.EFE_TEXT ?? 'My credit card was charged without consent',
  catalog: process.env.EFE_CATALOG ?? 'support_triage',
  tenantId: process.env.EFE_TENANT ?? 'example-tenant',
  requestId: `ts-${Date.now()}`,
  maxRounds: 0,
  budgetWallClockMs: 0,
  returnSubScores: false,
  metadata: {},
};

const deadline = new Date(Date.now() + 10_000);
client.classify(req, { deadline }, (err, resp) => {
  if (err) {
    console.error(`classify failed: code=${err.code} msg=${err.details}`);
    process.exit(1);
  }
  console.log(JSON.stringify({
    label:      resp!.label,
    confidence: resp!.confidence,
    zone:       resp!.zone,
    rounds:     resp!.rounds,
    reject:     resp!.reject,
    served_by:  resp!.servedByPod,
  }, null, 2));
});
