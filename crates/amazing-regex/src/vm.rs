use crate::parser::{Class, Member, Node};
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
    Char(u32),
    Any,
    Class(Rc<FoldedClass>),
    Start,
    End,
    Boundary(bool),
    Split(usize, usize),
    Jump(usize),
    Save(usize),
    Clear(usize, usize),
    Mark(usize),
    Progress(usize),
    Look(Box<Program>, bool),
    Backref(usize),
    Accept,
}
#[derive(Debug, Clone)]
struct FoldedClass {
    class: Class,
    folded: Vec<u32>,
}
impl FoldedClass {
    fn new(class: &Class, flags: Flags) -> Self {
        let mut folded = Vec::new();
        if flags.ignore_case {
            for &(a, b) in crate::unicode_casefold::PAIRS {
                for c in [a, b] {
                    if class
                        .members
                        .iter()
                        .any(|m| matches!(m,Member::Range(lo,hi) if (*lo..=*hi).contains(&c)))
                    {
                        folded.push(canonical(c, flags));
                    }
                }
            }
            folded.sort_unstable();
            folded.dedup();
        }
        Self {
            class: class.clone(),
            folded,
        }
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
                Node::Repeat(n, ..) | Node::Look(n, _) => range(n, out, remaining)?,
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
                self.emit(Op::Char(*c))?;
            }
            Node::Any => {
                self.emit(Op::Any)?;
            }
            Node::Class(c) => {
                // Safe pointer identity is used only as a compilation-local cache
                // key; the AST remains borrowed and is never dereferenced by key.
                let key = c as *const Class as usize;
                let prepared = if let Some(cached) = self.classes.get(&key) {
                    cached.clone()
                } else {
                    let cost = c.members.len()
                        + if self.flags.ignore_case {
                            crate::unicode_casefold::PAIRS.len() * 2
                        } else {
                            0
                        };
                    self.remaining = self
                        .remaining
                        .checked_sub(cost)
                        .ok_or(Error::Limit("class compilation work"))?;
                    let prepared = Rc::new(FoldedClass::new(c, self.flags));
                    self.classes.insert(key, prepared.clone());
                    prepared
                };
                self.emit(Op::Class(prepared))?;
            }
            Node::Start => {
                self.emit(Op::Start)?;
            }
            Node::End => {
                self.emit(Op::End)?;
            }
            Node::Boundary(b) => {
                self.emit(Op::Boundary(*b))?;
            }
            Node::Backref(i) => {
                self.emit(Op::Backref(*i))?;
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
    if flags.unicode
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
        start = advance_index(input, start, flags.unicode);
    }
}
fn run(
    program: &Program,
    input: &[u16],
    start: usize,
    captures: Vec<Option<usize>>,
    flags: Flags,
    budget: &mut Budget,
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
            Op::Accept => {
                budget.saved -= stack.len();
                budget.slots -= stack.len() * (state.captures.len() + state.marks.len() + 3);
                return Ok(Some(state));
            }
            Op::Char(c) => {
                if let Some((next, end)) = point(input, state.pos, flags.unicode) {
                    if canonical(*c, flags) == canonical(next, flags) {
                        state.pos = end;
                    } else {
                        success = false;
                    }
                } else {
                    success = false;
                }
            }
            Op::Any => {
                if let Some((c, end)) = point(input, state.pos, flags.unicode) {
                    if flags.dot_all || !line(c) {
                        state.pos = end;
                    } else {
                        success = false;
                    }
                } else {
                    success = false;
                }
            }
            Op::Class(c) => {
                if let Some((next, end)) = point(input, state.pos, flags.unicode) {
                    if c.contains(next, flags) {
                        state.pos = end;
                    } else {
                        success = false;
                    }
                } else {
                    success = false;
                }
            }
            Op::Start => {
                success = state.pos == 0 || (flags.multiline && line(input[state.pos - 1] as u32))
            }
            Op::End => {
                success = state.pos == input.len()
                    || (flags.multiline && input.get(state.pos).is_some_and(|u| line(*u as u32)))
            }
            Op::Boundary(positive) => {
                let left = if state.pos == 0 {
                    false
                } else {
                    let previous = if flags.unicode
                        && state.pos >= 2
                        && (0xDC00..=0xDFFF).contains(&input[state.pos - 1])
                        && (0xD800..=0xDBFF).contains(&input[state.pos - 2])
                    {
                        state.pos - 2
                    } else {
                        state.pos - 1
                    };
                    point(input, previous, flags.unicode).is_some_and(|(c, _)| word(c, flags))
                };
                let right =
                    point(input, state.pos, flags.unicode).is_some_and(|(c, _)| word(c, flags));
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
                let result = run(n, input, state.pos, state.captures.clone(), flags, budget)?;
                success = result.is_some() == *positive;
                if *positive {
                    if let Some(result) = result {
                        state.captures = result.captures;
                    }
                }
            }
            Op::Backref(id) => {
                if let (Some(mut a), Some(b)) = (state.captures[id * 2], state.captures[id * 2 + 1])
                {
                    let mut pos = state.pos;
                    while a < b {
                        budget.step()?;
                        let left = point(input, a, flags.unicode);
                        let right = point(input, pos, flags.unicode);
                        match (left, right) {
                            (Some((c, next)), Some((d, end)))
                                if canonical(c, flags) == canonical(d, flags) =>
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
