// WHATWG Streams (https://streams.spec.whatwg.org/): shared helpers, queuing
// strategies and the ReadableStream half. Written from the specification
// (the whatwg/streams reference implementation is BSD-3-Clause and was not
// copied). The five streams_*.js files are concatenated into one function
// body by runtime_host_streams.rs; `native` carries the few host hooks.
'use strict';
const globalObject = globalThis;
const ReflectApply = Reflect.apply;
const ObjectCreate = Object.create;
const ObjectDefineProperty = Object.defineProperty;
const ObjectGetPrototypeOf = Object.getPrototypeOf;
const ObjectGetOwnPropertyNames = Object.getOwnPropertyNames;
const ObjectGetOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
const NumberIsNaN = Number.isNaN;
const NumberIsFinite = Number.isFinite;
const MathMin = Math.min;
const MathFloor = Math.floor;
const SymbolAsyncIterator = Symbol.asyncIterator;
const SymbolIterator = Symbol.iterator;
const SymbolToStringTag = Symbol.toStringTag;
const PromiseCtor = Promise;
const PromiseThen = Promise.prototype.then;
const PromiseResolve = Promise.resolve;
const PromiseReject = Promise.reject;
const TypeErrorCtor = TypeError;
const RangeErrorCtor = RangeError;
const WeakMapCtor = WeakMap;
const WeakMapGet = WeakMap.prototype.get;
const WeakMapSet = WeakMap.prototype.set;
const WeakMapHas = WeakMap.prototype.has;
const ArrayBufferCtor = ArrayBuffer;
const Uint8ArrayCtor = Uint8Array;
const ArrayBufferIsView = ArrayBuffer.isView;
const ArrayPush = Array.prototype.push;
const ArrayShift = Array.prototype.shift;
const ArraySplice = Array.prototype.splice;
const ArrayBufferProto = ArrayBuffer.prototype;
const TypedArrayProto = ObjectGetPrototypeOf(Uint8Array.prototype);

function getterOf(proto, name) {
  return ObjectGetOwnPropertyDescriptor(proto, name).get;
}
const abByteLengthGet = getterOf(ArrayBufferProto, 'byteLength');
const abDetachedGet = getterOf(ArrayBufferProto, 'detached');
const abTransfer = ArrayBufferProto.transferToFixedLength || ArrayBufferProto.transfer;
const taBufferGet = getterOf(TypedArrayProto, 'buffer');
const taByteOffsetGet = getterOf(TypedArrayProto, 'byteOffset');
const taByteLengthGet = getterOf(TypedArrayProto, 'byteLength');
const taLengthGet = getterOf(TypedArrayProto, 'length');
const taTagGet = getterOf(TypedArrayProto, SymbolToStringTag);
const dvBufferGet = getterOf(DataView.prototype, 'buffer');
const dvByteOffsetGet = getterOf(DataView.prototype, 'byteOffset');
const dvByteLengthGet = getterOf(DataView.prototype, 'byteLength');
const viewCtors = ObjectCreate(null);
const viewElementSizes = ObjectCreate(null);
{
  const sizes = {
    Int8Array: 1, Uint8Array: 1, Uint8ClampedArray: 1, Int16Array: 2, Uint16Array: 2,
    Int32Array: 4, Uint32Array: 4, Float16Array: 2, Float32Array: 4, Float64Array: 8,
    BigInt64Array: 8, BigUint64Array: 8,
  };
  for (const name of ObjectGetOwnPropertyNames(sizes)) {
    if (typeof globalObject[name] === 'function') {
      viewCtors[name] = globalObject[name];
      viewElementSizes[name] = sizes[name];
    }
  }
}
const queueMicrotaskFn = typeof globalObject.queueMicrotask === 'function' ? globalObject.queueMicrotask : null;
function enqueueMicrotask(fn) {
  if (queueMicrotaskFn !== null) {
    ReflectApply(queueMicrotaskFn, globalObject, [fn]);
  } else {
    ReflectApply(PromiseThen, ReflectApply(PromiseResolve, PromiseCtor, [undefined]), [fn]);
  }
}

// ---- promise helpers --------------------------------------------------
function noop() {}
function newDeferred() {
  const d = { pending: true, promise: undefined, resolveFn: undefined, rejectFn: undefined };
  d.promise = new PromiseCtor((resolve, reject) => { d.resolveFn = resolve; d.rejectFn = reject; });
  return d;
}
function deferredResolve(d, value) {
  d.pending = false;
  d.resolveFn(value);
}
function deferredReject(d, reason) {
  d.pending = false;
  d.rejectFn(reason);
}
function resolvedPromise(value) {
  return ReflectApply(PromiseResolve, PromiseCtor, [value]);
}
function rejectedPromise(reason) {
  return ReflectApply(PromiseReject, PromiseCtor, [reason]);
}
function markHandled(promise) {
  ReflectApply(PromiseThen, promise, [undefined, noop]);
}
function rejectedDeferred(reason) {
  const d = newDeferred();
  deferredReject(d, reason);
  markHandled(d.promise);
  return d;
}
function resolvedDeferred(value) {
  const d = newDeferred();
  deferredResolve(d, value);
  return d;
}
// "upon fulfillment/rejection": returns nothing, swallows the derived promise.
function uponPromise(promise, onFulfilled, onRejected) {
  ReflectApply(PromiseThen, promise, [onFulfilled, onRejected === undefined ? noop : onRejected]);
}
// "transform ... with fulfillment/rejection steps": returns the derived promise.
function transformPromise(promise, onFulfilled, onRejected) {
  return ReflectApply(PromiseThen, promise, [onFulfilled, onRejected]);
}
function promiseCall(fn, thisArg, args) {
  try {
    return resolvedPromise(ReflectApply(fn, thisArg, args));
  } catch (e) {
    return rejectedPromise(e);
  }
}
function isObject(v) {
  return (typeof v === 'object' && v !== null) || typeof v === 'function';
}
function isCallable(v) {
  return typeof v === 'function';
}

// ---- brand checks -----------------------------------------------------
const brandRegistry = native.brands || {};
function makeBrand(name) {
  if (brandRegistry[name] === undefined) brandRegistry[name] = new WeakMapCtor();
  return brandRegistry[name];
}
function slotOf(map, object, what) {
  const slots = ReflectApply(WeakMapGet, map, [object]);
  if (slots === undefined) throw new TypeErrorCtor('Illegal invocation: receiver is not a ' + what);
  return slots;
}
function hasSlot(map, object) {
  return ReflectApply(WeakMapHas, map, [object]);
}
function setSlot(map, object, slots) {
  ReflectApply(WeakMapSet, map, [object, slots]);
}
const rsBrand = makeBrand('rsBrand');      // ReadableStream
const readerBrand = makeBrand('readerBrand');  // ReadableStreamDefaultReader / ReadableStreamBYOBReader
const rdcBrand = makeBrand('rdcBrand');     // ReadableStreamDefaultController
const rbcBrand = makeBrand('rbcBrand');     // ReadableByteStreamController
const brqBrand = makeBrand('brqBrand');     // ReadableStreamBYOBRequest
const wsBrand = makeBrand('wsBrand');      // WritableStream
const writerBrand = makeBrand('writerBrand');  // WritableStreamDefaultWriter
const wdcBrand = makeBrand('wdcBrand');     // WritableStreamDefaultController
const tsBrand = makeBrand('tsBrand');      // TransformStream
const tdcBrand = makeBrand('tdcBrand');     // TransformStreamDefaultController

// ---- IDL conversions --------------------------------------------------
function toDictionary(value, what) {
  if (value === undefined || value === null) return undefined;
  if (!isObject(value)) throw new TypeErrorCtor(what + ' must be an object');
  return value;
}
function toCallback(value, what) {
  if (!isCallable(value)) throw new TypeErrorCtor(what + ' must be a function');
  return value;
}
function enforceRangeUnsigned(value, what) {
  let n = +value;
  if (typeof value === 'bigint' || typeof value === 'symbol') n = Number(value);
  if (NumberIsNaN(n) || !NumberIsFinite(n)) throw new TypeErrorCtor(what + ' is not a finite number');
  n = n < 0 ? Math.ceil(n) : MathFloor(n);
  if (n === 0) n = 0;
  if (n < 0 || n > 9007199254740991) throw new TypeErrorCtor(what + ' is out of range');
  return n;
}
function toBoolean(v) {
  return !!v;
}

// ---- queuing strategies ------------------------------------------------
function convertStrategy(strategy) {
  const dict = toDictionary(strategy, 'strategy');
  const out = { highWaterMark: undefined, size: undefined };
  if (dict === undefined) return out;
  const hwm = dict.highWaterMark;
  if (hwm !== undefined) out.highWaterMark = +hwm;
  const size = dict.size;
  if (size !== undefined) out.size = toCallback(size, 'strategy.size');
  return out;
}
function extractHighWaterMark(strategy, defaultHWM) {
  const hwm = strategy.highWaterMark;
  if (hwm === undefined) return defaultHWM;
  if (NumberIsNaN(hwm) || hwm < 0) throw new RangeErrorCtor('Invalid highWaterMark');
  return hwm;
}
function extractSizeAlgorithm(strategy) {
  const size = strategy.size;
  if (size === undefined) return () => 1;
  return (chunk) => ReflectApply(size, undefined, [chunk]);
}
function isNonNegativeNumber(v) {
  return typeof v === 'number' && !NumberIsNaN(v) && v >= 0;
}
function resetQueue(container) {
  container.queue = [];
  container.queueTotalSize = 0;
}
function enqueueValueWithSize(container, value, size) {
  if (!isNonNegativeNumber(size) || size === Infinity) throw new RangeErrorCtor('Invalid chunk size');
  ReflectApply(ArrayPush, container.queue, [{ value, size }]);
  container.queueTotalSize += size;
}
function dequeueValue(container) {
  const pair = ReflectApply(ArrayShift, container.queue, []);
  container.queueTotalSize -= pair.size;
  if (container.queueTotalSize < 0) container.queueTotalSize = 0;
  return pair.value;
}
function peekQueueValue(container) {
  return container.queue[0].value;
}

const countSizeFunction = { size() { return 1; } }.size;
const byteLengthSizeFunction = { size(chunk) { return chunk.byteLength; } }.size;
const cqsHWM = makeBrand('cqsHWM');
const blqsHWM = makeBrand('blqsHWM');
function convertStrategyInit(init, what) {
  const dict = toDictionary(init, what);
  if (dict === undefined) throw new TypeErrorCtor(what + ' requires an init dictionary');
  const hwm = dict.highWaterMark;
  if (hwm === undefined) throw new TypeErrorCtor(what + ' requires highWaterMark');
  return +hwm;
}
class CountQueuingStrategy {
  constructor(init) {
    if (arguments.length < 1) throw new TypeErrorCtor('CountQueuingStrategy requires an init argument');
    setSlot(cqsHWM, this, convertStrategyInit(init, 'CountQueuingStrategy init'));
  }
  get highWaterMark() { return slotOf(cqsHWM, this, 'CountQueuingStrategy'); }
  get size() { slotOf(cqsHWM, this, 'CountQueuingStrategy'); return countSizeFunction; }
}
class ByteLengthQueuingStrategy {
  constructor(init) {
    if (arguments.length < 1) throw new TypeErrorCtor('ByteLengthQueuingStrategy requires an init argument');
    setSlot(blqsHWM, this, convertStrategyInit(init, 'ByteLengthQueuingStrategy init'));
  }
  get highWaterMark() { return slotOf(blqsHWM, this, 'ByteLengthQueuingStrategy'); }
  get size() { slotOf(blqsHWM, this, 'ByteLengthQueuingStrategy'); return byteLengthSizeFunction; }
}

// ---- ArrayBuffer / view helpers ----------------------------------------
function isDetachedBuffer(buffer) {
  return ReflectApply(abDetachedGet, buffer, []);
}
function transferArrayBuffer(buffer) {
  return ReflectApply(abTransfer, buffer, []);
}
function bufferByteLength(buffer) {
  return ReflectApply(abByteLengthGet, buffer, []);
}
function viewTag(view) {
  return ReflectApply(taTagGet, view, []);
}
function viewBuffer(view) {
  return viewTag(view) === undefined ? ReflectApply(dvBufferGet, view, []) : ReflectApply(taBufferGet, view, []);
}
function viewByteOffset(view) {
  return viewTag(view) === undefined ? ReflectApply(dvByteOffsetGet, view, []) : ReflectApply(taByteOffsetGet, view, []);
}
function viewByteLength(view) {
  return viewTag(view) === undefined ? ReflectApply(dvByteLengthGet, view, []) : ReflectApply(taByteLengthGet, view, []);
}
function copyBytes(destBuffer, destStart, srcBuffer, srcStart, count) {
  if (count === 0) return;
  const dest = new Uint8ArrayCtor(destBuffer, destStart, count);
  const src = new Uint8ArrayCtor(srcBuffer, srcStart, count);
  dest.set(src);
}
function cloneArrayBuffer(buffer, byteOffset, byteLength) {
  const clone = new ArrayBufferCtor(byteLength);
  copyBytes(clone, 0, buffer, byteOffset, byteLength);
  return clone;
}

// ---- ReadableStream abstract operations ---------------------------------
function createReadableStreamState() {
  return { self: undefined, state: 'readable', storedError: undefined, reader: undefined, disturbed: false, controller: undefined };
}
function newReadableStreamObject(proto) {
  const stream = createReadableStreamState();
  stream.self = ObjectCreate(proto);
  setSlot(rsBrand, stream.self, stream);
  return stream;
}
function isReadableStreamLocked(stream) {
  return stream.reader !== undefined;
}
function readableStreamState(object, what) {
  return slotOf(rsBrand, object, what || 'ReadableStream');
}
function createReadableStream(startAlgorithm, pullAlgorithm, cancelAlgorithm, highWaterMark, sizeAlgorithm) {
  if (highWaterMark === undefined) highWaterMark = 1;
  if (sizeAlgorithm === undefined) sizeAlgorithm = () => 1;
  const stream = newReadableStreamObject(ReadableStream.prototype);
  const controller = newDefaultController();
  setUpReadableStreamDefaultController(stream, controller, startAlgorithm, pullAlgorithm, cancelAlgorithm, highWaterMark, sizeAlgorithm);
  return stream;
}
function createReadableByteStream(startAlgorithm, pullAlgorithm, cancelAlgorithm) {
  const stream = newReadableStreamObject(ReadableStream.prototype);
  const controller = newByteController();
  setUpReadableByteStreamController(stream, controller, startAlgorithm, pullAlgorithm, cancelAlgorithm, 0, undefined);
  return stream;
}
function acquireReadableStreamDefaultReader(stream) {
  const reader = newReaderState('default', ReadableStreamDefaultReader.prototype);
  setUpReadableStreamReader(reader, stream);
  return reader;
}
function acquireReadableStreamBYOBReader(stream) {
  const reader = newReaderState('byob', ReadableStreamBYOBReader.prototype);
  setUpReadableStreamReader(reader, stream);
  return reader;
}
function newReaderState(kind, proto) {
  const reader = {
    self: ObjectCreate(proto), kind, stream: undefined, closed: undefined,
    readRequests: [], readIntoRequests: [],
  };
  setSlot(readerBrand, reader.self, reader);
  return reader;
}
function setUpReadableStreamReader(reader, stream) {
  if (isReadableStreamLocked(stream)) throw new TypeErrorCtor('ReadableStream is locked');
  if (reader.kind === 'byob' && stream.controller.kind !== 'byte') {
    throw new TypeErrorCtor('Cannot construct a ReadableStreamBYOBReader for a stream not constructed with a byte source');
  }
  readableStreamReaderGenericInitialize(reader, stream);
}
function readableStreamReaderGenericInitialize(reader, stream) {
  reader.stream = stream;
  stream.reader = reader;
  if (stream.state === 'readable') {
    reader.closed = newDeferred();
  } else if (stream.state === 'closed') {
    reader.closed = resolvedDeferred(undefined);
  } else {
    reader.closed = rejectedDeferred(stream.storedError);
  }
}
function readableStreamReaderGenericCancel(reader, reason) {
  return readableStreamCancel(reader.stream, reason);
}
function readableStreamReaderGenericRelease(reader) {
  const stream = reader.stream;
  const releasedError = new TypeErrorCtor('Reader was released');
  if (stream.state === 'readable') {
    deferredReject(reader.closed, releasedError);
  } else {
    reader.closed = rejectedDeferred(releasedError);
  }
  markHandled(reader.closed.promise);
  controllerReleaseSteps(stream.controller);
  stream.reader = undefined;
  reader.stream = undefined;
}
function readableStreamDefaultReaderRelease(reader) {
  readableStreamReaderGenericRelease(reader);
  readableStreamDefaultReaderErrorReadRequests(reader, new TypeErrorCtor('Reader was released'));
}
function readableStreamBYOBReaderRelease(reader) {
  readableStreamReaderGenericRelease(reader);
  readableStreamBYOBReaderErrorReadIntoRequests(reader, new TypeErrorCtor('Reader was released'));
}
function readableStreamDefaultReaderErrorReadRequests(reader, e) {
  const requests = reader.readRequests;
  reader.readRequests = [];
  for (let i = 0; i < requests.length; ++i) requests[i].error(e);
}
function readableStreamBYOBReaderErrorReadIntoRequests(reader, e) {
  const requests = reader.readIntoRequests;
  reader.readIntoRequests = [];
  for (let i = 0; i < requests.length; ++i) requests[i].error(e);
}
function readableStreamDefaultReaderRead(reader, readRequest) {
  const stream = reader.stream;
  stream.disturbed = true;
  if (stream.state === 'closed') {
    readRequest.close();
  } else if (stream.state === 'errored') {
    readRequest.error(stream.storedError);
  } else {
    controllerPullSteps(stream.controller, readRequest);
  }
}
function readableStreamBYOBReaderRead(reader, view, min, readIntoRequest) {
  const stream = reader.stream;
  stream.disturbed = true;
  if (stream.state === 'errored') {
    readIntoRequest.error(stream.storedError);
  } else {
    readableByteStreamControllerPullInto(stream.controller, view, min, readIntoRequest);
  }
}
function controllerCancelSteps(controller, reason) {
  return controller.kind === 'default' ? defaultControllerCancelSteps(controller, reason) : byteControllerCancelSteps(controller, reason);
}
function controllerPullSteps(controller, readRequest) {
  if (controller.kind === 'default') defaultControllerPullSteps(controller, readRequest);
  else byteControllerPullSteps(controller, readRequest);
}
function controllerReleaseSteps(controller) {
  if (controller.kind === 'byte') byteControllerReleaseSteps(controller);
}
function readableStreamCancel(stream, reason) {
  stream.disturbed = true;
  if (stream.state === 'closed') return resolvedPromise(undefined);
  if (stream.state === 'errored') return rejectedPromise(stream.storedError);
  readableStreamClose(stream);
  const reader = stream.reader;
  if (reader !== undefined && reader.kind === 'byob') {
    const requests = reader.readIntoRequests;
    reader.readIntoRequests = [];
    for (let i = 0; i < requests.length; ++i) requests[i].close(undefined);
  }
  const sourceCancelPromise = controllerCancelSteps(stream.controller, reason);
  return transformPromise(sourceCancelPromise, () => undefined);
}
function readableStreamClose(stream) {
  stream.state = 'closed';
  const reader = stream.reader;
  if (reader === undefined) return;
  deferredResolve(reader.closed, undefined);
  if (reader.kind === 'default') {
    const requests = reader.readRequests;
    reader.readRequests = [];
    for (let i = 0; i < requests.length; ++i) requests[i].close();
  }
}
function readableStreamError(stream, e) {
  stream.state = 'errored';
  stream.storedError = e;
  const reader = stream.reader;
  if (reader === undefined) return;
  deferredReject(reader.closed, e);
  markHandled(reader.closed.promise);
  if (reader.kind === 'default') readableStreamDefaultReaderErrorReadRequests(reader, e);
  else readableStreamBYOBReaderErrorReadIntoRequests(reader, e);
}
function readableStreamAddReadRequest(stream, readRequest) {
  ReflectApply(ArrayPush, stream.reader.readRequests, [readRequest]);
}
function readableStreamAddReadIntoRequest(stream, readIntoRequest) {
  ReflectApply(ArrayPush, stream.reader.readIntoRequests, [readIntoRequest]);
}
function readableStreamFulfillReadRequest(stream, chunk, done) {
  const readRequest = ReflectApply(ArrayShift, stream.reader.readRequests, []);
  if (done) readRequest.close();
  else readRequest.chunk(chunk);
}
function readableStreamFulfillReadIntoRequest(stream, chunk, done) {
  const readIntoRequest = ReflectApply(ArrayShift, stream.reader.readIntoRequests, []);
  if (done) readIntoRequest.close(chunk);
  else readIntoRequest.chunk(chunk);
}
function readableStreamGetNumReadIntoRequests(stream) {
  return stream.reader.readIntoRequests.length;
}
function readableStreamGetNumReadRequests(stream) {
  return stream.reader.readRequests.length;
}
function readableStreamHasBYOBReader(stream) {
  return stream.reader !== undefined && stream.reader.kind === 'byob';
}
function readableStreamHasDefaultReader(stream) {
  return stream.reader !== undefined && stream.reader.kind === 'default';
}
function createIterResult(value, done) {
  return { value, done };
}

// ---- ReadableStreamDefaultController -------------------------------------
function newDefaultController() {
  const c = {
    kind: 'default', self: ObjectCreate(ReadableStreamDefaultController.prototype), stream: undefined,
    queue: [], queueTotalSize: 0, started: false, closeRequested: false, pullAgain: false, pulling: false,
    strategySize: undefined, strategyHWM: 0, pullAlgorithm: undefined, cancelAlgorithm: undefined,
  };
  setSlot(rdcBrand, c.self, c);
  return c;
}
function defaultControllerCancelSteps(c, reason) {
  resetQueue(c);
  const result = c.cancelAlgorithm(reason);
  defaultControllerClearAlgorithms(c);
  return result;
}
function defaultControllerPullSteps(c, readRequest) {
  const stream = c.stream;
  if (c.queue.length > 0) {
    const chunk = dequeueValue(c);
    if (c.closeRequested && c.queue.length === 0) {
      defaultControllerClearAlgorithms(c);
      readableStreamClose(stream);
    } else {
      defaultControllerCallPullIfNeeded(c);
    }
    readRequest.chunk(chunk);
  } else {
    readableStreamAddReadRequest(stream, readRequest);
    defaultControllerCallPullIfNeeded(c);
  }
}
function defaultControllerCallPullIfNeeded(c) {
  if (!defaultControllerShouldCallPull(c)) return;
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
      defaultControllerCallPullIfNeeded(c);
    }
  }, (e) => {
    defaultControllerError(c, e);
  });
}
function defaultControllerShouldCallPull(c) {
  const stream = c.stream;
  if (!defaultControllerCanCloseOrEnqueue(c)) return false;
  if (!c.started) return false;
  if (isReadableStreamLocked(stream) && readableStreamGetNumReadRequests(stream) > 0) return true;
  return defaultControllerGetDesiredSize(c) > 0;
}
function defaultControllerClearAlgorithms(c) {
  c.pullAlgorithm = undefined;
  c.cancelAlgorithm = undefined;
  c.strategySize = undefined;
}
function defaultControllerClose(c) {
  if (!defaultControllerCanCloseOrEnqueue(c)) return;
  const stream = c.stream;
  c.closeRequested = true;
  if (c.queue.length === 0) {
    defaultControllerClearAlgorithms(c);
    readableStreamClose(stream);
  }
}
function defaultControllerEnqueue(c, chunk) {
  if (!defaultControllerCanCloseOrEnqueue(c)) return;
  const stream = c.stream;
  if (isReadableStreamLocked(stream) && readableStreamGetNumReadRequests(stream) > 0) {
    readableStreamFulfillReadRequest(stream, chunk, false);
  } else {
    let chunkSize;
    try {
      chunkSize = c.strategySize(chunk);
    } catch (e) {
      defaultControllerError(c, e);
      throw e;
    }
    try {
      enqueueValueWithSize(c, chunk, chunkSize);
    } catch (e) {
      defaultControllerError(c, e);
      throw e;
    }
  }
  defaultControllerCallPullIfNeeded(c);
}
function defaultControllerError(c, e) {
  const stream = c.stream;
  if (stream.state !== 'readable') return;
  resetQueue(c);
  defaultControllerClearAlgorithms(c);
  readableStreamError(stream, e);
}
function defaultControllerGetDesiredSize(c) {
  const state = c.stream.state;
  if (state === 'errored') return null;
  if (state === 'closed') return 0;
  return c.strategyHWM - c.queueTotalSize;
}
function defaultControllerHasBackpressure(c) {
  return !defaultControllerShouldCallPull(c);
}
function defaultControllerCanCloseOrEnqueue(c) {
  return !c.closeRequested && c.stream.state === 'readable';
}
function setUpReadableStreamDefaultController(stream, controller, startAlgorithm, pullAlgorithm, cancelAlgorithm, highWaterMark, sizeAlgorithm) {
  controller.stream = stream;
  resetQueue(controller);
  controller.started = false;
  controller.closeRequested = false;
  controller.pullAgain = false;
  controller.pulling = false;
  controller.strategySize = sizeAlgorithm;
  controller.strategyHWM = highWaterMark;
  controller.pullAlgorithm = pullAlgorithm;
  controller.cancelAlgorithm = cancelAlgorithm;
  stream.controller = controller;
  const startResult = startAlgorithm();
  const startPromise = resolvedPromise(startResult);
  uponPromise(startPromise, () => {
    controller.started = true;
    defaultControllerCallPullIfNeeded(controller);
  }, (r) => {
    defaultControllerError(controller, r);
  });
}
function setUpReadableStreamDefaultControllerFromUnderlyingSource(stream, source, dict, highWaterMark, sizeAlgorithm) {
  const controller = newDefaultController();
  const startAlgorithm = dict.start === undefined ? () => undefined : () => ReflectApply(dict.start, source, [controller.self]);
  const pullAlgorithm = dict.pull === undefined ? () => resolvedPromise(undefined) : () => promiseCall(dict.pull, source, [controller.self]);
  const cancelAlgorithm = dict.cancel === undefined ? () => resolvedPromise(undefined) : (reason) => promiseCall(dict.cancel, source, [reason]);
  setUpReadableStreamDefaultController(stream, controller, startAlgorithm, pullAlgorithm, cancelAlgorithm, highWaterMark, sizeAlgorithm);
}

// ---- underlying source dictionary ---------------------------------------
function convertUnderlyingSource(source) {
  const obj = toDictionary(source, 'underlyingSource');
  const dict = { autoAllocateChunkSize: undefined, cancel: undefined, pull: undefined, start: undefined, type: undefined };
  if (obj === undefined) return dict;
  let v = obj.autoAllocateChunkSize;
  if (v !== undefined) dict.autoAllocateChunkSize = enforceRangeUnsigned(v, 'autoAllocateChunkSize');
  v = obj.cancel;
  if (v !== undefined) dict.cancel = toCallback(v, 'underlyingSource.cancel');
  v = obj.pull;
  if (v !== undefined) dict.pull = toCallback(v, 'underlyingSource.pull');
  v = obj.start;
  if (v !== undefined) dict.start = toCallback(v, 'underlyingSource.start');
  v = obj.type;
  if (v !== undefined) {
    const type = '' + v;
    if (type !== 'bytes') throw new TypeErrorCtor('Invalid ReadableStream type');
    dict.type = type;
  }
  return dict;
}

// ---- ReadableStream ------------------------------------------------------
class ReadableStream {
  constructor(underlyingSource = undefined, strategy = undefined) {
    if (underlyingSource === null) throw new TypeErrorCtor('underlyingSource must not be null');
    const strategyDict = convertStrategy(strategy);
    const dict = convertUnderlyingSource(underlyingSource);
    const state = createReadableStreamState();
    state.self = this;
    setSlot(rsBrand, this, state);
    const source = underlyingSource === null ? undefined : underlyingSource;
    if (dict.type === 'bytes') {
      if (strategyDict.size !== undefined) throw new RangeErrorCtor('The strategy for a byte stream cannot have a size function');
      const highWaterMark = extractHighWaterMark(strategyDict, 0);
      setUpReadableByteStreamControllerFromUnderlyingSource(state, source, dict, highWaterMark);
    } else {
      const sizeAlgorithm = extractSizeAlgorithm(strategyDict);
      const highWaterMark = extractHighWaterMark(strategyDict, 1);
      setUpReadableStreamDefaultControllerFromUnderlyingSource(state, source, dict, highWaterMark, sizeAlgorithm);
    }
  }
  get locked() {
    return isReadableStreamLocked(readableStreamState(this));
  }
  cancel(reason = undefined) {
    if (!hasSlot(rsBrand, this)) return rejectedPromise(new TypeErrorCtor('Illegal invocation: receiver is not a ReadableStream'));
    const stream = readableStreamState(this);
    if (isReadableStreamLocked(stream)) return rejectedPromise(new TypeErrorCtor('Cannot cancel a locked ReadableStream'));
    return readableStreamCancel(stream, reason);
  }
  getReader(options = undefined) {
    const stream = readableStreamState(this);
    const dict = toDictionary(options, 'options');
    let mode;
    if (dict !== undefined) {
      mode = dict.mode;
      if (mode !== undefined) {
        mode = '' + mode;
        if (mode !== 'byob') throw new TypeErrorCtor('Invalid reader mode');
      }
    }
    if (mode === undefined) return acquireReadableStreamDefaultReader(stream).self;
    return acquireReadableStreamBYOBReader(stream).self;
  }
  pipeThrough(transform, options = undefined) {
    const stream = readableStreamState(this);
    if (arguments.length < 1) throw new TypeErrorCtor('pipeThrough requires a transform argument');
    const transformDict = toDictionary(transform, 'transform');
    if (transformDict === undefined) throw new TypeErrorCtor('transform must be an object');
    const readable = transformDict.readable;
    if (!hasSlot(rsBrand, readable)) throw new TypeErrorCtor('transform.readable must be a ReadableStream');
    const writable = transformDict.writable;
    if (!hasSlot(wsBrand, writable)) throw new TypeErrorCtor('transform.writable must be a WritableStream');
    const opts = convertPipeOptions(options);
    if (isReadableStreamLocked(stream)) throw new TypeErrorCtor('Cannot pipe a locked ReadableStream');
    const writableState = slotOf(wsBrand, writable, 'WritableStream');
    if (isWritableStreamLocked(writableState)) throw new TypeErrorCtor('Cannot pipe to a locked WritableStream');
    const promise = readableStreamPipeTo(stream, writableState, opts.preventClose, opts.preventAbort, opts.preventCancel, opts.signal);
    markHandled(promise);
    return readable;
  }
  pipeTo(destination, options = undefined) {
    if (!hasSlot(rsBrand, this)) return rejectedPromise(new TypeErrorCtor('Illegal invocation: receiver is not a ReadableStream'));
    if (arguments.length < 1) return rejectedPromise(new TypeErrorCtor('pipeTo requires a destination argument'));
    let opts;
    try {
      if (!hasSlot(wsBrand, destination)) throw new TypeErrorCtor('destination must be a WritableStream');
      opts = convertPipeOptions(options);
    } catch (e) {
      return rejectedPromise(e);
    }
    const stream = readableStreamState(this);
    const dest = slotOf(wsBrand, destination, 'WritableStream');
    if (isReadableStreamLocked(stream)) return rejectedPromise(new TypeErrorCtor('Cannot pipe a locked ReadableStream'));
    if (isWritableStreamLocked(dest)) return rejectedPromise(new TypeErrorCtor('Cannot pipe to a locked WritableStream'));
    return readableStreamPipeTo(stream, dest, opts.preventClose, opts.preventAbort, opts.preventCancel, opts.signal);
  }
  tee() {
    const stream = readableStreamState(this);
    const branches = readableStreamTee(stream, false);
    return [branches[0].self, branches[1].self];
  }
  values(options = undefined) {
    const stream = readableStreamState(this);
    const dict = toDictionary(options, 'options');
    const preventCancel = dict === undefined ? false : toBoolean(dict.preventCancel);
    return createAsyncIterator(stream, preventCancel);
  }
  static from(asyncIterable) {
    if (arguments.length < 1) throw new TypeErrorCtor('ReadableStream.from requires an argument');
    if (typeof asyncIterable === 'string') throw new TypeErrorCtor('ReadableStream.from does not accept a string');
    return readableStreamFromIterable(asyncIterable).self;
  }
}

function convertPipeOptions(options) {
  const dict = toDictionary(options, 'options');
  const out = { preventAbort: false, preventCancel: false, preventClose: false, signal: undefined };
  if (dict === undefined) return out;
  out.preventAbort = toBoolean(dict.preventAbort);
  out.preventCancel = toBoolean(dict.preventCancel);
  out.preventClose = toBoolean(dict.preventClose);
  const signal = dict.signal;
  if (signal !== undefined) {
    if (!isAbortSignal(signal)) throw new TypeErrorCtor('options.signal must be an AbortSignal');
    out.signal = signal;
  }
  return out;
}

// AbortSignal plumbing (captured once so later patching cannot affect streams).
const AbortSignalCtor = globalObject.AbortSignal;
const AbortControllerCtor = globalObject.AbortController;
const signalAbortedGet = AbortSignalCtor ? getterOf(AbortSignalCtor.prototype, 'aborted') : undefined;
const signalReasonGet = AbortSignalCtor ? getterOf(AbortSignalCtor.prototype, 'reason') : undefined;
const eventTargetAdd = globalObject.EventTarget ? globalObject.EventTarget.prototype.addEventListener : undefined;
const eventTargetRemove = globalObject.EventTarget ? globalObject.EventTarget.prototype.removeEventListener : undefined;
const abortControllerSignalGet = AbortControllerCtor ? getterOf(AbortControllerCtor.prototype, 'signal') : undefined;
const abortControllerAbort = AbortControllerCtor ? AbortControllerCtor.prototype.abort : undefined;
function isAbortSignal(v) {
  if (signalAbortedGet === undefined || !isObject(v)) return false;
  try {
    ReflectApply(signalAbortedGet, v, []);
    return true;
  } catch (e) {
    return false;
  }
}
function signalAborted(signal) {
  return ReflectApply(signalAbortedGet, signal, []);
}
function signalReason(signal) {
  return ReflectApply(signalReasonGet, signal, []);
}

// ---- tee -------------------------------------------------------------------
function readableStreamTee(stream, cloneForBranch2) {
  if (stream.controller.kind === 'byte') return readableByteStreamTee(stream);
  return readableStreamDefaultTee(stream, cloneForBranch2);
}
function readableStreamDefaultTee(stream, cloneForBranch2) {
  const reader = acquireReadableStreamDefaultReader(stream);
  let reading = false;
  let readAgain = false;
  let canceled1 = false;
  let canceled2 = false;
  let reason1;
  let reason2;
  let branch1;
  let branch2;
  const cancelPromise = newDeferred();
  function pullAlgorithm() {
    if (reading) {
      readAgain = true;
      return resolvedPromise(undefined);
    }
    reading = true;
    const readRequest = {
      chunk(chunk) {
        enqueueMicrotask(() => {
          readAgain = false;
          const chunk1 = chunk;
          let chunk2 = chunk;
          if (!canceled2 && cloneForBranch2) {
            try {
              chunk2 = structuredCloneValue(chunk2);
            } catch (cloneError) {
              defaultControllerError(branch1.controller, cloneError);
              defaultControllerError(branch2.controller, cloneError);
              deferredResolve(cancelPromise, readableStreamCancel(stream, cloneError));
              return;
            }
          }
          if (!canceled1) defaultControllerEnqueue(branch1.controller, chunk1);
          if (!canceled2) defaultControllerEnqueue(branch2.controller, chunk2);
          reading = false;
          if (readAgain) pullAlgorithm();
        });
      },
      close() {
        reading = false;
        if (!canceled1) defaultControllerClose(branch1.controller);
        if (!canceled2) defaultControllerClose(branch2.controller);
        if (!canceled1 || !canceled2) deferredResolve(cancelPromise, undefined);
      },
      error() {
        reading = false;
      },
    };
    readableStreamDefaultReaderRead(reader, readRequest);
    return resolvedPromise(undefined);
  }
  function cancel1Algorithm(reason) {
    canceled1 = true;
    reason1 = reason;
    if (canceled2) {
      const compositeReason = [reason1, reason2];
      deferredResolve(cancelPromise, readableStreamCancel(stream, compositeReason));
    }
    return cancelPromise.promise;
  }
  function cancel2Algorithm(reason) {
    canceled2 = true;
    reason2 = reason;
    if (canceled1) {
      const compositeReason = [reason1, reason2];
      deferredResolve(cancelPromise, readableStreamCancel(stream, compositeReason));
    }
    return cancelPromise.promise;
  }
  const startAlgorithm = () => undefined;
  branch1 = createReadableStream(startAlgorithm, pullAlgorithm, cancel1Algorithm);
  branch2 = createReadableStream(startAlgorithm, pullAlgorithm, cancel2Algorithm);
  uponPromise(reader.closed.promise, undefined, (r) => {
    defaultControllerError(branch1.controller, r);
    defaultControllerError(branch2.controller, r);
    if (!canceled1 || !canceled2) deferredResolve(cancelPromise, undefined);
  });
  return [branch1, branch2];
}
function structuredCloneValue(value) {
  return globalObject.structuredClone(value);
}

// ---- pipeTo ----------------------------------------------------------------
function readableStreamPipeTo(source, dest, preventClose, preventAbort, preventCancel, signal) {
  const reader = acquireReadableStreamDefaultReader(source);
  const writer = acquireWritableStreamDefaultWriter(dest);
  source.disturbed = true;
  let shuttingDown = false;
  const promise = newDeferred();
  let currentWrite = resolvedPromise(undefined);
  let abortAlgorithm;

  function finalize(isError, error) {
    writableStreamDefaultWriterRelease(writer);
    readableStreamDefaultReaderRelease(reader);
    if (signal !== undefined && abortAlgorithm !== undefined) {
      ReflectApply(eventTargetRemove, signal, ['abort', abortAlgorithm]);
    }
    if (isError) deferredReject(promise, error);
    else deferredResolve(promise, undefined);
  }
  function waitForWritesToFinish() {
    const oldCurrentWrite = currentWrite;
    return transformPromise(currentWrite, () => (oldCurrentWrite !== currentWrite ? waitForWritesToFinish() : undefined));
  }
  function shutdownWithAction(action, originalIsError, originalError) {
    if (shuttingDown) return;
    shuttingDown = true;
    function doTheRest() {
      uponPromise(action(), () => finalize(originalIsError, originalError), (newError) => finalize(true, newError));
    }
    if (dest.state === 'writable' && !writableStreamCloseQueuedOrInFlight(dest)) {
      uponPromise(waitForWritesToFinish(), doTheRest, doTheRest);
    } else {
      doTheRest();
    }
  }
  function shutdown(isError, error) {
    if (shuttingDown) return;
    shuttingDown = true;
    if (dest.state === 'writable' && !writableStreamCloseQueuedOrInFlight(dest)) {
      uponPromise(waitForWritesToFinish(), () => finalize(isError, error), () => finalize(isError, error));
    } else {
      finalize(isError, error);
    }
  }
  function isOrBecomesErrored(stream, p, action) {
    if (stream.state === 'errored') action(stream.storedError);
    else uponPromise(p, undefined, action);
  }
  function isOrBecomesClosed(stream, p, action) {
    if (stream.state === 'closed') action();
    else uponPromise(p, action, undefined);
  }

  if (signal !== undefined) {
    abortAlgorithm = () => {
      const error = signalReason(signal);
      const actions = [];
      if (!preventAbort) {
        ReflectApply(ArrayPush, actions, [() => (dest.state === 'writable' ? writableStreamAbort(dest, error) : resolvedPromise(undefined))]);
      }
      if (!preventCancel) {
        ReflectApply(ArrayPush, actions, [() => (source.state === 'readable' ? readableStreamCancel(source, error) : resolvedPromise(undefined))]);
      }
      shutdownWithAction(() => waitForAll(actions), true, error);
    };
    if (signalAborted(signal)) {
      abortAlgorithm();
      return promise.promise;
    }
    ReflectApply(eventTargetAdd, signal, ['abort', abortAlgorithm]);
  }

  isOrBecomesErrored(source, reader.closed.promise, (storedError) => {
    if (!preventAbort) shutdownWithAction(() => writableStreamAbort(dest, storedError), true, storedError);
    else shutdown(true, storedError);
  });
  isOrBecomesErrored(dest, writer.closed.promise, (storedError) => {
    if (!preventCancel) shutdownWithAction(() => readableStreamCancel(source, storedError), true, storedError);
    else shutdown(true, storedError);
  });
  isOrBecomesClosed(source, reader.closed.promise, () => {
    if (!preventClose) shutdownWithAction(() => writableStreamDefaultWriterCloseWithErrorPropagation(writer));
    else shutdown();
  });
  if (writableStreamCloseQueuedOrInFlight(dest) || dest.state === 'closed') {
    const destClosed = new TypeErrorCtor('the destination writable stream closed before all data could be piped to it');
    if (!preventCancel) shutdownWithAction(() => readableStreamCancel(source, destClosed), true, destClosed);
    else shutdown(true, destClosed);
  }

  function pipeStep() {
    if (shuttingDown) return resolvedPromise(true);
    return transformPromise(writer.ready.promise, () => new PromiseCtor((resolveRead, rejectRead) => {
      readableStreamDefaultReaderRead(reader, {
        chunk(chunk) {
          enqueueMicrotask(() => {
            currentWrite = transformPromise(writableStreamDefaultWriterWrite(writer, chunk), undefined, noop);
            resolveRead(false);
          });
        },
        close() { resolveRead(true); },
        error(e) { rejectRead(e); },
      });
    }));
  }
  function pipeLoop() {
    return new PromiseCtor((resolveLoop, rejectLoop) => {
      function next(done) {
        if (done) resolveLoop();
        else uponPromise(pipeStep(), next, rejectLoop);
      }
      next(false);
    });
  }
  markHandled(pipeLoop());
  return promise.promise;
}
function waitForAll(actions) {
  const promises = [];
  for (let i = 0; i < actions.length; ++i) ReflectApply(ArrayPush, promises, [actions[i]()]);
  return ReflectApply(PromiseAllFn, PromiseCtor, [promises]);
}
const PromiseAllFn = Promise.all;

// ---- readers -----------------------------------------------------------------
class ReadableStreamDefaultReader {
  constructor(stream) {
    if (arguments.length < 1) throw new TypeErrorCtor('ReadableStreamDefaultReader requires a stream');
    if (!hasSlot(rsBrand, stream)) throw new TypeErrorCtor('stream must be a ReadableStream');
    const state = newReaderStateFor(this, 'default');
    setUpReadableStreamReader(state, readableStreamState(stream));
  }
  get closed() {
    if (!hasSlot(readerBrand, this) || slotOf(readerBrand, this).kind !== 'default') {
      return rejectedPromise(new TypeErrorCtor('Illegal invocation: receiver is not a ReadableStreamDefaultReader'));
    }
    return slotOf(readerBrand, this).closed.promise;
  }
  cancel(reason = undefined) {
    const reader = defaultReaderState(this);
    if (reader instanceof Promise) return reader;
    if (reader.stream === undefined) return rejectedPromise(new TypeErrorCtor('Reader has been released'));
    return readableStreamReaderGenericCancel(reader, reason);
  }
  read() {
    const reader = defaultReaderState(this);
    if (reader instanceof Promise) return reader;
    if (reader.stream === undefined) return rejectedPromise(new TypeErrorCtor('Reader has been released'));
    const deferred = newDeferred();
    readableStreamDefaultReaderRead(reader, {
      chunk(chunk) { deferredResolve(deferred, createIterResult(chunk, false)); },
      close() { deferredResolve(deferred, createIterResult(undefined, true)); },
      error(e) { deferredReject(deferred, e); },
    });
    return deferred.promise;
  }
  releaseLock() {
    if (!hasSlot(readerBrand, this) || slotOf(readerBrand, this).kind !== 'default') {
      throw new TypeErrorCtor('Illegal invocation: receiver is not a ReadableStreamDefaultReader');
    }
    const reader = slotOf(readerBrand, this);
    if (reader.stream === undefined) return;
    readableStreamDefaultReaderRelease(reader);
  }
}
function defaultReaderState(object) {
  const reader = ReflectApply(WeakMapGet, readerBrand, [object]);
  if (reader === undefined || reader.kind !== 'default') {
    return rejectedPromise(new TypeErrorCtor('Illegal invocation: receiver is not a ReadableStreamDefaultReader'));
  }
  return reader;
}
function newReaderStateFor(self, kind) {
  const reader = {
    self, kind, stream: undefined, closed: undefined, readRequests: [], readIntoRequests: [],
  };
  setSlot(readerBrand, self, reader);
  return reader;
}

class ReadableStreamBYOBReader {
  constructor(stream) {
    if (arguments.length < 1) throw new TypeErrorCtor('ReadableStreamBYOBReader requires a stream');
    if (!hasSlot(rsBrand, stream)) throw new TypeErrorCtor('stream must be a ReadableStream');
    const state = newReaderStateFor(this, 'byob');
    setUpReadableStreamReader(state, readableStreamState(stream));
  }
  get closed() {
    if (!hasSlot(readerBrand, this) || slotOf(readerBrand, this).kind !== 'byob') {
      return rejectedPromise(new TypeErrorCtor('Illegal invocation: receiver is not a ReadableStreamBYOBReader'));
    }
    return slotOf(readerBrand, this).closed.promise;
  }
  cancel(reason = undefined) {
    const reader = byobReaderState(this);
    if (reader instanceof Promise) return reader;
    if (reader.stream === undefined) return rejectedPromise(new TypeErrorCtor('Reader has been released'));
    return readableStreamReaderGenericCancel(reader, reason);
  }
  read(view, options = undefined) {
    const reader = byobReaderState(this);
    if (reader instanceof Promise) return reader;
    let min;
    try {
      if (arguments.length < 1) throw new TypeErrorCtor('read requires a view argument');
      if (!ReflectApply(ArrayBufferIsView, ArrayBufferCtor, [view])) throw new TypeErrorCtor('view must be an ArrayBufferView');
      const dict = toDictionary(options, 'options');
      min = 1;
      if (dict !== undefined && dict.min !== undefined) min = enforceRangeUnsigned(dict.min, 'options.min');
    } catch (e) {
      return rejectedPromise(e);
    }
    let byteLength;
    try {
      byteLength = viewByteLength(view);
      if (byteLength === 0) return rejectedPromise(new TypeErrorCtor('view must have non-zero byteLength'));
      if (bufferByteLength(viewBuffer(view)) === 0) return rejectedPromise(new TypeErrorCtor("view's buffer must have non-zero byteLength"));
      if (isDetachedBuffer(viewBuffer(view))) return rejectedPromise(new TypeErrorCtor("view's buffer has been detached"));
      if (min === 0) return rejectedPromise(new TypeErrorCtor('options.min must be greater than 0'));
      if (viewTag(view) !== undefined) {
        if (min > ReflectApply(taLengthGet, view, [])) return rejectedPromise(new RangeErrorCtor("options.min must be less than or equal to view's length"));
      } else if (min > byteLength) {
        return rejectedPromise(new RangeErrorCtor("options.min must be less than or equal to view's byteLength"));
      }
    } catch (e) {
      return rejectedPromise(e);
    }
    if (reader.stream === undefined) return rejectedPromise(new TypeErrorCtor('Reader has been released'));
    const deferred = newDeferred();
    readableStreamBYOBReaderRead(reader, view, min, {
      chunk(chunk) { deferredResolve(deferred, createIterResult(chunk, false)); },
      close(chunk) { deferredResolve(deferred, createIterResult(chunk, true)); },
      error(e) { deferredReject(deferred, e); },
    });
    return deferred.promise;
  }
  releaseLock() {
    if (!hasSlot(readerBrand, this) || slotOf(readerBrand, this).kind !== 'byob') {
      throw new TypeErrorCtor('Illegal invocation: receiver is not a ReadableStreamBYOBReader');
    }
    const reader = slotOf(readerBrand, this);
    if (reader.stream === undefined) return;
    readableStreamBYOBReaderRelease(reader);
  }
}
function byobReaderState(object) {
  const reader = ReflectApply(WeakMapGet, readerBrand, [object]);
  if (reader === undefined || reader.kind !== 'byob') {
    return rejectedPromise(new TypeErrorCtor('Illegal invocation: receiver is not a ReadableStreamBYOBReader'));
  }
  return reader;
}

// ---- ReadableStreamDefaultController (public) -------------------------------
class ReadableStreamDefaultController {
  constructor() {
    throw new TypeErrorCtor('Illegal constructor');
  }
  get desiredSize() {
    return defaultControllerGetDesiredSize(slotOf(rdcBrand, this, 'ReadableStreamDefaultController'));
  }
  close() {
    const c = slotOf(rdcBrand, this, 'ReadableStreamDefaultController');
    if (!defaultControllerCanCloseOrEnqueue(c)) throw new TypeErrorCtor('The stream is not in a state that permits close');
    defaultControllerClose(c);
  }
  enqueue(chunk = undefined) {
    const c = slotOf(rdcBrand, this, 'ReadableStreamDefaultController');
    if (!defaultControllerCanCloseOrEnqueue(c)) throw new TypeErrorCtor('The stream is not in a state that permits enqueue');
    defaultControllerEnqueue(c, chunk);
  }
  error(e = undefined) {
    defaultControllerError(slotOf(rdcBrand, this, 'ReadableStreamDefaultController'), e);
  }
}

// ---- async iteration ------------------------------------------------------------
const AsyncIteratorPrototype = ObjectGetPrototypeOf(ObjectGetPrototypeOf(async function* () {}).prototype);
const asyncIterBrand = makeBrand('asyncIterBrand');
const ReadableStreamAsyncIteratorPrototype = ObjectCreate(AsyncIteratorPrototype);
function createAsyncIterator(stream, preventCancel) {
  const reader = acquireReadableStreamDefaultReader(stream);
  const iterator = ObjectCreate(ReadableStreamAsyncIteratorPrototype);
  setSlot(asyncIterBrand, iterator, { reader, preventCancel, ongoingPromise: null, isFinished: false });
  return iterator;
}
function asyncIteratorNextSteps(state) {
  if (state.isFinished) return resolvedPromise(createIterResult(undefined, true));
  const reader = state.reader;
  const deferred = newDeferred();
  readableStreamDefaultReaderRead(reader, {
    chunk(chunk) { deferredResolve(deferred, createIterResult(chunk, false)); },
    close() {
      state.isFinished = true;
      readableStreamDefaultReaderRelease(reader);
      deferredResolve(deferred, createIterResult(undefined, true));
    },
    error(e) {
      state.isFinished = true;
      readableStreamDefaultReaderRelease(reader);
      deferredReject(deferred, e);
    },
  });
  return deferred.promise;
}
function asyncIteratorReturnSteps(state, value) {
  if (state.isFinished) return resolvedPromise(createIterResult(value, true));
  state.isFinished = true;
  const reader = state.reader;
  if (!state.preventCancel) {
    const result = readableStreamReaderGenericCancel(reader, value);
    readableStreamDefaultReaderRelease(reader);
    return transformPromise(result, () => createIterResult(value, true));
  }
  readableStreamDefaultReaderRelease(reader);
  return resolvedPromise(createIterResult(value, true));
}
function chainOngoing(state, steps) {
  const ongoing = state.ongoingPromise;
  if (ongoing !== null) {
    const after = transformPromise(ongoing, steps, steps);
    state.ongoingPromise = after;
    return after;
  }
  const result = steps();
  state.ongoingPromise = result;
  return result;
}
{
  const proto = ReadableStreamAsyncIteratorPrototype;
  const methods = {
    next() {
      const state = ReflectApply(WeakMapGet, asyncIterBrand, [this]);
      if (state === undefined) return rejectedPromise(new TypeErrorCtor('Illegal invocation'));
      return chainOngoing(state, () => asyncIteratorNextSteps(state));
    },
    return(value) {
      const state = ReflectApply(WeakMapGet, asyncIterBrand, [this]);
      if (state === undefined) return rejectedPromise(new TypeErrorCtor('Illegal invocation'));
      return chainOngoing(state, () => asyncIteratorReturnSteps(state, value));
    },
  };
  ObjectDefineProperty(proto, 'next', { value: methods.next, writable: true, enumerable: true, configurable: true });
  ObjectDefineProperty(proto, 'return', { value: methods.return, writable: true, enumerable: true, configurable: true });
  ObjectDefineProperty(proto, SymbolToStringTag, { value: 'ReadableStream Async Iterator', configurable: true });
}

// ---- ReadableStream.from ------------------------------------------------------------
function readableStreamFromIterable(asyncIterable) {
  if (asyncIterable === undefined || asyncIterable === null) throw new TypeErrorCtor('ReadableStream.from requires an iterable');
  let iterator;
  let nextMethod;
  let method = asyncIterable[SymbolAsyncIterator];
  if (method === undefined || method === null) {
    const syncMethod = asyncIterable[SymbolIterator];
    if (syncMethod === undefined || syncMethod === null) throw new TypeErrorCtor('object is not iterable');
    if (!isCallable(syncMethod)) throw new TypeErrorCtor('Symbol.iterator is not a function');
    const syncIterator = ReflectApply(syncMethod, asyncIterable, []);
    if (!isObject(syncIterator)) throw new TypeErrorCtor('iterator is not an object');
    const syncNext = syncIterator.next;
    iterator = createAsyncFromSyncIterator(syncIterator, syncNext);
    nextMethod = iterator.next;
  } else {
    if (!isCallable(method)) throw new TypeErrorCtor('Symbol.asyncIterator is not a function');
    iterator = ReflectApply(method, asyncIterable, []);
    if (!isObject(iterator)) throw new TypeErrorCtor('iterator is not an object');
    nextMethod = iterator.next;
  }
  let stream;
  const startAlgorithm = () => undefined;
  const pullAlgorithm = () => {
    let nextResult;
    try {
      nextResult = ReflectApply(nextMethod, iterator, []);
    } catch (e) {
      return rejectedPromise(e);
    }
    const nextPromise = resolvedPromise(nextResult);
    return transformPromise(nextPromise, (iterResult) => {
      if (!isObject(iterResult)) throw new TypeErrorCtor('iterator result is not an object');
      const done = !!iterResult.done;
      if (done) {
        defaultControllerClose(stream.controller);
      } else {
        defaultControllerEnqueue(stream.controller, iterResult.value);
      }
    });
  };
  const cancelAlgorithm = (reason) => {
    let returnMethod;
    let returnResult;
    try {
      returnMethod = iterator.return;
      if (returnMethod === undefined || returnMethod === null) return resolvedPromise(undefined);
      if (!isCallable(returnMethod)) throw new TypeErrorCtor('iterator.return is not a function');
      returnResult = ReflectApply(returnMethod, iterator, [reason]);
    } catch (e) {
      return rejectedPromise(e);
    }
    return transformPromise(resolvedPromise(returnResult), (iterResult) => {
      if (!isObject(iterResult)) throw new TypeErrorCtor('iterator result is not an object');
      return undefined;
    });
  };
  stream = createReadableStream(startAlgorithm, pullAlgorithm, cancelAlgorithm, 0);
  return stream;
}
function createAsyncFromSyncIterator(syncIterator, syncNext) {
  function continuation(result, closeOnRejection) {
    if (!isObject(result)) throw new TypeErrorCtor('iterator result is not an object');
    const done = !!result.done;
    const value = result.value;
    return transformPromise(resolvedPromise(value), (v) => createIterResult(v, done), (e) => {
      if (!done && closeOnRejection) {
        try {
          const ret = syncIterator.return;
          if (ret !== undefined && ret !== null) ReflectApply(ret, syncIterator, []);
        } catch (ignored) { /* the original rejection wins */ }
      }
      throw e;
    });
  }
  return {
    next() {
      try {
        return continuation(ReflectApply(syncNext, syncIterator, arguments.length > 0 ? [arguments[0]] : []), true);
      } catch (e) {
        return rejectedPromise(e);
      }
    },
    return(value) {
      try {
        const ret = syncIterator.return;
        if (ret === undefined || ret === null) return resolvedPromise(createIterResult(value, true));
        const result = ReflectApply(ret, syncIterator, arguments.length > 0 ? [value] : []);
        if (!isObject(result)) throw new TypeErrorCtor('iterator result is not an object');
        return continuation(result, false);
      } catch (e) {
        return rejectedPromise(e);
      }
    },
  };
}
