use crate::parser::{Class, Member, Node, SetExpr};
use crate::{
    advance_index, canonical, line, point, space, word, Error, Flags, Match, MAX_CODE, MAX_STACK,
    MAX_STEPS,
};
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub(crate) struct Program {
    code: Vec<Op>,
    marks: usize,
}
#[derive(Debug, Clone)]
enum Op {
    /// The pattern character, and the flags in effect at this instruction's
    /// position (only `ignore_case` differs from the outer search flags,
    /// via a `(?ims-ims:...)` modifier group).
    Char(u32, Flags),
    Any(Flags),
    Class(Rc<FoldedClass>, Flags),
    Set(Rc<CompiledSet>, Flags),
    Start(Flags),
    End(Flags),
    Boundary(bool, Flags),
    Split(usize, usize),
    Jump(usize),
    Save(usize),
    Clear(usize, usize),
    Mark(usize),
    Progress(usize),
    Look(Box<Program>, bool),
    /// A lookbehind assertion, matched by re-running its nested program from
    /// every candidate start position at or before the current position,
    /// requiring it to end exactly there (see `run`'s `required_end`).
    LookBehind(Box<Program>, bool),
    Backref(Vec<usize>, Flags),
    Accept,
}
#[derive(Debug, Clone)]
struct FoldedClass {
    class: Class,
    folded: Vec<u32>,
}
#[derive(Debug, Clone)]
struct CompiledSet {
    expression: SetExpr,
    strings: HashMap<u32, Vec<Vec<u32>>>,
    empty: bool,
}
impl CompiledSet {
    fn new(expression: &SetExpr, flags: Flags) -> Self {
        fn collect(expression: &SetExpr, out: &mut Vec<Vec<u32>>) {
            match expression {
                SetExpr::String(s) => out.push(s.clone()),
                SetExpr::Property(name, _) => {
                    out.extend(crate::unicode_sets::strings(name).iter().cloned())
                }
                SetExpr::Union(terms) | SetExpr::Intersect(terms) => {
                    for term in terms {
                        collect(term, out);
                    }
                }
                SetExpr::Subtract(left, right) => {
                    collect(left, out);
                    collect(right, out);
                }
                SetExpr::Complement(term) => collect(term, out),
                SetExpr::Char(_) => {}
            }
        }
        let mut candidates = Vec::new();
        collect(expression, &mut candidates);
        candidates.sort();
        candidates.dedup();
        let empty = candidates.iter().any(|s| s.is_empty()) && expression.contains(&[], flags);
        let mut strings: HashMap<u32, Vec<Vec<u32>>> = HashMap::new();
        for sequence in candidates {
            if sequence.len() > 1 && expression.contains(&sequence, flags) {
                strings
                    .entry(canonical(sequence[0], flags))
                    .or_default()
                    .push(sequence);
            }
        }
        for sequences in strings.values_mut() {
            sequences.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        }
        Self {
            expression: expression.clone(),
            strings,
            empty,
        }
    }
    fn ends(&self, input: &[u16], pos: usize, flags: Flags) -> Vec<usize> {
        let mut ends = Vec::new();
        if let Some((first, end)) = point(input, pos, true) {
            if let Some(strings) = self.strings.get(&canonical(first, flags)) {
                for sequence in strings {
                    let mut index = pos;
                    let mut matched = true;
                    for expected in sequence {
                        if let Some((actual, next)) = point(input, index, true) {
                            if canonical(actual, flags) == canonical(*expected, flags) {
                                index = next;
                            } else {
                                matched = false;
                                break;
                            }
                        } else {
                            matched = false;
                            break;
                        }
                    }
                    if matched {
                        ends.push(index);
                    }
                }
            }
            if self.expression.contains(&[first], flags) {
                ends.push(end);
            }
        }
        if self.empty {
            ends.push(pos);
        }
        ends
    }
}
impl SetExpr {
    fn contains(&self, sequence: &[u32], flags: Flags) -> bool {
        match self {
            Self::Char(member) => match sequence {
                [c] => match member {
                    Member::Range(lo, hi) => {
                        // `Node::Set`/`SetExpr` only exists under the `v`
                        // flag (`class()` only builds it when
                        // `unicode_sets` is set), so `canonical` is always
                        // in its unicode-mode branch here: a fixed point, or
                        // the lower side of some `PAIRS` entry (or entries --
                        // e.g. both `K` and Kelvin-sign `K` fold to `k`).
                        // `folded`'s only other possible class members are
                        // those entries' upper sides, i.e. its `partners` --
                        // two binary searches instead of an up-to-1512-entry
                        // scan of the whole table (see `partners`' doc).
                        let folded = canonical(*c, flags);
                        (*lo..=*hi).contains(&folded)
                            || (flags.ignore_case
                                && crate::unicode_casefold::partners(folded)
                                    .any(|p| (*lo..=*hi).contains(&p)))
                    }
                    Member::Digit(invert) => (48..=57).contains(c) != *invert,
                    Member::Word(invert) => word(*c, flags) != *invert,
                    Member::Space(invert) => space(*c) != *invert,
                },
                _ => false,
            },
            Self::String(s) => {
                s.len() == sequence.len()
                    && s.iter()
                        .zip(sequence)
                        .all(|(a, b)| canonical(*a, flags) == canonical(*b, flags))
            }
            Self::Property(name, invert) => {
                if crate::unicode_sets::supported(name) {
                    let direct = crate::unicode_sets::contains(name, sequence);
                    if *invert {
                        sequence.len() == 1 && !direct
                    } else {
                        direct
                    }
                } else {
                    match sequence {
                        // `invert` is folded in *before* the ignoreCase
                        // case-fold check runs, not applied to its result:
                        // under `i`, `\P{Lu}` must match `A` (its lowercase
                        // fold partner is outside Lu) even though `A` itself
                        // is directly in Lu. A post-hoc `!` on a
                        // direct-membership-only test can't express that.
                        [c] => {
                            let test = |x: u32| {
                                let hit = crate::unicode_props::contains(name, x);
                                if *invert {
                                    !hit
                                } else {
                                    hit
                                }
                            };
                            // Same two-binary-search swap as `Member::Range`
                            // above: this is exactly `test` of each of `c`'s
                            // `PAIRS` partners (there can be more than one --
                            // both `K` and Kelvin-sign `K` fold to `k`), not
                            // a fresh per-character scan of the whole table.
                            test(*c)
                                || (flags.ignore_case
                                    && crate::unicode_casefold::partners(*c).any(test))
                        }
                        _ => false,
                    }
                }
            }
            Self::Union(terms) => terms.iter().any(|term| term.contains(sequence, flags)),
            Self::Intersect(terms) => {
                !terms.is_empty() && terms.iter().all(|term| term.contains(sequence, flags))
            }
            Self::Subtract(left, right) => {
                left.contains(sequence, flags) && !right.contains(sequence, flags)
            }
            Self::Complement(term) => sequence.len() == 1 && !term.contains(sequence, flags),
        }
    }
}
impl FoldedClass {
    fn new(class: &Class, flags: Flags, remaining: &mut usize) -> Result<Self, Error> {
        fn charge(remaining: &mut usize, cost: usize) -> Result<(), Error> {
            *remaining = remaining
                .checked_sub(cost)
                .ok_or(Error::Limit("class compilation work"))?;
            Ok(())
        }
        charge(remaining, class.members.len())?;
        let mut folded = Vec::new();
        if flags.ignore_case {
            // Index the same endpoints used by the old full-table scan. This
            // fixed Unicode-data index is shared; pattern-dependent work and
            // allocations below remain charged to the compilation budget.
            static ENDPOINTS: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();
            let endpoints = ENDPOINTS.get_or_init(|| {
                let mut points: Vec<_> = crate::unicode_casefold::PAIRS
                    .iter()
                    .flat_map(|&(a, b)| [a, b])
                    .collect();
                points.sort_unstable();
                points.dedup();
                points
            });
            for member in &class.members {
                if let Member::Range(lo, hi) = member {
                    // Bound both binary searches as well as the endpoints we
                    // actually visit, rather than charging an entire Unicode
                    // table for every small ASCII range.
                    charge(remaining, 2 * (endpoints.len().ilog2() as usize + 1))?;
                    let start = endpoints.partition_point(|c| c < lo);
                    let end = endpoints.partition_point(|c| c <= hi);
                    charge(remaining, end - start)?;
                    folded.extend(endpoints[start..end].iter().map(|&c| canonical(c, flags)));
                }
            }
            folded.sort_unstable();
            folded.dedup();
        }
        Ok(Self {
            class: class.clone(),
            folded,
        })
    }
    fn contains(&self, c: u32, flags: Flags) -> bool {
        let canon = canonical(c, flags);
        let yes = self.class.members.iter().any(|m| match m {
            Member::Range(lo, hi) => (*lo..=*hi).contains(&canon),
            Member::Digit(invert) => (48..=57).contains(&c) != *invert,
            Member::Word(invert) => word(c, flags) != *invert,
            Member::Space(invert) => space(c) != *invert,
        }) || self.folded.binary_search(&canon).is_ok();
        yes != self.class.inverted
    }
}
struct Compiler {
    code: Vec<Op>,
    marks: usize,
    flags: Flags,
    remaining: usize,
    classes: HashMap<usize, Rc<FoldedClass>>,
}
pub(crate) fn compile(ast: &Node, _captures: usize, flags: Flags) -> Result<Program, Error> {
    let mut c = Compiler {
        code: Vec::new(),
        marks: 0,
        flags,
        remaining: MAX_CODE,
        classes: HashMap::new(),
    };
    c.emit(Op::Save(0))?;
    c.node(ast)?;
    c.emit(Op::Save(1))?;
    c.emit(Op::Accept)?;
    Ok(Program {
        code: c.code,
        marks: c.marks,
    })
}
impl Compiler {
    fn emit(&mut self, op: Op) -> Result<usize, Error> {
        if self.remaining == 0 {
            return Err(Error::Limit("compiled instructions"));
        }
        self.remaining -= 1;
        let n = self.code.len();
        self.code.push(op);
        Ok(n)
    }
    fn clear(&mut self, n: &Node) -> Result<(), Error> {
        fn range(
            n: &Node,
            out: &mut Option<Range<usize>>,
            remaining: &mut usize,
        ) -> Result<(), Error> {
            *remaining = remaining
                .checked_sub(1)
                .ok_or(Error::Limit("compilation work"))?;
            match n {
                Node::Group(i, n) => {
                    let new = i * 2..i * 2 + 2;
                    *out = Some(match out.take() {
                        Some(r) => r.start.min(new.start)..r.end.max(new.end),
                        None => new,
                    });
                    range(n, out, remaining)?;
                }
                Node::Seq(ns) | Node::Alt(ns) => {
                    for n in ns {
                        range(n, out, remaining)?;
                    }
                }
                Node::Repeat(n, ..) | Node::Look(n, _) | Node::LookBehind(n, _) => {
                    range(n, out, remaining)?
                }
                Node::Mode(_, _, n) => range(n, out, remaining)?,
                _ => {}
            }
            Ok(())
        }
        let mut captures = None;
        range(n, &mut captures, &mut self.remaining)?;
        if let Some(r) = captures {
            self.emit(Op::Clear(r.start, r.end))?;
        }
        Ok(())
    }
    fn node(&mut self, n: &Node) -> Result<(), Error> {
        self.remaining = self
            .remaining
            .checked_sub(1)
            .ok_or(Error::Limit("compilation work"))?;
        match n {
            Node::Empty => {}
            Node::Char(c) => {
                self.emit(Op::Char(*c, self.flags))?;
            }
            Node::Any => {
                self.emit(Op::Any(self.flags))?;
            }
            Node::Class(c) => {
                // Safe pointer identity is used only as a compilation-local cache
                // key; the AST remains borrowed and is never dereferenced by key.
                let key = c as *const Class as usize;
                let prepared = if let Some(cached) = self.classes.get(&key) {
                    cached.clone()
                } else {
                    let prepared = Rc::new(FoldedClass::new(c, self.flags, &mut self.remaining)?);
                    self.classes.insert(key, prepared.clone());
                    prepared
                };
                self.emit(Op::Class(prepared, self.flags))?;
            }
            Node::Set(expression) => {
                self.emit(Op::Set(
                    Rc::new(CompiledSet::new(expression, self.flags)),
                    self.flags,
                ))?;
            }
            Node::Start => {
                self.emit(Op::Start(self.flags))?;
            }
            Node::End => {
                self.emit(Op::End(self.flags))?;
            }
            Node::Boundary(b) => {
                self.emit(Op::Boundary(*b, self.flags))?;
            }
            Node::Backref(ids) => {
                self.emit(Op::Backref(ids.clone(), self.flags))?;
            }
            Node::Group(i, n) => {
                self.emit(Op::Save(i * 2))?;
                self.node(n)?;
                self.emit(Op::Save(i * 2 + 1))?;
            }
            Node::Seq(ns) => {
                for n in ns {
                    self.node(n)?;
                }
            }
            Node::Alt(ns) => {
                let mut ends = Vec::new();
                for (i, n) in ns.iter().enumerate() {
                    if i + 1 == ns.len() {
                        self.node(n)?;
                        break;
                    }
                    let split = self.emit(Op::Split(0, 0))?;
                    let body = self.code.len();
                    self.node(n)?;
                    ends.push(self.emit(Op::Jump(0))?);
                    self.code[split] = Op::Split(body, self.code.len());
                }
                let end = self.code.len();
                for j in ends {
                    self.code[j] = Op::Jump(end);
                }
            }
            Node::Repeat(n, min, max, greedy) => {
                if *min > MAX_CODE || max.is_some_and(|n| n > MAX_CODE) {
                    return Err(Error::Limit("quantifier expansion"));
                }
                for _ in 0..*min {
                    self.clear(n)?;
                    self.node(n)?;
                }
                let copies = max.map(|m| m - min).unwrap_or(1);
                let mut exits = Vec::new();
                for _ in 0..copies {
                    let split = self.emit(Op::Split(0, 0))?;
                    let body = self.code.len();
                    let mark = self.marks;
                    self.marks += 1;
                    self.emit(Op::Mark(mark))?;
                    self.clear(n)?;
                    self.node(n)?;
                    self.emit(Op::Progress(mark))?;
                    if max.is_none() {
                        self.emit(Op::Jump(split))?;
                    }
                    exits.push((split, body));
                }
                let end = self.code.len();
                for (split, body) in exits {
                    self.code[split] = if *greedy {
                        Op::Split(body, end)
                    } else {
                        Op::Split(end, body)
                    };
                }
            }
            Node::Look(n, positive) => {
                let mut nested = Compiler {
                    code: Vec::new(),
                    marks: 0,
                    flags: self.flags,
                    remaining: self.remaining,
                    classes: self.classes.clone(),
                };
                nested.node(n)?;
                nested.emit(Op::Accept)?;
                self.remaining = nested.remaining;
                self.classes = nested.classes;
                self.emit(Op::Look(
                    Box::new(Program {
                        code: nested.code,
                        marks: nested.marks,
                    }),
                    *positive,
                ))?;
            }
            Node::LookBehind(n, positive) => {
                let mut nested = Compiler {
                    code: Vec::new(),
                    marks: 0,
                    flags: self.flags,
                    remaining: self.remaining,
                    classes: self.classes.clone(),
                };
                nested.node(n)?;
                nested.emit(Op::Accept)?;
                self.remaining = nested.remaining;
                self.classes = nested.classes;
                self.emit(Op::LookBehind(
                    Box::new(Program {
                        code: nested.code,
                        marks: nested.marks,
                    }),
                    *positive,
                ))?;
            }
            Node::Mode(add, remove, n) => {
                let saved = self.flags;
                let mut effective = self.flags;
                if add[0] {
                    effective.ignore_case = true;
                }
                if remove[0] {
                    effective.ignore_case = false;
                }
                if add[1] {
                    effective.multiline = true;
                }
                if remove[1] {
                    effective.multiline = false;
                }
                if add[2] {
                    effective.dot_all = true;
                }
                if remove[2] {
                    effective.dot_all = false;
                }
                self.flags = effective;
                self.node(n)?;
                self.flags = saved;
            }
        }
        Ok(())
    }
}
#[derive(Clone)]
struct State {
    pc: usize,
    pos: usize,
    captures: Vec<Option<usize>>,
    marks: Vec<Option<usize>>,
}
pub(crate) struct Budget {
    steps: usize,
    saved: usize,
    slots: usize,
}
impl Budget {
    pub(crate) fn new() -> Self {
        Self {
            steps: MAX_STEPS,
            saved: 0,
            slots: 0,
        }
    }
    fn step(&mut self) -> Result<(), Error> {
        self.steps = self
            .steps
            .checked_sub(1)
            .ok_or(Error::Limit("matching steps"))?;
        Ok(())
    }
}
pub(crate) fn search(
    program: &Program,
    input: &[u16],
    mut start: usize,
    flags: Flags,
    captures: usize,
    budget: &mut Budget,
) -> Result<Option<Match>, Error> {
    if start > input.len() {
        return Ok(None);
    }
    // Unicode RegExpBuiltinExec interprets a lastIndex inside a surrogate pair
    // at that code point's start (GetStringIndex/StringToCodePoints).
    if flags.unicode_mode()
        && start > 0
        && input
            .get(start)
            .is_some_and(|u| (0xDC00..=0xDFFF).contains(u))
        && (0xD800..=0xDBFF).contains(&input[start - 1])
    {
        start -= 1;
    }
    loop {
        budget.step()?;
        if let Some(result) = run(
            program,
            input,
            start,
            vec![None; (captures + 1) * 2],
            flags,
            budget,
            None,
        )? {
            let groups = result
                .captures
                .chunks_exact(2)
                .map(|p| match (p[0], p[1]) {
                    (Some(a), Some(b)) => Some(a..b),
                    _ => None,
                })
                .collect::<Vec<_>>();
            return Ok(Some(Match {
                range: start..result.pos,
                captures: groups,
            }));
        }
        if flags.sticky || start == input.len() {
            return Ok(None);
        }
        start = advance_index(input, start, flags.unicode_mode());
    }
}
fn run(
    program: &Program,
    input: &[u16],
    start: usize,
    captures: Vec<Option<usize>>,
    flags: Flags,
    budget: &mut Budget,
    required_end: Option<usize>,
) -> Result<Option<State>, Error> {
    let mut state = State {
        pc: 0,
        pos: start,
        captures,
        marks: vec![None; program.marks],
    };
    let mut stack: Vec<State> = Vec::new();
    loop {
        budget.step()?;
        let mut success = true;
        match &program.code[state.pc] {
            Op::Accept => match required_end {
                Some(end) if end != state.pos => success = false,
                _ => {
                    budget.saved -= stack.len();
                    budget.slots -= stack.len() * (state.captures.len() + state.marks.len() + 3);
                    return Ok(Some(state));
                }
            },
            Op::Char(c, cflags) => {
                if let Some((next, end)) = point(input, state.pos, flags.unicode_mode()) {
                    if canonical(*c, *cflags) == canonical(next, *cflags) {
                        state.pos = end;
                    } else {
                        success = false;
                    }
                } else {
                    success = false;
                }
            }
            Op::Any(aflags) => {
                if let Some((c, end)) = point(input, state.pos, flags.unicode_mode()) {
                    if aflags.dot_all || !line(c) {
                        state.pos = end;
                    } else {
                        success = false;
                    }
                } else {
                    success = false;
                }
            }
            Op::Class(c, cflags) => {
                if let Some((next, end)) = point(input, state.pos, flags.unicode_mode()) {
                    if c.contains(next, *cflags) {
                        state.pos = end;
                    } else {
                        success = false;
                    }
                } else {
                    success = false;
                }
            }
            Op::Set(set, sflags) => {
                let ends = set.ends(input, state.pos, *sflags);
                if let Some((&first, alternatives)) = ends.split_first() {
                    for &end in alternatives.iter().rev() {
                        if budget.saved >= MAX_STACK
                            || budget.slots + state.captures.len() + state.marks.len() + 3
                                > 1_048_576
                        {
                            return Err(Error::Limit("backtracking storage"));
                        }
                        let mut alternative = state.clone();
                        alternative.pos = end;
                        alternative.pc += 1;
                        stack.push(alternative);
                        budget.saved += 1;
                        budget.slots += state.captures.len() + state.marks.len() + 3;
                    }
                    state.pos = first;
                } else {
                    success = false;
                }
            }
            Op::Start(sflags) => {
                success =
                    state.pos == 0 || (sflags.multiline && line(input[state.pos - 1] as u32))
            }
            Op::End(eflags) => {
                success = state.pos == input.len()
                    || (eflags.multiline && input.get(state.pos).is_some_and(|u| line(*u as u32)))
            }
            Op::Boundary(positive, bflags) => {
                let left = if state.pos == 0 {
                    false
                } else {
                    let previous = if flags.unicode_mode()
                        && state.pos >= 2
                        && (0xDC00..=0xDFFF).contains(&input[state.pos - 1])
                        && (0xD800..=0xDBFF).contains(&input[state.pos - 2])
                    {
                        state.pos - 2
                    } else {
                        state.pos - 1
                    };
                    point(input, previous, flags.unicode_mode())
                        .is_some_and(|(c, _)| word(c, *bflags))
                };
                let right = point(input, state.pos, flags.unicode_mode())
                    .is_some_and(|(c, _)| word(c, *bflags));
                success = (left != right) == *positive;
            }
            Op::Split(a, b) => {
                // Count storage slots as well as frames: large capture/mark
                // vectors cannot turn a bounded frame count into gigabytes.
                if budget.saved >= MAX_STACK
                    || budget.slots + state.captures.len() + state.marks.len() + 3 > 1_048_576
                {
                    return Err(Error::Limit("backtracking storage"));
                }
                let mut alternative = state.clone();
                alternative.pc = *b;
                stack.push(alternative);
                budget.saved += 1;
                budget.slots += state.captures.len() + state.marks.len() + 3;
                state.pc = *a;
                continue;
            }
            Op::Jump(target) => {
                state.pc = *target;
                continue;
            }
            Op::Save(slot) => state.captures[*slot] = Some(state.pos),
            Op::Clear(start, end) => state.captures[*start..*end].fill(None),
            Op::Mark(id) => state.marks[*id] = Some(state.pos),
            Op::Progress(id) => success = state.marks[*id] != Some(state.pos),
            Op::Look(n, positive) => {
                let result =
                    run(n, input, state.pos, state.captures.clone(), flags, budget, None)?;
                success = result.is_some() == *positive;
                if *positive {
                    if let Some(result) = result {
                        state.captures = result.captures;
                    }
                }
            }
            Op::LookBehind(n, positive) => {
                let target = state.pos;
                let mut found = None;
                // Ascending order: the leftmost candidate start is the one a
                // real backward/reversed matcher would pick first, since it
                // corresponds to a greedy quantifier inside the lookbehind
                // consuming as much as possible (e.g. `(?<=(\d+)-)` on
                // "42-x" should capture "42", not "2").
                for candidate in 0..=target {
                    budget.step()?;
                    if let Some(result) = run(
                        n,
                        input,
                        candidate,
                        state.captures.clone(),
                        flags,
                        budget,
                        Some(target),
                    )? {
                        found = Some(result.captures);
                        break;
                    }
                }
                success = found.is_some() == *positive;
                if *positive {
                    if let Some(captures) = found {
                        state.captures = captures;
                    }
                }
            }
            Op::Backref(ids, rflags) => {
                // At most one id can be bound at once when several names
                // collapse onto the same backref (mutually-exclusive
                // alternatives); an unbound name is a no-op match.
                let bound = ids.iter().find_map(|id| {
                    match (state.captures[id * 2], state.captures[id * 2 + 1]) {
                        (Some(a), Some(b)) => Some((a, b)),
                        _ => None,
                    }
                });
                if let Some((mut a, b)) = bound {
                    let mut pos = state.pos;
                    while a < b {
                        budget.step()?;
                        let left = point(input, a, flags.unicode_mode());
                        let right = point(input, pos, flags.unicode_mode());
                        match (left, right) {
                            (Some((c, next)), Some((d, end)))
                                if canonical(c, *rflags) == canonical(d, *rflags) =>
                            {
                                a = next;
                                pos = end;
                            }
                            _ => {
                                success = false;
                                break;
                            }
                        }
                    }
                    if success {
                        state.pos = pos;
                    }
                }
            }
        }
        if success {
            state.pc += 1;
        } else if let Some(next) = stack.pop() {
            budget.saved -= 1;
            budget.slots -= next.captures.len() + next.marks.len() + 3;
            state = next;
        } else {
            return Ok(None);
        }
    }
}
