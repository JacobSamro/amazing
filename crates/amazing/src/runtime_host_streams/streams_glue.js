// Global installation and the Body-mixin glue used by Request/Response.

function installClass(klass, name, withTag) {
  const proto = klass.prototype;
  for (const key of ObjectGetOwnPropertyNames(proto)) {
    if (key === 'constructor') continue;
    const desc = ObjectGetOwnPropertyDescriptor(proto, key);
    desc.enumerable = true;
    ObjectDefineProperty(proto, key, desc);
  }
  for (const key of ObjectGetOwnPropertyNames(klass)) {
    if (key === 'length' || key === 'name' || key === 'prototype') continue;
    const desc = ObjectGetOwnPropertyDescriptor(klass, key);
    desc.enumerable = true;
    ObjectDefineProperty(klass, key, desc);
  }
  if (withTag !== false) {
    ObjectDefineProperty(proto, SymbolToStringTag, { value: name, configurable: true });
  }
}
{
  const exposed = [
    ['ReadableStream', ReadableStream], ['ReadableStreamDefaultReader', ReadableStreamDefaultReader],
    ['ReadableStreamBYOBReader', ReadableStreamBYOBReader], ['ReadableStreamBYOBRequest', ReadableStreamBYOBRequest],
    ['ReadableByteStreamController', ReadableByteStreamController],
    ['ReadableStreamDefaultController', ReadableStreamDefaultController],
    ['TransformStream', TransformStream], ['TransformStreamDefaultController', TransformStreamDefaultController],
    ['WritableStream', WritableStream], ['WritableStreamDefaultWriter', WritableStreamDefaultWriter],
    ['WritableStreamDefaultController', WritableStreamDefaultController],
    ['ByteLengthQueuingStrategy', ByteLengthQueuingStrategy], ['CountQueuingStrategy', CountQueuingStrategy],
    ['TextEncoderStream', TextEncoderStream], ['TextDecoderStream', TextDecoderStream],
    ['CompressionStream', CompressionStream], ['DecompressionStream', DecompressionStream],
  ];
  const proto = ReadableStream.prototype;
  ObjectDefineProperty(proto, SymbolAsyncIterator, { value: proto.values, writable: true, enumerable: false, configurable: true });
  for (const [name, klass] of exposed) {
    installClass(klass, name);
    ObjectDefineProperty(globalObject, name, { value: klass, writable: true, enumerable: false, configurable: true });
  }
  // `values` must be re-checked after enumerability fixes; the async iterator
  // symbol and `values` share one function object.
  ObjectDefineProperty(proto, SymbolAsyncIterator, { value: proto.values, writable: true, enumerable: false, configurable: true });
}

// ---- Body mixin glue ----------------------------------------------------------------
const bodyStreams = new WeakMapCtor();   // Request/Response -> { stream (state), user }
const ResponseCtor = globalObject.Response;
const BlobCtor = globalObject.Blob;
function bodyEntry(object) {
  return ReflectApply(WeakMapGet, bodyStreams, [object]);
}
function attachBodyStream(object, streamObject, user) {
  const stream = readableStreamState(streamObject);
  ReflectApply(WeakMapSet, bodyStreams, [object, { stream, user }]);
}
function createNativeBodyStream(object, alreadyUsed) {
  const stream = new ReadableStream({
    type: 'bytes',
    pull(controller) {
      const bytes = native.take(object);
      if (bytes.byteLength > 0) controller.enqueue(bytes);
      controller.close();
      const request = controller.byobRequest;
      if (request !== null) request.respond(0);
    },
  });
  const state = readableStreamState(stream);
  if (alreadyUsed) {
    acquireReadableStreamDefaultReader(state);
    readableStreamCancel(state, undefined);
  }
  return state;
}
function concatChunks(chunks, total) {
  const out = new Uint8ArrayCtor(total);
  let offset = 0;
  for (let i = 0; i < chunks.length; ++i) {
    out.set(chunks[i], offset);
    offset += chunks[i].byteLength;
  }
  return out;
}
function readAllBytes(state) {
  const reader = acquireReadableStreamDefaultReader(state);
  const deferred = newDeferred();
  const chunks = [];
  let total = 0;
  function step() {
    readableStreamDefaultReaderRead(reader, {
      chunk(chunk) {
        if (!ReflectApply(ArrayBufferIsView, ArrayBufferCtor, [chunk]) || viewTag(chunk) !== 'Uint8Array') {
          const e = new TypeErrorCtor('Response body read chunk is not a Uint8Array');
          readableStreamCancel(state, e);
          deferredReject(deferred, e);
          return;
        }
        const copy = new Uint8ArrayCtor(chunk.byteLength);
        copy.set(chunk);
        ReflectApply(ArrayPush, chunks, [copy]);
        total += copy.byteLength;
        step();
      },
      close() { deferredResolve(deferred, concatChunks(chunks, total)); },
      error(e) { deferredReject(deferred, e); },
    });
  }
  step();
  return deferred.promise;
}
const hooks = {
  isStream(v) {
    return hasSlot(rsBrand, v);
  },
  constructed(object, kind, input, init) {
    if (kind === 'response') {
      if (hasSlot(rsBrand, input)) {
        const state = readableStreamState(input);
        if (isReadableStreamLocked(state) || state.disturbed) throw new TypeErrorCtor('Response body stream is locked or disturbed');
        attachBodyStream(object, input, true);
      }
      return;
    }
    const initBody = isObject(init) ? init.body : undefined;
    if (hasSlot(rsBrand, initBody)) {
      if (init.duplex !== 'half') throw new TypeErrorCtor('RequestInit: duplex option is required when sending a body');
      const state = readableStreamState(initBody);
      if (isReadableStreamLocked(state) || state.disturbed) throw new TypeErrorCtor('Request body stream is locked or disturbed');
      attachBodyStream(object, initBody, true);
    } else if ((initBody === undefined || initBody === null) && isObject(input)) {
      const entry = bodyEntry(input);
      if (entry !== undefined && (isReadableStreamLocked(entry.stream) || entry.stream.disturbed)) {
        throw new TypeErrorCtor('Request body stream is locked or disturbed');
      }
      if (entry !== undefined && entry.user) {
        const proxy = new TransformStream();
        markHandled(readableStreamPipeTo(entry.stream, slotOf(wsBrand, proxy.writable), false, false, false, undefined));
        attachBodyStream(object, proxy.readable, true);
      }
    }
  },
  body(object, hasBody, rustUsed) {
    if (!hasBody) return null;
    let entry = bodyEntry(object);
    if (entry === undefined) {
      entry = { stream: createNativeBodyStream(object, rustUsed), user: false };
      ReflectApply(WeakMapSet, bodyStreams, [object, entry]);
    }
    return entry.stream.self;
  },
  disturbed(object) {
    const entry = bodyEntry(object);
    return entry !== undefined && entry.stream.disturbed;
  },
  consume(object, method, mime) {
    const entry = bodyEntry(object);
    if (entry === undefined) return undefined;
    const state = entry.stream;
    if (isReadableStreamLocked(state) || state.disturbed) {
      return rejectedPromise(new TypeErrorCtor('Body has already been consumed or is locked'));
    }
    if (!entry.user) {
      acquireReadableStreamDefaultReader(state);
      readableStreamCancel(state, undefined);
      return undefined;
    }
    return transformPromise(readAllBytes(state), (bytes) => {
      const init = mime ? { headers: { 'content-type': mime } } : {};
      const response = new ResponseCtor(bytes, init);
      return response[method]();
    });
  },
  upload(input, init) {
    const RequestCtor = globalObject.Request;
    let stream;
    let fromInit = false;
    const initBody = isObject(init) ? init.body : undefined;
    if (hasSlot(rsBrand, initBody)) {
      if (init.duplex !== 'half') return rejectedPromise(new TypeErrorCtor('RequestInit: duplex option is required when sending a body'));
      stream = readableStreamState(initBody);
      fromInit = true;
    } else if ((initBody === undefined || initBody === null) && isObject(input) && typeof RequestCtor === 'function') {
      const entry = bodyEntry(input);
      if (entry === undefined || !entry.user) return undefined;
      stream = entry.stream;
    } else {
      return undefined;
    }
    if (isReadableStreamLocked(stream) || stream.disturbed) {
      return rejectedPromise(new TypeErrorCtor('Request body stream is locked or disturbed'));
    }
    return transformPromise(readAllBytes(stream), (bytes) => {
      let request;
      if (fromInit) {
        const copy = {};
        for (const key of ObjectGetOwnPropertyNames(init)) copy[key] = init[key];
        copy.body = bytes;
        delete copy.duplex;
        request = new RequestCtor(input, copy);
      } else {
        request = new RequestCtor(input, { body: bytes });
      }
      return native.fetchRaw(request);
    });
  },
  cloneCheck(object) {
    const entry = bodyEntry(object);
    if (entry !== undefined && (isReadableStreamLocked(entry.stream) || entry.stream.disturbed)) {
      throw new TypeErrorCtor('Body has already been consumed or is locked');
    }
  },
  cloned(original, clone) {
    const entry = bodyEntry(original);
    if (entry === undefined || !entry.user) return;
    const branches = readableStreamTee(entry.stream, false);
    entry.stream = branches[0];
    ReflectApply(WeakMapSet, bodyStreams, [clone, { stream: branches[1], user: true }]);
  },
};
if (typeof BlobCtor === 'function' && typeof BlobCtor.prototype.stream !== 'function') {
  const blobArrayBuffer = BlobCtor.prototype.arrayBuffer;
  const blobStream = {
    stream() {
      const buffered = ReflectApply(blobArrayBuffer, this, []);
      return new ReadableStream({
        type: 'bytes',
        pull(controller) {
          return transformPromise(buffered, (buffer) => {
            if (buffer.byteLength > 0) controller.enqueue(new Uint8ArrayCtor(buffer));
            controller.close();
            const request = controller.byobRequest;
            if (request !== null) request.respond(0);
          });
        },
      });
    },
  }.stream;
  ObjectDefineProperty(BlobCtor.prototype, 'stream', { value: blobStream, writable: true, enumerable: true, configurable: true });
}
hooks.brands = brandRegistry;
return hooks;
