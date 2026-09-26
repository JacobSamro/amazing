// SCAFFOLDING ONLY. Not upstream testharness.js and not a conformance oracle.
// This intentionally tiny producer exercises the runner's callback transport.
var scaffold_result;
var scaffold_complete;
function setup(options) {}
function add_result_callback(callback) { scaffold_result = callback; }
function add_completion_callback(callback) { scaffold_complete = callback; }
function assert_equals(actual, expected) {
    if (actual !== expected) { throw new Error('scaffold assertion failed'); }
}
function test(callback, name) {
    var status = 0;
    var message = null;
    try { callback(); } catch (e) { status = 1; message = e.message; }
    scaffold_result({name: name, status: status, message: message});
}
function async_test(name) {
    return {
        done: function() { scaffold_result({name: name, status: 0, message: null}); },
        timeout: function() { scaffold_result({name: name, status: 2, message: 'scaffold timeout'}); }
    };
}
function done() { scaffold_complete([], {status: 0, message: null}); }
