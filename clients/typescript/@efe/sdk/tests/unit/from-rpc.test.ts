// Sprint D24 — unit tests : ConnectError → sentinel mapping.
import { Code, ConnectError } from '@connectrpc/connect';
import { describe, expect, it } from 'vitest';

import {
  ErrCatalogInvalid,
  ErrCatalogNotFound,
  ErrInternal,
  ErrInvalidArgument,
  ErrPermissionDenied,
  ErrRateLimited,
  ErrTimeout,
  ErrUnauthenticated,
  ErrUnavailable,
} from '../../src/index.js';
import { fromConnectError } from '../../src/core/from-rpc.js';

describe('fromConnectError', () => {
  it('maps Unauthenticated → ErrUnauthenticated', () => {
    const err = new ConnectError('bad token', Code.Unauthenticated);
    expect(fromConnectError(err)).toBeInstanceOf(ErrUnauthenticated);
  });

  it('maps PermissionDenied → ErrPermissionDenied', () => {
    expect(fromConnectError(new ConnectError('forbidden', Code.PermissionDenied)))
      .toBeInstanceOf(ErrPermissionDenied);
  });

  it('maps NotFound → ErrCatalogNotFound', () => {
    expect(fromConnectError(new ConnectError('no such', Code.NotFound)))
      .toBeInstanceOf(ErrCatalogNotFound);
  });

  it('maps FailedPrecondition → ErrCatalogInvalid', () => {
    expect(fromConnectError(new ConnectError('bad cat', Code.FailedPrecondition)))
      .toBeInstanceOf(ErrCatalogInvalid);
  });

  it('maps DeadlineExceeded + Canceled → ErrTimeout', () => {
    expect(fromConnectError(new ConnectError('to', Code.DeadlineExceeded)))
      .toBeInstanceOf(ErrTimeout);
    expect(fromConnectError(new ConnectError('cx', Code.Canceled)))
      .toBeInstanceOf(ErrTimeout);
  });

  it('maps Unavailable → ErrUnavailable', () => {
    expect(fromConnectError(new ConnectError('down', Code.Unavailable)))
      .toBeInstanceOf(ErrUnavailable);
  });

  it('maps ResourceExhausted → ErrRateLimited', () => {
    expect(fromConnectError(new ConnectError('quota', Code.ResourceExhausted)))
      .toBeInstanceOf(ErrRateLimited);
  });

  it('maps InvalidArgument + OutOfRange → ErrInvalidArgument', () => {
    expect(fromConnectError(new ConnectError('bad arg', Code.InvalidArgument)))
      .toBeInstanceOf(ErrInvalidArgument);
    expect(fromConnectError(new ConnectError('oob', Code.OutOfRange)))
      .toBeInstanceOf(ErrInvalidArgument);
  });

  it('maps Internal/Unknown/DataLoss → ErrInternal', () => {
    expect(fromConnectError(new ConnectError('crash', Code.Internal)))
      .toBeInstanceOf(ErrInternal);
    expect(fromConnectError(new ConnectError('?', Code.Unknown)))
      .toBeInstanceOf(ErrInternal);
    expect(fromConnectError(new ConnectError('lost', Code.DataLoss)))
      .toBeInstanceOf(ErrInternal);
  });

  it('passes non-ConnectError through unchanged', () => {
    const local = new Error('local');
    expect(fromConnectError(local)).toBe(local);
  });
});
