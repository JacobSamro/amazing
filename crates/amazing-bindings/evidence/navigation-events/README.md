# DOM-B navigation event payloads ready

Implementation `9d13ddc7`, fixed base `d50cd1bf`. HashChangeEvent,
PopStateEvent, PageTransitionEvent and BeforeUnloadEvent now have native payloads
and interface brands. Constructors convert inherited dictionaries first, keep
state-object identity and convert URL fields to USVString. BeforeUnloadEvent's
string returnValue remains independent of Event's boolean cancellation flag;
legacy factory aliases select the correct interfaces.

Linux workspace **2767 passed /10 skipped**, doctests **4/4**. Fresh complete
**482-source dom/nodes + dom/events** corpus: **8442→8448 passing assertions**,
**1019→1013 failing**, **six gains /zero lost passes**. Files unchanged:199 pass,
131 fail,99 timeouts,27 unsupported,26 harness errors;6 subtest errors unchanged.
Document-createEvent improves **249→255 passes**.

Navigation scheduling and checking unload cancellation remain embedding work.
Compared with the older events corpus, the fresh baseline registers43 additional
body/frameset handler failures after HTMLFrameSetElement became available;
no passes were lost, and inline handler compilation remains deferred. Continuing
lossless UTF-16 event types across constructors, initializers and listener keys.

Spec: https://html.spec.whatwg.org/multipage/nav-history-apis.html#event-interfaces .
Reproduce with dom-b-audit/run-corpus.py and `--directory dom/nodes --directory dom/events`.
