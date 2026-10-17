// TransformStream, TextEncoderStream, TextDecoderStream.

function createTransformStreamState() {
  return {
    self: undefined, readable: undefined, writable: undefined, backpressure: undefined,
    backpressureChangePromise: undefined, controller: undefined,
  };
}
function initializeTransformStream(stream, startPromise, writableHWM, writableSizeAlgorithm, readableHWM, readableSizeAlgorithm) {
  const startAlgorithm = () => startPromise.promise;
  const writeAlgorithm = (chunk) => transformStreamDefaultSinkWriteAlgorithm(stream, chunk);
  const abortAlgorithm = (reason) => transformStreamDefaultSinkAbortAlgorithm(stream, reason);
  const closeAlgorithm = () => transformStreamDefaultSinkCloseAlgorithm(stream);
  stream.writable = createWritableStream(startAlgorithm, writeAlgorithm, closeAlgorithm, abortAlgorithm, writableHWM, writableSizeAlgorithm);
  const pullAlgorithm = () => transformStreamDefaultSourcePullAlgorithm(stream);
  const cancelAlgorithm = (reason) => transformStreamDefaultSourceCancelAlgorithm(stream, reason);
  stream.readable = createReadableStream(startAlgorithm, pullAlgorithm, cancelAlgorithm, readableHWM, readableSizeAlgorithm);
  stream.backpressure = undefined;
  stream.backpressureChangePromise = undefined;
  transformStreamSetBackpressure(stream, true);
  stream.controller = undefined;
}
function transformStreamError(stream, e) {
  defaultControllerError(stream.readable.controller, e);
  transformStreamErrorWritableAndUnblockWrite(stream, e);
}
function transformStreamErrorWritableAndUnblockWrite(stream, e) {
  transformStreamDefaultControllerClearAlgorithms(stream.controller);
  writableStreamDefaultControllerErrorIfNeeded(stream.writable.controller, e);
  transformStreamUnblockWrite(stream);
}
function transformStreamUnblockWrite(stream) {
  if (stream.backpressure === true) transformStreamSetBackpressure(stream, false);
}
function transformStreamSetBackpressure(stream, backpressure) {
  if (stream.backpressureChangePromise !== undefined) deferredResolve(stream.backpressureChangePromise, undefined);
  stream.backpressureChangePromise = newDeferred();
  stream.backpressure = backpressure;
}
function newTransformController() {
  const c = {
    self: ObjectCreate(TransformStreamDefaultController.prototype), stream: undefined, flushAlgorithm: undefined,
    transformAlgorithm: undefined, cancelAlgorithm: undefined, finishPromise: undefined,
  };
  setSlot(tdcBrand, c.self, c);
  return c;
}
function setUpTransformStreamDefaultController(stream, controller, transformAlgorithm, flushAlgorithm, cancelAlgorithm) {
  controller.stream = stream;
  stream.controller = controller;
  controller.transformAlgorithm = transformAlgorithm;
  controller.flushAlgorithm = flushAlgorithm;
  controller.cancelAlgorithm = cancelAlgorithm;
}
function setUpTransformStreamDefaultControllerFromTransformer(stream, transformer, dict) {
  const controller = newTransformController();
  let transformAlgorithm = (chunk) => {
    try {
      transformStreamDefaultControllerEnqueue(controller, chunk);
    } catch (e) {
      return rejectedPromise(e);
    }
    return resolvedPromise(undefined);
  };
  if (dict.transform !== undefined) transformAlgorithm = (chunk) => promiseCall(dict.transform, transformer, [chunk, controller.self]);
  const flushAlgorithm = dict.flush === undefined ? () => resolvedPromise(undefined) : () => promiseCall(dict.flush, transformer, [controller.self]);
  const cancelAlgorithm = dict.cancel === undefined ? () => resolvedPromise(undefined) : (reason) => promiseCall(dict.cancel, transformer, [reason]);
  setUpTransformStreamDefaultController(stream, controller, transformAlgorithm, flushAlgorithm, cancelAlgorithm);
}
function transformStreamDefaultControllerClearAlgorithms(c) {
  c.transformAlgorithm = undefined;
  c.flushAlgorithm = undefined;
  c.cancelAlgorithm = undefined;
}
function transformStreamDefaultControllerEnqueue(c, chunk) {
  const stream = c.stream;
  const readableController = stream.readable.controller;
  if (!defaultControllerCanCloseOrEnqueue(readableController)) throw new TypeErrorCtor('Readable side is not in a state that permits enqueue');
  try {
    defaultControllerEnqueue(readableController, chunk);
  } catch (e) {
    transformStreamErrorWritableAndUnblockWrite(stream, e);
    throw stream.readable.storedError;
  }
  const backpressure = defaultControllerHasBackpressure(readableController);
  if (backpressure !== stream.backpressure) transformStreamSetBackpressure(stream, true);
}
function transformStreamDefaultControllerPerformTransform(c, chunk) {
  const promise = c.transformAlgorithm(chunk);
  return transformPromise(promise, undefined, (r) => {
    transformStreamError(c.stream, r);
    throw r;
  });
}
function transformStreamDefaultControllerTerminate(c) {
  const stream = c.stream;
  defaultControllerClose(stream.readable.controller);
  transformStreamErrorWritableAndUnblockWrite(stream, new TypeErrorCtor('TransformStream terminated'));
}
function transformStreamDefaultSinkWriteAlgorithm(stream, chunk) {
  const controller = stream.controller;
  if (stream.backpressure) {
    return transformPromise(stream.backpressureChangePromise.promise, () => {
      const writable = stream.writable;
      if (writable.state === 'erroring') throw writable.storedError;
      return transformStreamDefaultControllerPerformTransform(controller, chunk);
    });
  }
  return transformStreamDefaultControllerPerformTransform(controller, chunk);
}
function transformStreamDefaultSinkAbortAlgorithm(stream, reason) {
  const controller = stream.controller;
  if (controller.finishPromise !== undefined) return controller.finishPromise.promise;
  const readable = stream.readable;
  controller.finishPromise = newDeferred();
  const finishPromise = controller.finishPromise;
  markHandled(finishPromise.promise);
  const cancelPromise = controller.cancelAlgorithm(reason);
  transformStreamDefaultControllerClearAlgorithms(controller);
  uponPromise(cancelPromise, () => {
    if (readable.state === 'errored') {
      deferredReject(finishPromise, readable.storedError);
    } else {
      defaultControllerError(readable.controller, reason);
      deferredResolve(finishPromise, undefined);
    }
  }, (r) => {
    defaultControllerError(readable.controller, r);
    deferredReject(finishPromise, r);
  });
  return finishPromise.promise;
}
function transformStreamDefaultSinkCloseAlgorithm(stream) {
  const controller = stream.controller;
  if (controller.finishPromise !== undefined) return controller.finishPromise.promise;
  const readable = stream.readable;
  controller.finishPromise = newDeferred();
  const finishPromise = controller.finishPromise;
  markHandled(finishPromise.promise);
  const flushPromise = controller.flushAlgorithm();
  transformStreamDefaultControllerClearAlgorithms(controller);
  uponPromise(flushPromise, () => {
    if (readable.state === 'errored') {
      deferredReject(finishPromise, readable.storedError);
    } else {
      defaultControllerClose(readable.controller);
      deferredResolve(finishPromise, undefined);
    }
  }, (r) => {
    defaultControllerError(readable.controller, r);
    deferredReject(finishPromise, r);
  });
  return finishPromise.promise;
}
function transformStreamDefaultSourceCancelAlgorithm(stream, reason) {
  const controller = stream.controller;
  if (controller.finishPromise !== undefined) return controller.finishPromise.promise;
  const writable = stream.writable;
  controller.finishPromise = newDeferred();
  const finishPromise = controller.finishPromise;
  markHandled(finishPromise.promise);
  const cancelPromise = controller.cancelAlgorithm(reason);
  transformStreamDefaultControllerClearAlgorithms(controller);
  uponPromise(cancelPromise, () => {
    if (writable.state === 'errored') {
      deferredReject(finishPromise, writable.storedError);
    } else {
      writableStreamDefaultControllerErrorIfNeeded(writable.controller, reason);
      transformStreamUnblockWrite(stream);
      deferredResolve(finishPromise, undefined);
    }
  }, (r) => {
    writableStreamDefaultControllerErrorIfNeeded(writable.controller, r);
    transformStreamUnblockWrite(stream);
    deferredReject(finishPromise, r);
  });
  return finishPromise.promise;
}
function transformStreamDefaultSourcePullAlgorithm(stream) {
  transformStreamSetBackpressure(stream, false);
  return stream.backpressureChangePromise.promise;
}

function convertTransformer(transformer) {
  const obj = toDictionary(transformer, 'transformer');
  const dict = { cancel: undefined, flush: undefined, readableType: undefined, start: undefined, transform: undefined, writableType: undefined };
  if (obj === undefined) return dict;
  let v = obj.cancel;
  if (v !== undefined) dict.cancel = toCallback(v, 'transformer.cancel');
  v = obj.flush;
  if (v !== undefined) dict.flush = toCallback(v, 'transformer.flush');
  dict.readableType = obj.readableType;
  v = obj.start;
  if (v !== undefined) dict.start = toCallback(v, 'transformer.start');
  v = obj.transform;
  if (v !== undefined) dict.transform = toCallback(v, 'transformer.transform');
  dict.writableType = obj.writableType;
  return dict;
}

class TransformStream {
  constructor(transformer = undefined, writableStrategy = undefined, readableStrategy = undefined) {
    const dict = convertTransformer(transformer);
    const writableStrategyDict = convertStrategy(writableStrategy);
    const readableStrategyDict = convertStrategy(readableStrategy);
    if (dict.readableType !== undefined) throw new RangeErrorCtor('Invalid readableType specified');
    if (dict.writableType !== undefined) throw new RangeErrorCtor('Invalid writableType specified');
    const readableHWM = extractHighWaterMark(readableStrategyDict, 0);
    const readableSizeAlgorithm = extractSizeAlgorithm(readableStrategyDict);
    const writableHWM = extractHighWaterMark(writableStrategyDict, 1);
    const writableSizeAlgorithm = extractSizeAlgorithm(writableStrategyDict);
    const startPromise = newDeferred();
    const stream = createTransformStreamState();
    stream.self = this;
    setSlot(tsBrand, this, stream);
    initializeTransformStream(stream, startPromise, writableHWM, writableSizeAlgorithm, readableHWM, readableSizeAlgorithm);
    const source = transformer === null ? undefined : transformer;
    setUpTransformStreamDefaultControllerFromTransformer(stream, source, dict);
    if (dict.start !== undefined) {
      deferredResolve(startPromise, ReflectApply(dict.start, source, [stream.controller.self]));
    } else {
      deferredResolve(startPromise, undefined);
    }
  }
  get readable() {
    return slotOf(tsBrand, this, 'TransformStream').readable.self;
  }
  get writable() {
    return slotOf(tsBrand, this, 'TransformStream').writable.self;
  }
}

class TransformStreamDefaultController {
  constructor() {
    throw new TypeErrorCtor('Illegal constructor');
  }
  get desiredSize() {
    const c = slotOf(tdcBrand, this, 'TransformStreamDefaultController');
    return defaultControllerGetDesiredSize(c.stream.readable.controller);
  }
  enqueue(chunk = undefined) {
    transformStreamDefaultControllerEnqueue(slotOf(tdcBrand, this, 'TransformStreamDefaultController'), chunk);
  }
  error(reason = undefined) {
    transformStreamError(slotOf(tdcBrand, this, 'TransformStreamDefaultController').stream, reason);
  }
  terminate() {
    transformStreamDefaultControllerTerminate(slotOf(tdcBrand, this, 'TransformStreamDefaultController'));
  }
}

// ---- text encoding streams --------------------------------------------------------
const TextEncoderCtor = globalObject.TextEncoder;
const TextDecoderCtor = globalObject.TextDecoder;
const tesBrand = makeBrand('tesBrand');
const tdsBrand = makeBrand('tdsBrand');
const tsReadableGet = getterOf(TransformStream.prototype, 'readable');
const tsWritableGet = getterOf(TransformStream.prototype, 'writable');

class TextEncoderStream {
  constructor() {
    const encoder = new TextEncoderCtor();
    let pendingHighSurrogate = null;
    const transform = new TransformStream({
      transform(chunk, controller) {
        let input = '' + chunk;
        if (pendingHighSurrogate !== null) {
          input = pendingHighSurrogate + input;
          pendingHighSurrogate = null;
        }
        if (input.length > 0) {
          const last = input.charCodeAt(input.length - 1);
          if (last >= 0xd800 && last <= 0xdbff) {
            pendingHighSurrogate = input.slice(input.length - 1);
            input = input.slice(0, input.length - 1);
          }
        }
        if (input.length > 0) controller.enqueue(encoder.encode(input));
      },
      flush(controller) {
        if (pendingHighSurrogate !== null) controller.enqueue(encoder.encode('�'));
      },
    });
    setSlot(tesBrand, this, transform);
  }
  get encoding() {
    slotOf(tesBrand, this, 'TextEncoderStream');
    return 'utf-8';
  }
  get readable() {
    return ReflectApply(tsReadableGet, slotOf(tesBrand, this, 'TextEncoderStream'), []);
  }
  get writable() {
    return ReflectApply(tsWritableGet, slotOf(tesBrand, this, 'TextEncoderStream'), []);
  }
}

class TextDecoderStream {
  constructor(label = 'utf-8', options = {}) {
    const decoder = new TextDecoderCtor(label, options);
    const transform = new TransformStream({
      transform(chunk, controller) {
        const text = decoder.decode(chunk, { stream: true });
        if (text.length > 0) controller.enqueue(text);
      },
      flush(controller) {
        const text = decoder.decode();
        if (text.length > 0) controller.enqueue(text);
      },
    });
    setSlot(tdsBrand, this, { transform, decoder });
  }
  get encoding() {
    return slotOf(tdsBrand, this, 'TextDecoderStream').decoder.encoding;
  }
  get fatal() {
    return slotOf(tdsBrand, this, 'TextDecoderStream').decoder.fatal;
  }
  get ignoreBOM() {
    return slotOf(tdsBrand, this, 'TextDecoderStream').decoder.ignoreBOM;
  }
  get readable() {
    return ReflectApply(tsReadableGet, slotOf(tdsBrand, this, 'TextDecoderStream').transform, []);
  }
  get writable() {
    return ReflectApply(tsWritableGet, slotOf(tdsBrand, this, 'TextDecoderStream').transform, []);
  }
}

// ---- CompressionStream / DecompressionStream ----------------------------------------
const compressionFormats = ['gzip', 'deflate', 'deflate-raw', 'brotli'];
const csBrand = makeBrand('csBrand');
const dsBrand = makeBrand('dsBrand');
function convertFormat(format) {
  const value = '' + format;
  if (compressionFormats.indexOf(value) < 0) throw new TypeErrorCtor("Unsupported compression format: '" + value + "'");
  return value;
}
// BufferSource that is not backed by a SharedArrayBuffer.
function chunkBytes(chunk) {
  let buffer;
  let offset = 0;
  let length;
  if (ReflectApply(ArrayBufferIsView, ArrayBufferCtor, [chunk])) {
    buffer = viewBuffer(chunk);
    offset = viewByteOffset(chunk);
    length = viewByteLength(chunk);
  } else {
    try {
      length = bufferByteLength(chunk);
    } catch (e) {
      throw new TypeErrorCtor('Chunk must be an ArrayBuffer or ArrayBufferView');
    }
    buffer = chunk;
  }
  let view;
  try {
    view = new Uint8ArrayCtor(buffer, offset, length);
  } catch (e) {
    throw new TypeErrorCtor('Chunk must be a non-shared BufferSource');
  }
  const copy = new Uint8ArrayCtor(length);
  copy.set(view);
  return copy;
}
class CompressionStream {
  constructor(format) {
    if (arguments.length < 1) throw new TypeErrorCtor('CompressionStream requires a format');
    const fmt = convertFormat(format);
    const chunks = [];
    let total = 0;
    const transform = new TransformStream({
      transform(chunk) {
        const bytes = chunkBytes(chunk);
        ReflectApply(ArrayPush, chunks, [bytes]);
        total += bytes.byteLength;
      },
      flush(controller) {
        const out = native.compress(fmt, concatChunks(chunks, total));
        if (out.byteLength > 0) controller.enqueue(out);
      },
    });
    setSlot(csBrand, this, transform);
  }
  get readable() {
    return ReflectApply(tsReadableGet, slotOf(csBrand, this, 'CompressionStream'), []);
  }
  get writable() {
    return ReflectApply(tsWritableGet, slotOf(csBrand, this, 'CompressionStream'), []);
  }
}
class DecompressionStream {
  constructor(format) {
    if (arguments.length < 1) throw new TypeErrorCtor('DecompressionStream requires a format');
    const fmt = convertFormat(format);
    const chunks = [];
    let total = 0;
    let finished = false;
    const transform = new TransformStream({
      transform(chunk, controller) {
        const bytes = chunkBytes(chunk);
        if (finished) {
          if (bytes.byteLength > 0) throw new TypeErrorCtor('Unexpected data after the end of the compressed stream');
          return;
        }
        ReflectApply(ArrayPush, chunks, [bytes]);
        total += bytes.byteLength;
        if (total === 0) return;
        const result = native.decompress(fmt, concatChunks(chunks, total));
        if (result !== null) {
          finished = true;
          if (result[0].byteLength > 0) controller.enqueue(result[0]);
          if (result[1] < total) throw new TypeErrorCtor('Unexpected data after the end of the compressed stream');
        }
      },
      flush() {
        if (!finished) throw new TypeErrorCtor('Unexpected end of the compressed stream');
      },
    });
    setSlot(dsBrand, this, transform);
  }
  get readable() {
    return ReflectApply(tsReadableGet, slotOf(dsBrand, this, 'DecompressionStream'), []);
  }
  get writable() {
    return ReflectApply(tsWritableGet, slotOf(dsBrand, this, 'DecompressionStream'), []);
  }
}
