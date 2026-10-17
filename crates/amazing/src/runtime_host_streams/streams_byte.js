// ReadableByteStreamController, ReadableStreamBYOBRequest and byte-stream tee.

function newByteController() {
  const c = {
    kind: 'byte', self: ObjectCreate(ReadableByteStreamController.prototype), stream: undefined,
    queue: [], queueTotalSize: 0, started: false, closeRequested: false, pullAgain: false, pulling: false,
    strategyHWM: 0, pullAlgorithm: undefined, cancelAlgorithm: undefined, autoAllocateChunkSize: undefined,
    byobRequest: null, pendingPullIntos: [],
  };
  setSlot(rbcBrand, c.self, c);
  return c;
}
function byteControllerCancelSteps(c, reason) {
  readableByteStreamControllerClearPendingPullIntos(c);
  resetQueue(c);
  const result = c.cancelAlgorithm(reason);
  readableByteStreamControllerClearAlgorithms(c);
  return result;
}
function byteControllerPullSteps(c, readRequest) {
  const stream = c.stream;
  if (c.queueTotalSize > 0) {
    readableByteStreamControllerFillReadRequestFromQueue(c, readRequest);
    return;
  }
  const autoAllocateChunkSize = c.autoAllocateChunkSize;
  if (autoAllocateChunkSize !== undefined) {
    let buffer;
    try {
      buffer = new ArrayBufferCtor(autoAllocateChunkSize);
    } catch (bufferE) {
      readRequest.error(bufferE);
      return;
    }
    ReflectApply(ArrayPush, c.pendingPullIntos, [{
      buffer, bufferByteLength: autoAllocateChunkSize, byteOffset: 0, byteLength: autoAllocateChunkSize,
      bytesFilled: 0, minimumFill: 1, elementSize: 1, viewConstructor: Uint8ArrayCtor, readerType: 'default',
    }]);
  }
  readableStreamAddReadRequest(stream, readRequest);
  readableByteStreamControllerCallPullIfNeeded(c);
}
function byteControllerReleaseSteps(c) {
  if (c.pendingPullIntos.length > 0) {
    const first = c.pendingPullIntos[0];
    first.readerType = 'none';
    c.pendingPullIntos = [first];
  }
}
function readableByteStreamControllerCallPullIfNeeded(c) {
  if (!readableByteStreamControllerShouldCallPull(c)) return;
  if (c.pulling) {
    c.pullAgain = true;
    return;
  }
  c.pulling = true;
  const pullPromise = c.pullAlgorithm();
  uponPromise(pullPromise, () => {
    c.pulling = false;
    if (c.pullAgain) {
      c.pullAgain = false;
      readableByteStreamControllerCallPullIfNeeded(c);
    }
  }, (e) => {
    readableByteStreamControllerError(c, e);
  });
}
function readableByteStreamControllerClearAlgorithms(c) {
  c.pullAlgorithm = undefined;
  c.cancelAlgorithm = undefined;
}
function readableByteStreamControllerClearPendingPullIntos(c) {
  readableByteStreamControllerInvalidateBYOBRequest(c);
  c.pendingPullIntos = [];
}
function readableByteStreamControllerClose(c) {
  const stream = c.stream;
  if (c.closeRequested || stream.state !== 'readable') return;
  if (c.queueTotalSize > 0) {
    c.closeRequested = true;
    return;
  }
  if (c.pendingPullIntos.length > 0) {
    const first = c.pendingPullIntos[0];
    if (first.bytesFilled % first.elementSize !== 0) {
      const e = new TypeErrorCtor('Insufficient bytes to fill elements in the given buffer');
      readableByteStreamControllerError(c, e);
      throw e;
    }
  }
  readableByteStreamControllerClearAlgorithms(c);
  readableStreamClose(stream);
}
function readableByteStreamControllerCommitPullIntoDescriptor(stream, desc) {
  let done = false;
  if (stream.state === 'closed') done = true;
  const filledView = readableByteStreamControllerConvertPullIntoDescriptor(desc);
  if (desc.readerType === 'default') readableStreamFulfillReadRequest(stream, filledView, done);
  else readableStreamFulfillReadIntoRequest(stream, filledView, done);
}
function readableByteStreamControllerConvertPullIntoDescriptor(desc) {
  const buffer = transferArrayBuffer(desc.buffer);
  return new desc.viewConstructor(buffer, desc.byteOffset, desc.bytesFilled / desc.elementSize);
}
function readableByteStreamControllerEnqueueChunkToQueue(c, buffer, byteOffset, byteLength) {
  ReflectApply(ArrayPush, c.queue, [{ buffer, byteOffset, byteLength }]);
  c.queueTotalSize += byteLength;
}
function readableByteStreamControllerEnqueueClonedChunkToQueue(c, buffer, byteOffset, byteLength) {
  let clonedChunk;
  try {
    clonedChunk = cloneArrayBuffer(buffer, byteOffset, byteLength);
  } catch (cloneE) {
    readableByteStreamControllerError(c, cloneE);
    throw cloneE;
  }
  readableByteStreamControllerEnqueueChunkToQueue(c, clonedChunk, 0, byteLength);
}
function readableByteStreamControllerEnqueueDetachedPullIntoToQueue(c, desc) {
  if (desc.bytesFilled > 0) {
    readableByteStreamControllerEnqueueClonedChunkToQueue(c, desc.buffer, desc.byteOffset, desc.bytesFilled);
  }
  readableByteStreamControllerShiftPendingPullInto(c);
}
function readableByteStreamControllerError(c, e) {
  const stream = c.stream;
  if (stream.state !== 'readable') return;
  readableByteStreamControllerClearPendingPullIntos(c);
  resetQueue(c);
  readableByteStreamControllerClearAlgorithms(c);
  readableStreamError(stream, e);
}
function readableByteStreamControllerFillHeadPullIntoDescriptor(c, size, desc) {
  desc.bytesFilled += size;
}
function readableByteStreamControllerFillPullIntoDescriptorFromQueue(c, desc) {
  const maxBytesToCopy = MathMin(c.queueTotalSize, desc.byteLength - desc.bytesFilled);
  const maxBytesFilled = desc.bytesFilled + maxBytesToCopy;
  let totalBytesToCopyRemaining = maxBytesToCopy;
  let ready = false;
  const remainderBytes = maxBytesFilled % desc.elementSize;
  const maxAlignedBytes = maxBytesFilled - remainderBytes;
  if (maxAlignedBytes >= desc.minimumFill) {
    totalBytesToCopyRemaining = maxAlignedBytes - desc.bytesFilled;
    ready = true;
  }
  const queue = c.queue;
  while (totalBytesToCopyRemaining > 0) {
    const head = queue[0];
    const bytesToCopy = MathMin(totalBytesToCopyRemaining, head.byteLength);
    const destStart = desc.byteOffset + desc.bytesFilled;
    copyBytes(desc.buffer, destStart, head.buffer, head.byteOffset, bytesToCopy);
    if (head.byteLength === bytesToCopy) {
      ReflectApply(ArrayShift, queue, []);
    } else {
      head.byteOffset += bytesToCopy;
      head.byteLength -= bytesToCopy;
    }
    c.queueTotalSize -= bytesToCopy;
    readableByteStreamControllerFillHeadPullIntoDescriptor(c, bytesToCopy, desc);
    totalBytesToCopyRemaining -= bytesToCopy;
  }
  return ready;
}
function readableByteStreamControllerFillReadRequestFromQueue(c, readRequest) {
  const entry = ReflectApply(ArrayShift, c.queue, []);
  c.queueTotalSize -= entry.byteLength;
  readableByteStreamControllerHandleQueueDrain(c);
  const view = new Uint8ArrayCtor(entry.buffer, entry.byteOffset, entry.byteLength);
  readRequest.chunk(view);
}
function readableByteStreamControllerGetBYOBRequest(c) {
  if (c.byobRequest === null && c.pendingPullIntos.length > 0) {
    const first = c.pendingPullIntos[0];
    const view = new Uint8ArrayCtor(first.buffer, first.byteOffset + first.bytesFilled, first.byteLength - first.bytesFilled);
    const request = { self: ObjectCreate(ReadableStreamBYOBRequest.prototype), controller: c, view };
    setSlot(brqBrand, request.self, request);
    c.byobRequest = request;
  }
  return c.byobRequest;
}
function readableByteStreamControllerGetDesiredSize(c) {
  const state = c.stream.state;
  if (state === 'errored') return null;
  if (state === 'closed') return 0;
  return c.strategyHWM - c.queueTotalSize;
}
function readableByteStreamControllerHandleQueueDrain(c) {
  if (c.queueTotalSize === 0 && c.closeRequested) {
    readableByteStreamControllerClearAlgorithms(c);
    readableStreamClose(c.stream);
  } else {
    readableByteStreamControllerCallPullIfNeeded(c);
  }
}
function readableByteStreamControllerInvalidateBYOBRequest(c) {
  if (c.byobRequest === null) return;
  c.byobRequest.controller = undefined;
  c.byobRequest.view = null;
  c.byobRequest = null;
}
function readableByteStreamControllerProcessPullIntoDescriptorsUsingQueue(c) {
  while (c.pendingPullIntos.length > 0) {
    if (c.queueTotalSize === 0) return;
    const desc = c.pendingPullIntos[0];
    if (readableByteStreamControllerFillPullIntoDescriptorFromQueue(c, desc)) {
      readableByteStreamControllerShiftPendingPullInto(c);
      readableByteStreamControllerCommitPullIntoDescriptor(c.stream, desc);
    }
  }
}
function readableByteStreamControllerProcessReadRequestsUsingQueue(c) {
  const reader = c.stream.reader;
  while (reader.readRequests.length > 0) {
    if (c.queueTotalSize === 0) return;
    const readRequest = ReflectApply(ArrayShift, reader.readRequests, []);
    readableByteStreamControllerFillReadRequestFromQueue(c, readRequest);
  }
}
function readableByteStreamControllerPullInto(c, view, min, readIntoRequest) {
  const stream = c.stream;
  let elementSize = 1;
  let ctor = DataView;
  const tag = viewTag(view);
  if (tag !== undefined) {
    ctor = viewCtors[tag];
    elementSize = viewElementSizes[tag];
  }
  const minimumFill = min * elementSize;
  const byteOffset = viewByteOffset(view);
  const byteLength = viewByteLength(view);
  let buffer;
  try {
    buffer = transferArrayBuffer(viewBuffer(view));
  } catch (e) {
    readIntoRequest.error(e);
    return;
  }
  const desc = {
    buffer, bufferByteLength: bufferByteLength(buffer), byteOffset, byteLength, bytesFilled: 0,
    minimumFill, elementSize, viewConstructor: ctor, readerType: 'byob',
  };
  if (c.pendingPullIntos.length > 0) {
    ReflectApply(ArrayPush, c.pendingPullIntos, [desc]);
    readableStreamAddReadIntoRequest(stream, readIntoRequest);
    return;
  }
  if (stream.state === 'closed') {
    const emptyView = new ctor(desc.buffer, desc.byteOffset, 0);
    readIntoRequest.close(emptyView);
    return;
  }
  if (c.queueTotalSize > 0) {
    if (readableByteStreamControllerFillPullIntoDescriptorFromQueue(c, desc)) {
      const filledView = readableByteStreamControllerConvertPullIntoDescriptor(desc);
      readableByteStreamControllerHandleQueueDrain(c);
      readIntoRequest.chunk(filledView);
      return;
    }
    if (c.closeRequested) {
      const e = new TypeErrorCtor('Insufficient bytes to fill elements in the given buffer');
      readableByteStreamControllerError(c, e);
      readIntoRequest.error(e);
      return;
    }
  }
  ReflectApply(ArrayPush, c.pendingPullIntos, [desc]);
  readableStreamAddReadIntoRequest(stream, readIntoRequest);
  readableByteStreamControllerCallPullIfNeeded(c);
}
function readableByteStreamControllerRespondInClosedState(c, firstDescriptor) {
  if (firstDescriptor.readerType === 'none') readableByteStreamControllerShiftPendingPullInto(c);
  const stream = c.stream;
  if (readableStreamHasBYOBReader(stream)) {
    while (readableStreamGetNumReadIntoRequests(stream) > 0) {
      const desc = readableByteStreamControllerShiftPendingPullInto(c);
      readableByteStreamControllerCommitPullIntoDescriptor(stream, desc);
    }
  }
}
function readableByteStreamControllerRespondInReadableState(c, bytesWritten, desc) {
  readableByteStreamControllerFillHeadPullIntoDescriptor(c, bytesWritten, desc);
  if (desc.readerType === 'none') {
    readableByteStreamControllerEnqueueDetachedPullIntoToQueue(c, desc);
    readableByteStreamControllerProcessPullIntoDescriptorsUsingQueue(c);
    return;
  }
  if (desc.bytesFilled < desc.minimumFill) return;
  readableByteStreamControllerShiftPendingPullInto(c);
  const remainderSize = desc.bytesFilled % desc.elementSize;
  if (remainderSize > 0) {
    const end = desc.byteOffset + desc.bytesFilled;
    readableByteStreamControllerEnqueueClonedChunkToQueue(c, desc.buffer, end - remainderSize, remainderSize);
  }
  desc.bytesFilled -= remainderSize;
  readableByteStreamControllerCommitPullIntoDescriptor(c.stream, desc);
  readableByteStreamControllerProcessPullIntoDescriptorsUsingQueue(c);
}
function readableByteStreamControllerRespondInternal(c, bytesWritten) {
  const desc = c.pendingPullIntos[0];
  readableByteStreamControllerInvalidateBYOBRequest(c);
  const state = c.stream.state;
  if (state === 'closed') readableByteStreamControllerRespondInClosedState(c, desc);
  else readableByteStreamControllerRespondInReadableState(c, bytesWritten, desc);
  readableByteStreamControllerCallPullIfNeeded(c);
}
function readableByteStreamControllerRespond(c, bytesWritten) {
  const first = c.pendingPullIntos[0];
  const state = c.stream.state;
  if (state === 'closed') {
    if (bytesWritten !== 0) throw new TypeErrorCtor('bytesWritten must be 0 when the stream is closed');
  } else {
    if (bytesWritten === 0) throw new TypeErrorCtor('bytesWritten must be greater than 0 when the stream is readable');
    if (first.bytesFilled + bytesWritten > first.byteLength) throw new RangeErrorCtor('bytesWritten out of range');
  }
  first.buffer = transferArrayBuffer(first.buffer);
  readableByteStreamControllerRespondInternal(c, bytesWritten);
}
function readableByteStreamControllerRespondWithNewView(c, view) {
  const first = c.pendingPullIntos[0];
  const state = c.stream.state;
  const viewLength = viewByteLength(view);
  if (state === 'closed') {
    if (viewLength !== 0) throw new TypeErrorCtor("The view's length must be 0 when the stream is closed");
  } else if (viewLength === 0) {
    throw new TypeErrorCtor("The view's length must be greater than 0 when the stream is readable");
  }
  if (first.byteOffset + first.bytesFilled !== viewByteOffset(view)) throw new RangeErrorCtor('The region specified by view does not match byobRequest');
  if (first.bufferByteLength !== bufferByteLength(viewBuffer(view))) throw new RangeErrorCtor('The buffer of view has different capacity than byobRequest');
  if (first.bytesFilled + viewLength > first.byteLength) throw new RangeErrorCtor('The region specified by view is larger than byobRequest');
  first.buffer = transferArrayBuffer(viewBuffer(view));
  readableByteStreamControllerRespondInternal(c, viewLength);
}
function readableByteStreamControllerShiftPendingPullInto(c) {
  return ReflectApply(ArrayShift, c.pendingPullIntos, []);
}
function readableByteStreamControllerShouldCallPull(c) {
  const stream = c.stream;
  if (stream.state !== 'readable') return false;
  if (c.closeRequested) return false;
  if (!c.started) return false;
  if (readableStreamHasDefaultReader(stream) && readableStreamGetNumReadRequests(stream) > 0) return true;
  if (readableStreamHasBYOBReader(stream) && readableStreamGetNumReadIntoRequests(stream) > 0) return true;
  return readableByteStreamControllerGetDesiredSize(c) > 0;
}
function readableByteStreamControllerEnqueue(c, chunk) {
  const stream = c.stream;
  if (c.closeRequested || stream.state !== 'readable') return;
  const buffer = viewBuffer(chunk);
  const byteOffset = viewByteOffset(chunk);
  const byteLength = viewByteLength(chunk);
  if (isDetachedBuffer(buffer)) throw new TypeErrorCtor('chunk buffer is detached');
  const transferredBuffer = transferArrayBuffer(buffer);
  if (c.pendingPullIntos.length > 0) {
    const first = c.pendingPullIntos[0];
    if (isDetachedBuffer(first.buffer)) throw new TypeErrorCtor('The BYOB request buffer has been detached');
    readableByteStreamControllerInvalidateBYOBRequest(c);
    first.buffer = transferArrayBuffer(first.buffer);
    if (first.readerType === 'none') readableByteStreamControllerEnqueueDetachedPullIntoToQueue(c, first);
  }
  if (readableStreamHasDefaultReader(stream)) {
    readableByteStreamControllerProcessReadRequestsUsingQueue(c);
    if (readableStreamGetNumReadRequests(stream) === 0) {
      readableByteStreamControllerEnqueueChunkToQueue(c, transferredBuffer, byteOffset, byteLength);
    } else {
      if (c.pendingPullIntos.length > 0) readableByteStreamControllerShiftPendingPullInto(c);
      const transferredView = new Uint8ArrayCtor(transferredBuffer, byteOffset, byteLength);
      readableStreamFulfillReadRequest(stream, transferredView, false);
    }
  } else if (readableStreamHasBYOBReader(stream)) {
    readableByteStreamControllerEnqueueChunkToQueue(c, transferredBuffer, byteOffset, byteLength);
    readableByteStreamControllerProcessPullIntoDescriptorsUsingQueue(c);
  } else {
    readableByteStreamControllerEnqueueChunkToQueue(c, transferredBuffer, byteOffset, byteLength);
  }
  readableByteStreamControllerCallPullIfNeeded(c);
}
function setUpReadableByteStreamController(stream, controller, startAlgorithm, pullAlgorithm, cancelAlgorithm, highWaterMark, autoAllocateChunkSize) {
  controller.stream = stream;
  controller.pullAgain = false;
  controller.pulling = false;
  controller.byobRequest = null;
  resetQueue(controller);
  controller.closeRequested = false;
  controller.started = false;
  controller.strategyHWM = highWaterMark;
  controller.pullAlgorithm = pullAlgorithm;
  controller.cancelAlgorithm = cancelAlgorithm;
  controller.autoAllocateChunkSize = autoAllocateChunkSize;
  controller.pendingPullIntos = [];
  stream.controller = controller;
  const startResult = startAlgorithm();
  uponPromise(resolvedPromise(startResult), () => {
    controller.started = true;
    readableByteStreamControllerCallPullIfNeeded(controller);
  }, (r) => {
    readableByteStreamControllerError(controller, r);
  });
}
function setUpReadableByteStreamControllerFromUnderlyingSource(stream, source, dict, highWaterMark) {
  const controller = newByteController();
  const startAlgorithm = dict.start === undefined ? () => undefined : () => ReflectApply(dict.start, source, [controller.self]);
  const pullAlgorithm = dict.pull === undefined ? () => resolvedPromise(undefined) : () => promiseCall(dict.pull, source, [controller.self]);
  const cancelAlgorithm = dict.cancel === undefined ? () => resolvedPromise(undefined) : (reason) => promiseCall(dict.cancel, source, [reason]);
  const autoAllocateChunkSize = dict.autoAllocateChunkSize;
  if (autoAllocateChunkSize === 0) throw new TypeErrorCtor('autoAllocateChunkSize must be greater than 0');
  setUpReadableByteStreamController(stream, controller, startAlgorithm, pullAlgorithm, cancelAlgorithm, highWaterMark, autoAllocateChunkSize);
}

class ReadableStreamBYOBRequest {
  constructor() {
    throw new TypeErrorCtor('Illegal constructor');
  }
  get view() {
    return slotOf(brqBrand, this, 'ReadableStreamBYOBRequest').view;
  }
  respond(bytesWritten) {
    const request = slotOf(brqBrand, this, 'ReadableStreamBYOBRequest');
    if (arguments.length < 1) throw new TypeErrorCtor('respond requires a bytesWritten argument');
    const written = enforceRangeUnsigned(bytesWritten, 'bytesWritten');
    if (request.controller === undefined) throw new TypeErrorCtor('This BYOB request has been invalidated');
    if (isDetachedBuffer(ReflectApply(taBufferGet, request.view, []))) throw new TypeErrorCtor('The BYOB request buffer has been detached');
    readableByteStreamControllerRespond(request.controller, written);
  }
  respondWithNewView(view) {
    const request = slotOf(brqBrand, this, 'ReadableStreamBYOBRequest');
    if (arguments.length < 1) throw new TypeErrorCtor('respondWithNewView requires a view argument');
    if (!ReflectApply(ArrayBufferIsView, ArrayBufferCtor, [view])) throw new TypeErrorCtor('You can only respond with array buffer views');
    if (request.controller === undefined) throw new TypeErrorCtor('This BYOB request has been invalidated');
    if (isDetachedBuffer(viewBuffer(view))) throw new TypeErrorCtor('The given view has been detached');
    readableByteStreamControllerRespondWithNewView(request.controller, view);
  }
}

class ReadableByteStreamController {
  constructor() {
    throw new TypeErrorCtor('Illegal constructor');
  }
  get byobRequest() {
    const request = readableByteStreamControllerGetBYOBRequest(slotOf(rbcBrand, this, 'ReadableByteStreamController'));
    return request === null ? null : request.self;
  }
  get desiredSize() {
    return readableByteStreamControllerGetDesiredSize(slotOf(rbcBrand, this, 'ReadableByteStreamController'));
  }
  close() {
    const c = slotOf(rbcBrand, this, 'ReadableByteStreamController');
    if (c.closeRequested) throw new TypeErrorCtor('The stream has already been closed; do not close it again!');
    if (c.stream.state !== 'readable') throw new TypeErrorCtor('The stream is not in the readable state and cannot be closed');
    readableByteStreamControllerClose(c);
  }
  enqueue(chunk) {
    const c = slotOf(rbcBrand, this, 'ReadableByteStreamController');
    if (arguments.length < 1) throw new TypeErrorCtor('enqueue requires a chunk argument');
    if (!ReflectApply(ArrayBufferIsView, ArrayBufferCtor, [chunk])) throw new TypeErrorCtor('chunk must be an ArrayBufferView');
    if (viewByteLength(chunk) === 0) throw new TypeErrorCtor('chunk must have non-zero byteLength');
    if (bufferByteLength(viewBuffer(chunk)) === 0) throw new TypeErrorCtor("chunk's buffer must have non-zero byteLength");
    if (c.closeRequested) throw new TypeErrorCtor('stream is closed or draining');
    if (c.stream.state !== 'readable') throw new TypeErrorCtor('The stream is not in the readable state and cannot be enqueued to');
    readableByteStreamControllerEnqueue(c, chunk);
  }
  error(e = undefined) {
    readableByteStreamControllerError(slotOf(rbcBrand, this, 'ReadableByteStreamController'), e);
  }
}

// ---- byte stream tee -------------------------------------------------------------
function cloneAsUint8Array(view) {
  const buffer = cloneArrayBuffer(viewBuffer(view), viewByteOffset(view), viewByteLength(view));
  return new Uint8ArrayCtor(buffer);
}
function readableByteStreamTee(stream) {
  let reader = acquireReadableStreamDefaultReader(stream);
  let reading = false;
  let readAgainForBranch1 = false;
  let readAgainForBranch2 = false;
  let canceled1 = false;
  let canceled2 = false;
  let reason1;
  let reason2;
  let branch1;
  let branch2;
  const cancelPromise = newDeferred();
  function forwardReaderError(thisReader) {
    uponPromise(thisReader.closed.promise, undefined, (r) => {
      if (thisReader !== reader) return;
      readableByteStreamControllerError(branch1.controller, r);
      readableByteStreamControllerError(branch2.controller, r);
      if (!canceled1 || !canceled2) deferredResolve(cancelPromise, undefined);
    });
  }
  function pullWithDefaultReader() {
    if (reader.kind === 'byob') {
      readableStreamBYOBReaderRelease(reader);
      reader = acquireReadableStreamDefaultReader(stream);
      forwardReaderError(reader);
    }
    const readRequest = {
      chunk(chunk) {
        enqueueMicrotask(() => {
          readAgainForBranch1 = false;
          readAgainForBranch2 = false;
          const chunk1 = chunk;
          let chunk2 = chunk;
          if (!canceled1 && !canceled2) {
            try {
              chunk2 = cloneAsUint8Array(chunk);
            } catch (cloneE) {
              readableByteStreamControllerError(branch1.controller, cloneE);
              readableByteStreamControllerError(branch2.controller, cloneE);
              deferredResolve(cancelPromise, readableStreamCancel(stream, cloneE));
              return;
            }
          }
          if (!canceled1) readableByteStreamControllerEnqueue(branch1.controller, chunk1);
          if (!canceled2) readableByteStreamControllerEnqueue(branch2.controller, chunk2);
          reading = false;
          if (readAgainForBranch1) pull1Algorithm();
          else if (readAgainForBranch2) pull2Algorithm();
        });
      },
      close() {
        reading = false;
        if (!canceled1) readableByteStreamControllerClose(branch1.controller);
        if (!canceled2) readableByteStreamControllerClose(branch2.controller);
        if (branch1.controller.pendingPullIntos.length > 0) readableByteStreamControllerRespond(branch1.controller, 0);
        if (branch2.controller.pendingPullIntos.length > 0) readableByteStreamControllerRespond(branch2.controller, 0);
        if (!canceled1 || !canceled2) deferredResolve(cancelPromise, undefined);
      },
      error() {
        reading = false;
      },
    };
    readableStreamDefaultReaderRead(reader, readRequest);
  }
  function pullWithBYOBReader(view, forBranch2) {
    if (reader.kind === 'default') {
      readableStreamDefaultReaderRelease(reader);
      reader = acquireReadableStreamBYOBReader(stream);
      forwardReaderError(reader);
    }
    const byobBranch = forBranch2 ? branch2 : branch1;
    const otherBranch = forBranch2 ? branch1 : branch2;
    const readIntoRequest = {
      chunk(chunk) {
        enqueueMicrotask(() => {
          readAgainForBranch1 = false;
          readAgainForBranch2 = false;
          const byobCanceled = forBranch2 ? canceled2 : canceled1;
          const otherCanceled = forBranch2 ? canceled1 : canceled2;
          if (!otherCanceled) {
            let clonedChunk;
            try {
              clonedChunk = cloneAsUint8Array(chunk);
            } catch (cloneE) {
              readableByteStreamControllerError(byobBranch.controller, cloneE);
              readableByteStreamControllerError(otherBranch.controller, cloneE);
              deferredResolve(cancelPromise, readableStreamCancel(stream, cloneE));
              return;
            }
            if (!byobCanceled) readableByteStreamControllerRespondWithNewView(byobBranch.controller, chunk);
            readableByteStreamControllerEnqueue(otherBranch.controller, clonedChunk);
          } else if (!byobCanceled) {
            readableByteStreamControllerRespondWithNewView(byobBranch.controller, chunk);
          }
          reading = false;
          if (readAgainForBranch1) pull1Algorithm();
          else if (readAgainForBranch2) pull2Algorithm();
        });
      },
      close(chunk) {
        reading = false;
        const byobCanceled = forBranch2 ? canceled2 : canceled1;
        const otherCanceled = forBranch2 ? canceled1 : canceled2;
        if (!byobCanceled) readableByteStreamControllerClose(byobBranch.controller);
        if (!otherCanceled) readableByteStreamControllerClose(otherBranch.controller);
        if (chunk !== undefined) {
          if (!byobCanceled) readableByteStreamControllerRespondWithNewView(byobBranch.controller, chunk);
          if (!otherCanceled && otherBranch.controller.pendingPullIntos.length > 0) {
            readableByteStreamControllerRespond(otherBranch.controller, 0);
          }
        }
        if (!byobCanceled || !otherCanceled) deferredResolve(cancelPromise, undefined);
      },
      error() {
        reading = false;
      },
    };
    readableStreamBYOBReaderRead(reader, view, 1, readIntoRequest);
  }
  function pull1Algorithm() {
    if (reading) {
      readAgainForBranch1 = true;
      return resolvedPromise(undefined);
    }
    reading = true;
    const byobRequest = readableByteStreamControllerGetBYOBRequest(branch1.controller);
    if (byobRequest === null) pullWithDefaultReader();
    else pullWithBYOBReader(byobRequest.view, false);
    return resolvedPromise(undefined);
  }
  function pull2Algorithm() {
    if (reading) {
      readAgainForBranch2 = true;
      return resolvedPromise(undefined);
    }
    reading = true;
    const byobRequest = readableByteStreamControllerGetBYOBRequest(branch2.controller);
    if (byobRequest === null) pullWithDefaultReader();
    else pullWithBYOBReader(byobRequest.view, true);
    return resolvedPromise(undefined);
  }
  function cancel1Algorithm(reason) {
    canceled1 = true;
    reason1 = reason;
    if (canceled2) {
      deferredResolve(cancelPromise, readableStreamCancel(stream, [reason1, reason2]));
    }
    return cancelPromise.promise;
  }
  function cancel2Algorithm(reason) {
    canceled2 = true;
    reason2 = reason;
    if (canceled1) {
      deferredResolve(cancelPromise, readableStreamCancel(stream, [reason1, reason2]));
    }
    return cancelPromise.promise;
  }
  const startAlgorithm = () => undefined;
  branch1 = createReadableByteStream(startAlgorithm, pull1Algorithm, cancel1Algorithm);
  branch2 = createReadableByteStream(startAlgorithm, pull2Algorithm, cancel2Algorithm);
  forwardReaderError(reader);
  return [branch1, branch2];
}
