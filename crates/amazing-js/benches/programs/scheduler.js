// A small cooperative task scheduler (richards-like): tasks with priorities
// pass packets through queues; the scheduler picks the highest-priority
// runnable task each step. Written from scratch for this suite.
function Packet(link, kind, datum) { this.link = link; this.kind = kind; this.datum = datum; }
function Queue() { this.head = null; this.tail = null; this.length = 0; }
Queue.prototype.push = function (p) {
  p.link = null;
  if (this.tail) this.tail.link = p; else this.head = p;
  this.tail = p; this.length++;
};
Queue.prototype.shift = function () {
  var p = this.head;
  if (p) { this.head = p.link; if (!this.head) this.tail = null; this.length--; }
  return p;
};
function Task(id, priority, work) {
  this.id = id; this.priority = priority; this.work = work;
  this.inbox = new Queue(); this.state = 0; this.processed = 0; this.acc = 0;
}
Task.prototype.runnable = function () { return this.inbox.length > 0; };
Task.prototype.step = function (sched) {
  var p = this.inbox.shift();
  this.processed++;
  this.acc = (this.acc + this.work(p)) % 1000003;
  return p;
};
function Scheduler() { this.tasks = []; this.steps = 0; }
Scheduler.prototype.add = function (t) {
  this.tasks.push(t);
  this.tasks.sort(function (a, b) { return b.priority - a.priority; });
};
Scheduler.prototype.send = function (id, p) {
  for (var i = 0; i < this.tasks.length; i++) if (this.tasks[i].id === id) { this.tasks[i].inbox.push(p); return; }
};
Scheduler.prototype.run = function (limit) {
  while (this.steps < limit) {
    var chosen = null;
    for (var i = 0; i < this.tasks.length; i++) if (this.tasks[i].runnable()) { chosen = this.tasks[i]; break; }
    if (!chosen) break;
    var p = chosen.step(this);
    this.steps++;
    if (p.datum > 0) {
      var next = (chosen.id + 1) % 4;
      p.datum--; p.kind = (p.kind + 1) % 3;
      this.send(next, p);
    }
  }
};
var s = new Scheduler();
for (var t = 0; t < 4; t++) {
  s.add(new Task(t, 4 - t, function (p) { return p.datum * 3 + p.kind; }));
}
for (var n = 0; n < 16; n++) s.send(n % 4, new Packet(null, n % 3, 400 + n % 17));
s.run(200000);
var out = [];
for (var q = 0; q < s.tasks.length; q++) out.push(s.tasks[q].id + ":" + s.tasks[q].processed + ":" + s.tasks[q].acc);
print("scheduler " + s.steps + " " + out.join(","));
