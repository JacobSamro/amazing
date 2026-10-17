// WritableStream, its writer and controller.

const closeSentinel = { sentinel: 'close' };

function createWritableStreamState() {
  return {
    self: undefined, state: 'writable', storedError: undefined, writer: undefined, controller: undefined,
    inFlightWriteRequest: undefined, closeRequest: undefined, inFlightCloseRequest: undefined,
    pendingAbortRequest: undefined, writeRequests: [], backpressure: false,
  };
}
function isWritableStreamLocked(stream) {
  return stream.writer !== undefined;
}
function createWritableStream(startAlgorithm, writeAlgorithm, closeAlgorithm, abortAlgorithm, highWaterMark, sizeAlgorithm) {
  if (highWaterMark === undefined) highWaterMark = 1;
  if (sizeAlgorithm === undefined) sizeAlgorithm = () => 1;
  const stream = createWritableStreamState();
  stream.self = ObjectCreate(WritableStream.prototype);
  setSlot(wsBrand, stream.self, stream);
  const controller = newWritableController();
  setUpWritableStreamDefaultController(stream, controller, startAlgorithm, writeAlgorithm, closeAlgorithm, abortAlgorithm, highWaterMark, sizeAlgorithm);
  return stream;
}
function acquireWritableStreamDefaultWriter(stream) {
  const writer = newWriterState(ObjectCreate(WritableStreamDefaultWriter.prototype));
  setUpWritableStreamDefaultWriter(writer, stream);
  return writer;
}
function newWriterState(self) {
  const writer = { self, stream: undefined, closed: undefined, ready: undefined };
  setSlot(writerBrand, self, writer);
  return writer;
}
function writableStreamAbort(stream, reason) {
  if (stream.state === 'closed' || stream.state === 'errored') return resolvedPromise(undefined);
  abortControllerAbortFn(stream.controller, reason);
  const state = stream.state;
  if (state === 'closed' || state === 'errored') return resolvedPromise(undefined);
  if (stream.pendingAbortRequest !== undefined) return stream.pendingAbortRequest.promise.promise;
  let wasAlreadyErroring = false;
  if (state === 'erroring') {
    wasAlreadyErroring = true;
    reason = undefined;
  }
  const promise = newDeferred();
  stream.pendingAbortRequest = { promise, reason, wasAlreadyErroring };
  if (!wasAlreadyErroring) writableStreamStartErroring(stream, reason);
  return promise.promise;
}
function abortControllerAbortFn(controller, reason) {
  ReflectApply(abortControllerAbort, controller.abortController, [reason]);
}
function writableStreamClose(stream) {
  const state = stream.state;
  if (state === 'closed' || state === 'errored') {
    return rejectedPromise(new TypeErrorCtor('The stream is not in a state that permits close'));
  }
  const promise = newDeferred();
  stream.closeRequest = promise;
  const writer = stream.writer;
  if (writer !== undefined && stream.backpressure && state === 'writable') deferredResolve(writer.ready, undefined);
  writableStreamDefaultControllerClose(stream.controller);
  return promise.promise;
}
function writableStreamAddWriteRequest(stream) {
  const promise = newDeferred();
  ReflectApply(ArrayPush, stream.writeRequests, [promise]);
  return promise.promise;
}
function writableStreamCloseQueuedOrInFlight(stream) {
  return stream.closeRequest !== undefined || stream.inFlightCloseRequest !== undefined;
}
function writableStreamDealWithRejection(stream, error) {
  if (stream.state === 'writable') {
    writableStreamStartErroring(stream, error);
    return;
  }
  writableStreamFinishErroring(stream);
}
function writableStreamFinishErroring(stream) {
  stream.state = 'errored';
  writableControllerErrorSteps(stream.controller);
  const storedError = stream.storedError;
  const requests = stream.writeRequests;
  stream.writeRequests = [];
  for (let i = 0; i < requests.length; ++i) deferredReject(requests[i], storedError);
  if (stream.pendingAbortRequest === undefined) {
    writableStreamRejectCloseAndClosedPromiseIfNeeded(stream);
    return;
  }
  const abortRequest = stream.pendingAbortRequest;
  stream.pendingAbortRequest = undefined;
  if (abortRequest.wasAlreadyErroring) {
    deferredReject(abortRequest.promise, storedError);
    writableStreamRejectCloseAndClosedPromiseIfNeeded(stream);
    return;
  }
  const promise = writableControllerAbortSteps(stream.controller, abortRequest.reason);
  uponPromise(promise, () => {
    deferredResolve(abortRequest.promise, undefined);
    writableStreamRejectCloseAndClosedPromiseIfNeeded(stream);
  }, (reason) => {
    deferredReject(abortRequest.promise, reason);
    writableStreamRejectCloseAndClosedPromiseIfNeeded(stream);
  });
}
function writableStreamFinishInFlightClose(stream) {
  deferredResolve(stream.inFlightCloseRequest, undefined);
  stream.inFlightCloseRequest = undefined;
  if (stream.state === 'erroring') {
    stream.storedError = undefined;
    if (stream.pendingAbortRequest !== undefined) {
      deferredResolve(stream.pendingAbortRequest.promise, undefined);
      stream.pendingAbortRequest = undefined;
    }
  }
  stream.state = 'closed';
  const writer = stream.writer;
  if (writer !== undefined) deferredResolve(writer.closed, undefined);
}
function writableStreamFinishInFlightCloseWithError(stream, error) {
  deferredReject(stream.inFlightCloseRequest, error);
  stream.inFlightCloseRequest = undefined;
  if (stream.pendingAbortRequest !== undefined) {
    deferredReject(stream.pendingAbortRequest.promise, error);
    stream.pendingAbortRequest = undefined;
  }
  writableStreamDealWithRejection(stream, error);
}
function writableStreamFinishInFlightWrite(stream) {
  deferredResolve(stream.inFlightWriteRequest, undefined);
  stream.inFlightWriteRequest = undefined;
}
function writableStreamFinishInFlightWriteWithError(stream, error) {
  deferredReject(stream.inFlightWriteRequest, error);
  stream.inFlightWriteRequest = undefined;
  writableStreamDealWithRejection(stream, error);
}
function writableStreamHasOperationMarkedInFlight(stream) {
  return stream.inFlightWriteRequest !== undefined || stream.inFlightCloseRequest !== undefined;
}
function writableStreamMarkCloseRequestInFlight(stream) {
  stream.inFlightCloseRequest = stream.closeRequest;
  stream.closeRequest = undefined;
}
function writableStreamMarkFirstWriteRequestInFlight(stream) {
  stream.inFlightWriteRequest = ReflectApply(ArrayShift, stream.writeRequests, []);
}
function writableStreamRejectCloseAndClosedPromiseIfNeeded(stream) {
  if (stream.closeRequest !== undefined) {
    deferredReject(stream.closeRequest, stream.storedError);
    stream.closeRequest = undefined;
  }
  const writer = stream.writer;
  if (writer !== undefined) {
    deferredReject(writer.closed, stream.storedError);
    markHandled(writer.closed.promise);
  }
}
function writableStreamStartErroring(stream, reason) {
  const controller = stream.controller;
  stream.state = 'erroring';
  stream.storedError = reason;
  const writer = stream.writer;
  if (writer !== undefined) writableStreamDefaultWriterEnsureReadyPromiseRejected(writer, reason);
  if (!writableStreamHasOperationMarkedInFlight(stream) && controller.started) writableStreamFinishErroring(stream);
}
function writableStreamUpdateBackpressure(stream, backpressure) {
  const writer = stream.writer;
  if (writer !== undefined && backpressure !== stream.backpressure) {
    if (backpressure) writer.ready = newDeferred();
    else deferredResolve(writer.ready, undefined);
  }
  stream.backpressure = backpressure;
}

// ---- writer --------------------------------------------------------------------
function setUpWritableStreamDefaultWriter(writer, stream) {
  if (isWritableStreamLocked(stream)) throw new TypeErrorCtor('This stream has already been locked for exclusive writing by another writer');
  writer.stream = stream;
  stream.writer = writer;
  const state = stream.state;
  if (state === 'writable') {
    if (!writableStreamCloseQueuedOrInFlight(stream) && stream.backpressure) writer.ready = newDeferred();
    else writer.ready = resolvedDeferred(undefined);
    writer.closed = newDeferred();
  } else if (state === 'erroring') {
    writer.ready = rejectedDeferred(stream.storedError);
    writer.closed = newDeferred();
  } else if (state === 'closed') {
    writer.ready = resolvedDeferred(undefined);
    writer.closed = resolvedDeferred(undefined);
  } else {
    writer.ready = rejectedDeferred(stream.storedError);
    writer.closed = rejectedDeferred(stream.storedError);
  }
}
function writableStreamDefaultWriterAbort(writer, reason) {
  return writableStreamAbort(writer.stream, reason);
}
function writableStreamDefaultWriterClose(writer) {
  return writableStreamClose(writer.stream);
}
function writableStreamDefaultWriterCloseWithErrorPropagation(writer) {
  const stream = writer.stream;
  const state = stream.state;
  if (writableStreamCloseQueuedOrInFlight(stream) || state === 'closed') return resolvedPromise(undefined);
  if (state === 'errored') return rejectedPromise(stream.storedError);
  return writableStreamDefaultWriterClose(writer);
}
function writableStreamDefaultWriterEnsureClosedPromiseRejected(writer, error) {
  if (writer.closed.pending) deferredReject(writer.closed, error);
  else writer.closed = rejectedDeferred(error);
  markHandled(writer.closed.promise);
}
function writableStreamDefaultWriterEnsureReadyPromiseRejected(writer, error) {
  if (writer.ready.pending) deferredReject(writer.ready, error);
  else writer.ready = rejectedDeferred(error);
  markHandled(writer.ready.promise);
}
function writableStreamDefaultWriterGetDesiredSize(writer) {
  const stream = writer.stream;
  const state = stream.state;
  if (state === 'errored' || state === 'erroring') return null;
  if (state === 'closed') return 0;
  return writableStreamDefaultControllerGetDesiredSize(stream.controller);
}
function writableStreamDefaultWriterRelease(writer) {
  const stream = writer.stream;
  const releasedError = new TypeErrorCtor('Writer was released');
  writableStreamDefaultWriterEnsureReadyPromiseRejected(writer, releasedError);
  writableStreamDefaultWriterEnsureClosedPromiseRejected(writer, releasedError);
  stream.writer = undefined;
  writer.stream = undefined;
}
function writableStreamDefaultWriterWrite(writer, chunk) {
  const stream = writer.stream;
  const controller = stream.controller;
  const chunkSize = writableStreamDefaultControllerGetChunkSize(controller, chunk);
  if (stream !== writer.stream) return rejectedPromise(new TypeErrorCtor('Writer was released'));
  const state = stream.state;
  if (state === 'errored') return rejectedPromise(stream.storedError);
  if (writableStreamCloseQueuedOrInFlight(stream) || state === 'closed') {
    return rejectedPromise(new TypeErrorCtor('The stream is closing or closed and cannot be written to'));
  }
  if (state === 'erroring') return rejectedPromise(stream.storedError);
  const promise = writableStreamAddWriteRequest(stream);
  writableStreamDefaultControllerWrite(controller, chunk, chunkSize);
  return promise;
}

// ---- controller -------------------------------------------------------------------
function newWritableController() {
  const c = {
    self: ObjectCreate(WritableStreamDefaultController.prototype), stream: undefined, queue: [], queueTotalSize: 0,
    abortController: undefined, started: false, strategySize: undefined, strategyHWM: 0,
    writeAlgorithm: undefined, closeAlgorithm: undefined, abortAlgorithm: undefined,
  };
  setSlot(wdcBrand, c.self, c);
  return c;
}
function writableControllerAbortSteps(c, reason) {
  const result = c.abortAlgorithm(reason);
  writableStreamDefaultControllerClearAlgorithms(c);
  return result;
}
function writableControllerErrorSteps(c) {
  resetQueue(c);
}
function setUpWritableStreamDefaultController(stream, controller, startAlgorithm, writeAlgorithm, closeAlgorithm, abortAlgorithm, highWaterMark, sizeAlgorithm) {
  controller.stream = stream;
  stream.controller = controller;
  resetQueue(controller);
  controller.abortController = new AbortControllerCtor();
  controller.started = false;
  controller.strategySize = sizeAlgorithm;
  controller.strategyHWM = highWaterMark;
  controller.writeAlgorithm = writeAlgorithm;
  controller.closeAlgorithm = closeAlgorithm;
  controller.abortAlgorithm = abortAlgorithm;
  const backpressure = writableStreamDefaultControllerGetBackpressure(controller);
  writableStreamUpdateBackpressure(stream, backpressure);
  const startResult = startAlgorithm();
  uponPromise(resolvedPromise(startResult), () => {
    controller.started = true;
    writableStreamDefaultControllerAdvanceQueueIfNeeded(controller);
  }, (r) => {
    controller.started = true;
    writableStreamDealWithRejection(stream, r);
  });
}
function setUpWritableStreamDefaultControllerFromUnderlyingSink(stream, sink, dict, highWaterMark, sizeAlgorithm) {
  const controller = newWritableController();
  const startAlgorithm = dict.start === undefined ? () => undefined : () => ReflectApply(dict.start, sink, [controller.self]);
  const writeAlgorithm = dict.write === undefined ? () => resolvedPromise(undefined) : (chunk) => promiseCall(dict.write, sink, [chunk, controller.self]);
  const closeAlgorithm = dict.close === undefined ? () => resolvedPromise(undefined) : () => promiseCall(dict.close, sink, []);
  const abortAlgorithm = dict.abort === undefined ? () => resolvedPromise(undefined) : (reason) => promiseCall(dict.abort, sink, [reason]);
  setUpWritableStreamDefaultController(stream, controller, startAlgorithm, writeAlgorithm, closeAlgorithm, abortAlgorithm, highWaterMark, sizeAlgorithm);
}
function writableStreamDefaultControllerClearAlgorithms(c) {
  c.writeAlgorithm = undefined;
  c.closeAlgorithm = undefined;
  c.abortAlgorithm = undefined;
  c.strategySize = undefined;
}
function writableStreamDefaultControllerClose(c) {
  enqueueValueWithSize(c, closeSentinel, 0);
  writableStreamDefaultControllerAdvanceQueueIfNeeded(c);
}
function writableStreamDefaultControllerGetChunkSize(c, chunk) {
  if (c.strategySize === undefined) return 1;
  try {
    return c.strategySize(chunk);
  } catch (e) {
    writableStreamDefaultControllerErrorIfNeeded(c, e);
    return 1;
  }
}
function writableStreamDefaultControllerGetDesiredSize(c) {
  return c.strategyHWM - c.queueTotalSize;
}
function writableStreamDefaultControllerWrite(c, chunk, chunkSize) {
  try {
    enqueueValueWithSize(c, chunk, chunkSize);
  } catch (e) {
    writableStreamDefaultControllerErrorIfNeeded(c, e);
    return;
  }
  const stream = c.stream;
  if (!writableStreamCloseQueuedOrInFlight(stream) && stream.state === 'writable') {
    writableStreamUpdateBackpressure(stream, writableStreamDefaultControllerGetBackpressure(c));
  }
  writableStreamDefaultControllerAdvanceQueueIfNeeded(c);
}
function writableStreamDefaultControllerAdvanceQueueIfNeeded(c) {
  const stream = c.stream;
  if (!c.started) return;
  if (stream.inFlightWriteRequest !== undefined) return;
  const state = stream.state;
  if (state === 'erroring') {
    writableStreamFinishErroring(stream);
    return;
  }
  if (c.queue.length === 0) return;
  const value = peekQueueValue(c);
  if (value === closeSentinel) writableStreamDefaultControllerProcessClose(c);
  else writableStreamDefaultControllerProcessWrite(c, value);
}
function writableStreamDefaultControllerErrorIfNeeded(c, error) {
  if (c.stream.state === 'writable') writableStreamDefaultControllerError(c, error);
}
function writableStreamDefaultControllerProcessClose(c) {
  const stream = c.stream;
  writableStreamMarkCloseRequestInFlight(stream);
  dequeueValue(c);
  const sinkClosePromise = c.closeAlgorithm();
  writableStreamDefaultControllerClearAlgorithms(c);
  uponPromise(sinkClosePromise, () => {
    writableStreamFinishInFlightClose(stream);
  }, (reason) => {
    writableStreamFinishInFlightCloseWithError(stream, reason);
  });
}
function writableStreamDefaultControllerProcessWrite(c, chunk) {
  const stream = c.stream;
  writableStreamMarkFirstWriteRequestInFlight(stream);
  const sinkWritePromise = c.writeAlgorithm(chunk);
  uponPromise(sinkWritePromise, () => {
    writableStreamFinishInFlightWrite(stream);
    const state = stream.state;
    dequeueValue(c);
    if (!writableStreamCloseQueuedOrInFlight(stream) && state === 'writable') {
      writableStreamUpdateBackpressure(stream, writableStreamDefaultControllerGetBackpressure(c));
    }
    writableStreamDefaultControllerAdvanceQueueIfNeeded(c);
  }, (reason) => {
    if (stream.state === 'writable') writableStreamDefaultControllerClearAlgorithms(c);
    writableStreamFinishInFlightWriteWithError(stream, reason);
  });
}
function writableStreamDefaultControllerGetBackpressure(c) {
  return writableStreamDefaultControllerGetDesiredSize(c) <= 0;
}
function writableStreamDefaultControllerError(c, error) {
  const stream = c.stream;
  writableStreamDefaultControllerClearAlgorithms(c);
  writableStreamStartErroring(stream, error);
}

function convertUnderlyingSink(sink) {
  const obj = toDictionary(sink, 'underlyingSink');
  const dict = { abort: undefined, close: undefined, start: undefined, type: undefined, write: undefined };
  if (obj === undefined) return dict;
  let v = obj.abort;
  if (v !== undefined) dict.abort = toCallback(v, 'underlyingSink.abort');
  v = obj.close;
  if (v !== undefined) dict.close = toCallback(v, 'underlyingSink.close');
  v = obj.start;
  if (v !== undefined) dict.start = toCallback(v, 'underlyingSink.start');
  dict.type = obj.type;
  v = obj.write;
  if (v !== undefined) dict.write = toCallback(v, 'underlyingSink.write');
  return dict;
}

class WritableStream {
  constructor(underlyingSink = undefined, strategy = undefined) {
    const strategyDict = convertStrategy(strategy);
    const dict = convertUnderlyingSink(underlyingSink);
    const stream = createWritableStreamState();
    stream.self = this;
    setSlot(wsBrand, this, stream);
    if (dict.type !== undefined) throw new RangeErrorCtor('Invalid type is specified');
    const sizeAlgorithm = extractSizeAlgorithm(strategyDict);
    const highWaterMark = extractHighWaterMark(strategyDict, 1);
    setUpWritableStreamDefaultControllerFromUnderlyingSink(stream, underlyingSink === null ? undefined : underlyingSink, dict, highWaterMark, sizeAlgorithm);
  }
  get locked() {
    return isWritableStreamLocked(slotOf(wsBrand, this, 'WritableStream'));
  }
  abort(reason = undefined) {
    if (!hasSlot(wsBrand, this)) return rejectedPromise(new TypeErrorCtor('Illegal invocation: receiver is not a WritableStream'));
    const stream = slotOf(wsBrand, this);
    if (isWritableStreamLocked(stream)) return rejectedPromise(new TypeErrorCtor('Cannot abort a locked WritableStream'));
    return writableStreamAbort(stream, reason);
  }
  close() {
    if (!hasSlot(wsBrand, this)) return rejectedPromise(new TypeErrorCtor('Illegal invocation: receiver is not a WritableStream'));
    const stream = slotOf(wsBrand, this);
    if (isWritableStreamLocked(stream)) return rejectedPromise(new TypeErrorCtor('Cannot close a locked WritableStream'));
    if (writableStreamCloseQueuedOrInFlight(stream)) return rejectedPromise(new TypeErrorCtor('Cannot close an already-closing WritableStream'));
    return writableStreamClose(stream);
  }
  getWriter() {
    return acquireWritableStreamDefaultWriter(slotOf(wsBrand, this, 'WritableStream')).self;
  }
}

class WritableStreamDefaultWriter {
  constructor(stream) {
    if (arguments.length < 1) throw new TypeErrorCtor('WritableStreamDefaultWriter requires a stream');
    if (!hasSlot(wsBrand, stream)) throw new TypeErrorCtor('stream must be a WritableStream');
    setUpWritableStreamDefaultWriter(newWriterState(this), slotOf(wsBrand, stream));
  }
  get closed() {
    const writer = ReflectApply(WeakMapGet, writerBrand, [this]);
    if (writer === undefined) return rejectedPromise(new TypeErrorCtor('Illegal invocation: receiver is not a WritableStreamDefaultWriter'));
    return writer.closed.promise;
  }
  get desiredSize() {
    const writer = slotOf(writerBrand, this, 'WritableStreamDefaultWriter');
    if (writer.stream === undefined) throw new TypeErrorCtor('Writer has been released');
    return writableStreamDefaultWriterGetDesiredSize(writer);
  }
  get ready() {
    const writer = ReflectApply(WeakMapGet, writerBrand, [this]);
    if (writer === undefined) return rejectedPromise(new TypeErrorCtor('Illegal invocation: receiver is not a WritableStreamDefaultWriter'));
    return writer.ready.promise;
  }
  abort(reason = undefined) {
    const writer = ReflectApply(WeakMapGet, writerBrand, [this]);
    if (writer === undefined) return rejectedPromise(new TypeErrorCtor('Illegal invocation: receiver is not a WritableStreamDefaultWriter'));
    if (writer.stream === undefined) return rejectedPromise(new TypeErrorCtor('Writer has been released'));
    return writableStreamDefaultWriterAbort(writer, reason);
  }
  close() {
    const writer = ReflectApply(WeakMapGet, writerBrand, [this]);
    if (writer === undefined) return rejectedPromise(new TypeErrorCtor('Illegal invocation: receiver is not a WritableStreamDefaultWriter'));
    const stream = writer.stream;
    if (stream === undefined) return rejectedPromise(new TypeErrorCtor('Writer has been released'));
    if (writableStreamCloseQueuedOrInFlight(stream)) return rejectedPromise(new TypeErrorCtor('Cannot close an already-closing WritableStream'));
    return writableStreamDefaultWriterClose(writer);
  }
  releaseLock() {
    const writer = slotOf(writerBrand, this, 'WritableStreamDefaultWriter');
    if (writer.stream === undefined) return;
    writableStreamDefaultWriterRelease(writer);
  }
  write(chunk = undefined) {
    const writer = ReflectApply(WeakMapGet, writerBrand, [this]);
    if (writer === undefined) return rejectedPromise(new TypeErrorCtor('Illegal invocation: receiver is not a WritableStreamDefaultWriter'));
    if (writer.stream === undefined) return rejectedPromise(new TypeErrorCtor('Writer has been released'));
    return writableStreamDefaultWriterWrite(writer, chunk);
  }
}

class WritableStreamDefaultController {
  constructor() {
    throw new TypeErrorCtor('Illegal constructor');
  }
  get signal() {
    const c = slotOf(wdcBrand, this, 'WritableStreamDefaultController');
    return ReflectApply(abortControllerSignalGet, c.abortController, []);
  }
  error(e = undefined) {
    const c = slotOf(wdcBrand, this, 'WritableStreamDefaultController');
    if (c.stream.state !== 'writable') return;
    writableStreamDefaultControllerError(c, e);
  }
}
